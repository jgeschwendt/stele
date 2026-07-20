//! Comment-anchor compilation (SPEC §2.4/§2.5).
//!
//! Two jobs, both tree-sitter-fronted. The scanner walks VCS-tracked files for
//! `stele:landmark <slug>` and `stele:claim <addr>` tokens: in a file whose
//! language has a bundled parser, only tokens inside COMMENT nodes count (a token
//! inside a string literal is ignored); a parser-less file (markdown, …) falls to
//! a lexical line scan. Symbol resolution binds a `<path>#<symbol>` anchor to a
//! named definition via the same parsers. The language registry maps extensions to
//! grammars; ABI compatibility of core 0.26 against grammars at ABI 14/15 is
//! verified empirically (the parse tests below exercise every bundled grammar).

use crate::model::{
    AnchorData, ClaimAnchor, LANDMARK_ANCHOR_PREFIX, Occurrence, Result, SteleError, is_valid_slug,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Parser};

/// The `.stele/` directory prefix excluded from the anchor scan (§2.4 scan scope).
const STELE_DIR_PREFIX: &str = ".stele/";
/// The two comment-anchor tokens (§2.5). Distinct literals that never collide, and
/// neither is a prefix of the other, so each is scanned independently.
const LANDMARK_TOKEN: &str = "stele:landmark";
const CLAIM_TOKEN: &str = "stele:claim";
/// The tree-sitter field naming a definition node's identifier across the bundled
/// grammars (rust/python/js/ts all expose `name`); symbol resolution matches on it.
const NAME_FIELD: &str = "name";
/// Elixir defines functions/modules with macro `call` nodes, not dedicated
/// definition kinds (§2.4 trade-off). These are the leading identifiers that make a
/// `call` a definition for landmark binding (§4.5); every other `call` (`use`,
/// `import`, `alias`, `@attr …`) is not a definition and never binds a landmark.
const ELIXIR_DEF_KEYWORDS: [&str; 5] = ["def", "defmacro", "defmacrop", "defmodule", "defp"];

// ─── language registry (§2.4) ────────────────────────────────────────────────

/// A bundled grammar. Extensions map here; a file with no mapping is parser-less
/// and scanned lexically (§2.5 fallback).
#[derive(Clone, Copy)]
enum Lang {
    Elixir,
    Javascript,
    Python,
    Rust,
    Tsx,
    Typescript,
}

/// The grammar for a file extension (lowercased, no dot), or `None` for a
/// parser-less file. `.jsx` rides the JavaScript grammar; `.tsx` needs the TSX
/// dialect, `.ts` the plain TypeScript one.
fn lang_for_extension(ext: &str) -> Option<Lang> {
    match ext {
        "cjs" | "js" | "jsx" | "mjs" => Some(Lang::Javascript),
        "ex" | "exs" => Some(Lang::Elixir),
        "py" => Some(Lang::Python),
        "rs" => Some(Lang::Rust),
        "ts" => Some(Lang::Typescript),
        "tsx" => Some(Lang::Tsx),
        _ => None,
    }
}

/// The tree-sitter `Language` for a grammar. Each grammar crate exposes a
/// `LanguageFn` that `Language::from` wraps.
fn language(lang: Lang) -> Language {
    match lang {
        Lang::Elixir => tree_sitter_elixir::LANGUAGE.into(),
        Lang::Javascript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    }
}

/// Whether `kind` is a named-definition node for `lang` — the node set symbol
/// resolution scans for a matching `name` field (§2.4 "function/module/class/etc.").
/// Elixir has none: `def`/`defmodule` are macro calls, so Elixir claims bind via
/// `lm:` landmarks (the EXAMPLE §4 trade-off), never `<path>#<symbol>`.
fn is_definition_kind(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Elixir => false,
        Lang::Javascript | Lang::Tsx | Lang::Typescript => matches!(
            kind,
            "abstract_class_declaration"
                | "class_declaration"
                | "enum_declaration"
                | "function_declaration"
                | "generator_function_declaration"
                | "interface_declaration"
                | "method_definition"
                | "type_alias_declaration"
        ),
        Lang::Python => matches!(kind, "class_definition" | "function_definition"),
        Lang::Rust => matches!(
            kind,
            "const_item"
                | "enum_item"
                | "function_item"
                | "macro_definition"
                | "mod_item"
                | "static_item"
                | "struct_item"
                | "trait_item"
                | "type_item"
                | "union_item"
        ),
    }
}

// ─── the scan (§2.4/§2.5) ─────────────────────────────────────────────────────

/// Scan every tracked file for comment anchors (§2.4 scan scope: `.stele/`
/// excluded, AGENTS.md files included). Parser-backed files count tokens only
/// inside comments; parser-less files fall to a lexical line scan. A malformed slug
/// in any anchor is a §5.3 input error (exit 2) naming the offending `file:line`.
pub fn scan(root: &Path, tracked: &[PathBuf]) -> Result<AnchorData> {
    let mut data = AnchorData::default();
    let mut parser = Parser::new();
    for rel in tracked {
        let file = posix(rel);
        if file.starts_with(STELE_DIR_PREFIX) {
            continue;
        }
        let contents = std::fs::read_to_string(root.join(rel))
            .map_err(|e| SteleError::internal(format!("read {file}: {e}")))?;
        match extension(rel).as_deref().and_then(lang_for_extension) {
            Some(lang) => scan_parsed(&mut parser, lang, &file, &contents, &mut data)?,
            None => scan_text(&contents, 0, &file, &mut data)?,
        }
    }
    Ok(data)
}

/// Scan one parser-backed file: parse it, then scan the text of every top-level
/// comment node (a token in a string literal never enters a comment node, so it is
/// ignored by construction).
fn scan_parsed(
    parser: &mut Parser,
    lang: Lang,
    file: &str,
    contents: &str,
    data: &mut AnchorData,
) -> Result<()> {
    let tree = parse(parser, lang, contents, file)?;
    let mut comments = Vec::new();
    collect_comments(tree.root_node(), contents.as_bytes(), &mut comments);
    for (start_row, text) in comments {
        scan_text(&text, start_row, file, data)?;
    }
    Ok(())
}

/// Collect each comment node's `(start row, text)`, not descending into a comment
/// (so a nested doc-comment marker never double-counts its parent's tokens).
fn collect_comments(node: Node, src: &[u8], out: &mut Vec<(usize, String)>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind().contains("comment") {
            if let Ok(text) = child.utf8_text(src) {
                out.push((child.start_position().row, text.to_string()));
            }
        } else {
            collect_comments(child, src, out);
        }
    }
}

/// Scan a text blob (a comment's text, or a whole parser-less file) line by line,
/// recording every anchor token. `base_row` is the 0-based file row of the blob's
/// first line, so a token's reported line is the real file position.
fn scan_text(text: &str, base_row: usize, file: &str, data: &mut AnchorData) -> Result<()> {
    for (offset, line) in text.lines().enumerate() {
        let line_no = base_row + offset + 1;
        for slug in token_values(line, LANDMARK_TOKEN) {
            if !is_valid_slug(&slug) {
                return Err(SteleError::input(
                    file,
                    line_no,
                    format!("malformed landmark slug {slug:?} (§2.5: {SLUG_LEXEME})"),
                ));
            }
            data.landmarks.entry(slug).or_default().push(Occurrence {
                file: file.to_string(),
                line: line_no,
            });
        }
        for addr in token_values(line, CLAIM_TOKEN) {
            let tail = addr.rsplit('/').next().unwrap_or(&addr);
            if !is_valid_slug(tail) {
                return Err(SteleError::input(
                    file,
                    line_no,
                    format!("malformed claim slug in address {addr:?} (§2.5: {SLUG_LEXEME})"),
                ));
            }
            data.claims.push(ClaimAnchor {
                addr,
                file: file.to_string(),
                line: line_no,
            });
        }
    }
    Ok(())
}

/// The §2.5 slug lexeme, quoted in error messages.
const SLUG_LEXEME: &str = "[a-z0-9]+(-[a-z0-9]+)*";

/// Every value following `token` on `line`: the token must be immediately followed
/// by whitespace (so `stele:landmarks` never matches `stele:landmark`), then the
/// value runs to the next whitespace or end-of-line (§2.5).
fn token_values(line: &str, token: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut from = 0;
    while let Some(rel) = line[from..].find(token) {
        let after = &line[from + rel + token.len()..];
        if after.chars().next().is_some_and(char::is_whitespace) {
            let value: String = after
                .trim_start()
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            if !value.is_empty() {
                values.push(value);
            }
        }
        from += rel + token.len();
    }
    values
}

// ─── symbol resolution (§2.4 `<path>#<symbol>`) ───────────────────────────────

/// The outcome of resolving a `<path>#<symbol>` anchor (§2.4): cardinality MUST be
/// 1; 0 is unresolved and >1 is ambiguous, and §4.1 reports the two differently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolResolution {
    /// More than one named definition of the symbol in the file.
    Ambiguous,
    /// Exactly one — the 1-based line of its definition.
    Resolved(usize),
    /// Zero: a missing file, a parser-less language, or no matching definition.
    Unresolved,
}

/// Resolve a `<path>#<symbol>` anchor (§2.4) to a named definition's line via
/// tree-sitter. A missing file or parser-less language yields `Unresolved` (the
/// anchor cannot bind); a genuine read failure is a §5.3 internal error.
pub fn resolve_symbol(root: &Path, rel_path: &str, symbol: &str) -> Result<SymbolResolution> {
    let path = Path::new(rel_path);
    let Some(lang) = extension(path).as_deref().and_then(lang_for_extension) else {
        return Ok(SymbolResolution::Unresolved);
    };
    let contents = match std::fs::read_to_string(root.join(path)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SymbolResolution::Unresolved);
        }
        Err(e) => return Err(SteleError::internal(format!("read {rel_path}: {e}"))),
    };
    let mut parser = Parser::new();
    let tree = parse(&mut parser, lang, &contents, rel_path)?;
    let mut lines = Vec::new();
    collect_definitions(
        tree.root_node(),
        contents.as_bytes(),
        lang,
        symbol,
        &mut lines,
    );
    Ok(match lines.as_slice() {
        [] => SymbolResolution::Unresolved,
        [line] => SymbolResolution::Resolved(*line),
        _ => SymbolResolution::Ambiguous,
    })
}

/// Collect the 1-based start line of every named definition of `symbol`: a node
/// whose kind is a definition kind for `lang` and whose `name` field text equals
/// `symbol`. Recurses through the whole tree (definitions nest).
fn collect_definitions(node: Node, src: &[u8], lang: Lang, symbol: &str, out: &mut Vec<usize>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_definition_kind(lang, child.kind())
            && child
                .child_by_field_name(NAME_FIELD)
                .and_then(|n| n.utf8_text(src).ok())
                == Some(symbol)
        {
            out.push(child.start_position().row + 1);
        }
        collect_definitions(child, src, lang, symbol, out);
    }
}

// ─── AST-region structural digest (§4.5) ──────────────────────────────────────

/// The §4.5 structural digest of a resolved claim's BOUND DEFINITION, or `None`
/// when the anchored file's language has no bundled parser (§2.4 — those claims
/// fall to the Phase D5 churn-count path). `resolved` is the claim's `file:line`
/// (the comment/symbol line, §4.5); the digested region is the bound definition,
/// which may sit on a different line.
///
/// Binding (§4.5, EXAMPLE 8.4):
/// - `<path>#<symbol>` → the resolved symbol's definition node.
/// - `lm:<slug>` (a landmark/`stele:claim` comment) → the named definition the
///   comment IMMEDIATELY PRECEDES in source order within its enclosing scope,
///   skipping intervening comment/attribute/doc siblings. If it precedes none, the
///   bound region falls back to the strictly-enclosing named definition, then the
///   whole file.
pub fn digest_for_claim(root: &Path, anchor: &str, resolved: &str) -> Result<Option<String>> {
    let (file, line) = split_resolved(resolved);
    let path = Path::new(file);
    let Some(lang) = extension(path).as_deref().and_then(lang_for_extension) else {
        return Ok(None);
    };
    let contents = std::fs::read_to_string(root.join(path))
        .map_err(|e| SteleError::internal(format!("read {file}: {e}")))?;
    let src = contents.as_bytes();
    let mut parser = Parser::new();
    let tree = parse(&mut parser, lang, &contents, file)?;
    let root_node = tree.root_node();

    let bound = if anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX).is_some() {
        bind_landmark(root_node, src, lang, line)
    } else {
        let symbol = anchor.rsplit('#').next().unwrap_or(anchor);
        find_definition_node(root_node, src, lang, symbol).unwrap_or(root_node)
    };

    let mut serialized = String::new();
    serialize_structure(bound, src, &mut serialized);
    Ok(Some(sha256_hex(&serialized)))
}

/// Split a claim's `resolved` `file:line` into its parts. The `file:line` was
/// produced by build from repo-relative paths, so the tail past the last `:` is the
/// 1-based line; a colon-free string (never produced today) degrades to line 0.
fn split_resolved(resolved: &str) -> (&str, usize) {
    match resolved.rsplit_once(':') {
        Some((file, num)) => (file, num.parse().unwrap_or(0)),
        None => (resolved, 0),
    }
}

/// Bind an `lm:` landmark comment at 1-based `line` to its digested region (§4.5):
/// the next named definition among the comment's later siblings, else the
/// strictly-enclosing named definition, else the whole file (`root`).
fn bind_landmark<'a>(root: Node<'a>, src: &[u8], lang: Lang, line: usize) -> Node<'a> {
    let Some(comment) = find_comment_at_row(root, line.saturating_sub(1)) else {
        return root;
    };
    if let Some(def) = next_definition_sibling(comment, src, lang) {
        return def;
    }
    enclosing_definition(comment, src, lang).unwrap_or(root)
}

/// The comment node beginning on 0-based `row` (the landmark occurrence's line),
/// or `None`. Comment nodes are leaves, so the search only descends non-comments.
fn find_comment_at_row(node: Node, row: usize) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind().contains("comment") {
            if child.start_position().row == row {
                return Some(child);
            }
        } else if let Some(found) = find_comment_at_row(child, row) {
            return Some(found);
        }
    }
    None
}

/// The first named definition among `comment`'s siblings that follow it in source
/// order (§4.5 immediate-precedence): intervening comment/attribute/doc siblings are
/// not definitions, so they are skipped rather than breaking the binding.
fn next_definition_sibling<'a>(comment: Node<'a>, src: &[u8], lang: Lang) -> Option<Node<'a>> {
    let parent = comment.parent()?;
    let mut cursor = parent.walk();
    let mut after = false;
    for child in parent.children(&mut cursor) {
        if after && is_definition_node(lang, child, src) {
            return Some(child);
        }
        if child.id() == comment.id() {
            after = true;
        }
    }
    None
}

/// The nearest ancestor of `node` that is a named definition (§4.5 fallback), or
/// `None` when none encloses it (the caller then digests the whole file).
fn enclosing_definition<'a>(node: Node<'a>, src: &[u8], lang: Lang) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if is_definition_node(lang, ancestor, src) {
            return Some(ancestor);
        }
        current = ancestor.parent();
    }
    None
}

/// Whether `node` is a named definition for `lang` (§4.5 binding). Elixir's
/// `def`/`defmodule`/… are macro `call` nodes discriminated by their leading
/// identifier ([`ELIXIR_DEF_KEYWORDS`]); every other language reuses the kind-based
/// [`is_definition_kind`] set that symbol resolution scans.
fn is_definition_node(lang: Lang, node: Node, src: &[u8]) -> bool {
    match lang {
        Lang::Elixir => {
            node.kind() == "call"
                && node
                    .child(0)
                    .filter(|c| c.kind() == "identifier")
                    .and_then(|c| c.utf8_text(src).ok())
                    .is_some_and(|keyword| ELIXIR_DEF_KEYWORDS.contains(&keyword))
        }
        _ => is_definition_kind(lang, node.kind()),
    }
}

/// The first named definition of `symbol` anywhere in the tree (§4.5 symbol
/// binding): a definition-kind node whose `name` field equals `symbol`. The anchor
/// resolved to exactly one such node (§2.4), so the first match is that node.
fn find_definition_node<'a>(
    node: Node<'a>,
    src: &[u8],
    lang: Lang,
    symbol: &str,
) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_definition_kind(lang, child.kind())
            && child
                .child_by_field_name(NAME_FIELD)
                .and_then(|n| n.utf8_text(src).ok())
                == Some(symbol)
        {
            return Some(child);
        }
        if let Some(found) = find_definition_node(child, src, lang, symbol) {
            return Some(found);
        }
    }
    None
}

/// Serialize a bound node's structure into `out` (§4.5): a pre-order walk emitting
/// `(<kind> …children…)` for interior nodes and `(<kind> <token-text>)` for leaves,
/// over ALL children (named and anonymous, so operators and punctuation count).
/// Comment subtrees are dropped entirely and whitespace-only leaves are skipped, so
/// the digest is stable across formatting and comment churn yet changes on any
/// token-level semantic edit (flipped constant, reordered guard, changed field
/// list). Whitespace never enters the digest; the same node always yields the same
/// bytes, so the sha256 is reproducible.
fn serialize_structure(node: Node, src: &[u8], out: &mut String) {
    if node.kind().contains("comment") {
        return;
    }
    if node.child_count() == 0 {
        let text = node.utf8_text(src).unwrap_or("");
        if text.trim().is_empty() {
            return;
        }
        out.push('(');
        out.push_str(node.kind());
        out.push(' ');
        out.push_str(text);
        out.push(')');
        return;
    }
    out.push('(');
    out.push_str(node.kind());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        serialize_structure(child, src, out);
    }
    out.push(')');
}

/// Lowercase-hex sha256 of `serialized` (§4.5 digest emission).
fn sha256_hex(serialized: &str) -> String {
    let digest = Sha256::digest(serialized.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

// ─── shared helpers ───────────────────────────────────────────────────────────

/// Parse `contents` with `lang`, mapping a grammar-load or parse failure to a §5.3
/// internal error (exit 3) — the extractor crashing, never a repo-out-of-spec case.
fn parse(parser: &mut Parser, lang: Lang, contents: &str, file: &str) -> Result<tree_sitter::Tree> {
    parser
        .set_language(&language(lang))
        .map_err(|e| SteleError::internal(format!("load grammar for {file}: {e}")))?;
    parser
        .parse(contents, None)
        .ok_or_else(|| SteleError::internal(format!("tree-sitter failed to parse {file}")))
}

/// A path's lowercased extension without the dot, or `None` when it has none.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// A repo-root-relative path as a POSIX string (forward slashes on every platform).
fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExitCode;
    use std::io::Write;

    /// Scan a single in-memory file of the given name, returning the anchor index.
    fn scan_one(name: &str, contents: &str) -> Result<AnchorData> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        scan(dir.path(), &[PathBuf::from(name)])
    }

    // ─── comment-vs-string discrimination (§2.4) ──────────────────────────────

    #[test]
    fn rust_token_in_comment_counts_but_in_string_does_not() {
        let src = "// stele:landmark real-one\n\
                   fn f() { let _ = \"stele:landmark fake-one\"; }\n";
        let data = scan_one("src/lib.rs", src).unwrap();
        assert!(
            data.landmarks.contains_key("real-one"),
            "comment token missed"
        );
        assert!(
            !data.landmarks.contains_key("fake-one"),
            "string-literal token wrongly counted"
        );
    }

    #[test]
    fn python_token_in_comment_counts_but_in_string_does_not() {
        let src = "# stele:landmark py-real\n\
                   x = \"stele:landmark py-fake\"\n";
        let data = scan_one("app.py", src).unwrap();
        assert!(data.landmarks.contains_key("py-real"));
        assert!(!data.landmarks.contains_key("py-fake"));
    }

    #[test]
    fn elixir_token_in_comment_counts_but_in_heredoc_does_not() {
        let src = "defmodule M do\n\
                   \x20 # stele:landmark ex-real\n\
                   \x20 @moduledoc \"\"\"\n\
                   \x20 stele:landmark ex-fake\n\
                   \x20 \"\"\"\n\
                   end\n";
        let data = scan_one("lib/m.ex", src).unwrap();
        assert!(data.landmarks.contains_key("ex-real"));
        assert!(!data.landmarks.contains_key("ex-fake"));
    }

    #[test]
    fn parserless_markdown_falls_to_lexical_scan() {
        // No bundled parser for .md: every line counts, comment framing irrelevant.
        let data = scan_one("NOTES.md", "text\n<!-- stele:landmark doc-mark -->\n").unwrap();
        let occ = &data.landmarks["doc-mark"];
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].line, 2);
    }

    // ─── slug lexeme rejection (§2.5) ─────────────────────────────────────────

    #[test]
    fn malformed_landmark_slug_is_exit_2_at_file_line() {
        let err = scan_one("src/lib.rs", "fn f() {}\n// stele:landmark Bad_Slug\n").unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
        assert_eq!(err.line, Some(2));
        assert_eq!(err.file.as_deref(), Some(Path::new("src/lib.rs")));
    }

    #[test]
    fn malformed_claim_tail_slug_is_exit_2() {
        let err = scan_one("src/lib.rs", "// stele:claim node/Bad_Slug\n").unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
    }

    // ─── winner determinism (§3.2) ────────────────────────────────────────────

    #[test]
    fn winner_is_the_lexicographically_smallest_file_line() {
        let mut data = AnchorData::default();
        data.landmarks.insert(
            "dup".to_string(),
            vec![
                Occurrence {
                    file: "b.rs".to_string(),
                    line: 1,
                },
                Occurrence {
                    file: "a.rs".to_string(),
                    line: 9,
                },
                Occurrence {
                    file: "a.rs".to_string(),
                    line: 4,
                },
            ],
        );
        let winner = data.winner("dup").unwrap();
        // a.rs < b.rs on file first, then line 4 < 9 within a.rs.
        assert_eq!((winner.file.as_str(), winner.line), ("a.rs", 4));
        assert!(data.winner("absent").is_none());
    }

    // ─── path#symbol resolution 0/1/many (§2.4) ───────────────────────────────

    #[test]
    fn symbol_resolution_counts_zero_one_and_many() {
        let dir = tempfile::tempdir().unwrap();
        // Two `foo` definitions (redefinition is legal syntax), one `bar`, no `baz`.
        std::fs::write(
            dir.path().join("m.py"),
            "def foo():\n    pass\ndef foo():\n    pass\ndef bar():\n    pass\n",
        )
        .unwrap();
        assert_eq!(
            resolve_symbol(dir.path(), "m.py", "bar").unwrap(),
            SymbolResolution::Resolved(5),
        );
        assert_eq!(
            resolve_symbol(dir.path(), "m.py", "foo").unwrap(),
            SymbolResolution::Ambiguous,
        );
        assert_eq!(
            resolve_symbol(dir.path(), "m.py", "baz").unwrap(),
            SymbolResolution::Unresolved,
        );
    }

    #[test]
    fn symbol_resolution_on_missing_file_is_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_symbol(dir.path(), "gone.rs", "whatever").unwrap(),
            SymbolResolution::Unresolved,
        );
    }
}
