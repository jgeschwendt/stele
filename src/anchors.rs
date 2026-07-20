//! Comment-anchor compilation (SPEC §2.4/§2.5).
//!
//! Two jobs, both tree-sitter-fronted. The scanner walks VCS-tracked files for the
//! `stele:landmark` <slug> and `stele:claim` <addr> tokens: in a file whose language
//! has a bundled parser, only tokens inside COMMENT nodes count (a token inside a
//! string literal is ignored); in markdown the native comment is the HTML comment, so
//! only tokens inside `<!-- -->` count and fenced code / prose are quotations (§2.5);
//! only a parser-less NON-markdown file falls to a lexical line scan. Symbol resolution
//! binds a `<path>#<symbol>` anchor to a named definition via the same parsers. The
//! language registry maps extensions to grammars; ABI compatibility of core 0.26
//! against grammars at ABI 14/15 is verified empirically (the parse tests below
//! exercise every bundled grammar).
//!
//! Backtick discipline in THIS file's own comments: a token is always written with a
//! closing backtick glued immediately after it (no space between the token and the next
//! character), so the scanner never reads the engine's own doc comment as a declaration
//! — these comments are scanned like any other source (a dogfood constraint; §2.5).

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

/// Scan every tracked file for comment anchors (§2.4 scan scope: `.stele/` excluded,
/// AGENTS.md files included; `.steleignore`d paths never reach here — they are already
/// gone from `tracked`). A parser-backed file counts tokens only inside comments; a
/// markdown file counts tokens only inside HTML comments (§2.5); only a parser-less
/// non-markdown file falls to a lexical line scan. A malformed slug in any anchor is a
/// §5.3 input error (exit 2) naming the offending `file:line`.
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
        match extension(rel).as_deref() {
            Some("markdown" | "md") => scan_markdown(&contents, &file, &mut data)?,
            other => match other.and_then(lang_for_extension) {
                Some(lang) => scan_parsed(&mut parser, lang, &file, &contents, &mut data)?,
                None => scan_text(&contents, 0, &file, &mut data)?,
            },
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

/// The HTML comment delimiters — in markdown the ONLY construct whose anchor tokens are
/// declarations (§2.5 "language-native comments"). Tokens in a fenced code block or in
/// prose (inline backticks included) are quotations and are skipped.
const HTML_COMMENT_OPEN: &str = "<!--";
const HTML_COMMENT_CLOSE: &str = "-->";

/// Scan a markdown file for comment anchors (§2.5): only tokens inside `<!-- -->` HTML
/// comments count. Fenced code blocks are skipped entirely (a `<!--` inside a fence is
/// literal code, not a comment), and prose outside a comment is never scanned — so the
/// EXAMPLE 8.4 table cell that quotes the landmark token is a quotation, not a
/// declaration. The comment state carries across lines; malformed slugs inside a comment
/// still fail (§2.5), so `scan_text` does the per-fragment recording and validation.
fn scan_markdown(contents: &str, file: &str, data: &mut AnchorData) -> Result<()> {
    let mut fence: Option<(char, usize)> = None;
    let mut in_comment = false;
    for (row, line) in contents.lines().enumerate() {
        if let Some((fence_char, open_len)) = fence {
            // Inside a fenced block: skip every line; the matching closer ends it. A
            // fence is never recognized while inside a comment, so this arm only runs
            // when `in_comment` is false.
            if crate::parse::is_close_fence(line, fence_char, open_len) {
                fence = None;
            }
            continue;
        }
        // A fence opener is only recognized outside a comment (inside a comment the
        // ``` is comment text, not code); the closer is handled above.
        if !in_comment && let Some((fence_char, open_len, _)) = crate::parse::open_fence(line) {
            fence = Some((fence_char, open_len));
            continue;
        }
        in_comment = scan_markdown_line(line, row, in_comment, file, data)?;
    }
    Ok(())
}

/// Scan the portions of one markdown `line` that lie inside an HTML comment, returning
/// whether the line ends still inside a comment (carried to the next line). `in_comment`
/// is the state on entry. Each in-comment fragment is handed to `scan_text` at `row`
/// (0-based) so a token reports its true 1-based file line.
fn scan_markdown_line(
    line: &str,
    row: usize,
    mut in_comment: bool,
    file: &str,
    data: &mut AnchorData,
) -> Result<bool> {
    let mut rest = line;
    loop {
        if in_comment {
            match rest.find(HTML_COMMENT_CLOSE) {
                Some(close) => {
                    scan_text(&rest[..close], row, file, data)?;
                    rest = &rest[close + HTML_COMMENT_CLOSE.len()..];
                    in_comment = false;
                }
                None => {
                    // Rest of the line is inside the comment; scan it and carry state on.
                    scan_text(rest, row, file, data)?;
                    return Ok(true);
                }
            }
        } else {
            match rest.find(HTML_COMMENT_OPEN) {
                Some(open) => {
                    rest = &rest[open + HTML_COMMENT_OPEN.len()..];
                    in_comment = true;
                }
                None => return Ok(false), // rest is prose — skip it.
            }
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
    let ext = extension(path);
    // Markdown has no tree-sitter named-definition grammar (§2.4), so a `<path>#<symbol>`
    // anchor into a `.md`/`.markdown` file resolves against GitHub-style heading slugs —
    // headings are the definition analogue. This repo's own AGENTS.md depends on it
    // (`research/claims.md#c1`, `SPEC.md#decision-log`). digest stays null (parser-less →
    // §4.5 churn fallback), same as any other parser-less anchored file.
    if matches!(ext.as_deref(), Some("markdown" | "md")) {
        return resolve_markdown_heading(root, path, rel_path, symbol);
    }
    let Some(lang) = ext.as_deref().and_then(lang_for_extension) else {
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

// ─── markdown heading-slug resolution (§2.4) ──────────────────────────────────

/// The ATX heading marker: 1–6 `#` opening a heading line (§2.4 markdown analogue).
const HEADING_MARK: char = '#';
const MAX_HEADING_LEVEL: usize = 6;

/// Resolve a `<path>#<symbol>` anchor into a markdown file against GitHub-style heading
/// slugs (§2.4): the symbol matches a heading whose slug EQUALS it, or begins with it
/// immediately followed by a `-` (so `c1` binds `## C1: …` but not `## C10: …`).
/// Cardinality discipline is identical to tree-sitter resolution — exactly 1 → resolved,
/// 0 → unresolved, ≥2 → ambiguous. A missing file is unresolved (the anchor cannot bind).
fn resolve_markdown_heading(
    root: &Path,
    path: &Path,
    rel_path: &str,
    symbol: &str,
) -> Result<SymbolResolution> {
    let contents = match std::fs::read_to_string(root.join(path)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SymbolResolution::Unresolved);
        }
        Err(e) => return Err(SteleError::internal(format!("read {rel_path}: {e}"))),
    };
    let mut lines = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if let Some(text) = atx_heading_text(line) {
            let slug = heading_slug(text);
            if slug == symbol
                || slug
                    .strip_prefix(symbol)
                    .is_some_and(|rest| rest.starts_with('-'))
            {
                lines.push(index + 1);
            }
        }
    }
    Ok(match lines.as_slice() {
        [] => SymbolResolution::Unresolved,
        [line] => SymbolResolution::Resolved(*line),
        _ => SymbolResolution::Ambiguous,
    })
}

/// The heading text of an ATX heading line (`## Title`), or `None` when `line` is not a
/// heading. The opening `#` run (1–6) must be followed by whitespace or end-of-line; a
/// trailing space-preceded `#` run (the optional closing sequence) is stripped.
fn atx_heading_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == HEADING_MARK).count();
    if hashes == 0 || hashes > MAX_HEADING_LEVEL {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let text = rest.trim();
    Some(text.trim_end_matches(HEADING_MARK).trim_end())
}

/// The GitHub-style slug of heading `text` (§2.4): lowercased, punctuation stripped, and
/// each run of whitespace or `-` collapsed to a single `-`, with leading/trailing `-`
/// trimmed. Alphanumerics (Unicode included) survive; every other character is dropped.
fn heading_slug(text: &str) -> String {
    let mut slug = String::new();
    let mut pending_hyphen = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if pending_hyphen && !slug.is_empty() {
                slug.push('-');
            }
            pending_hyphen = false;
            slug.extend(c.to_lowercase());
        } else if c.is_whitespace() || c == '-' {
            pending_hyphen = true;
        }
        // Every other character is punctuation and is stripped.
    }
    slug
}

// ─── AST-region structural digest (§4.5) ──────────────────────────────────────

/// A bound region's §4.5 structural digest plus its human name (the region
/// descriptor the freshness finding prints, EXAMPLE 8.4 `changeset/2`).
#[derive(Clone, Debug)]
pub struct RegionDigest {
    pub digest: String,
    pub name: String,
}

/// The §4.5 structural digest (only) of a resolved claim's bound definition, or
/// `None` for a parser-less anchored file (§2.4). The stable `build`-time entry
/// point (`stamp_verified`); a thin wrapper over [`region_digest_for_claim`] so
/// the stamped bytes are byte-identical to what freshness later recomputes.
pub fn digest_for_claim(root: &Path, anchor: &str, resolved: &str) -> Result<Option<String>> {
    Ok(region_digest_for_claim(root, anchor, resolved)?.map(|region| region.digest))
}

/// The §4.5 digest AND region name of a resolved claim's BOUND DEFINITION from the
/// CURRENT working tree, or `None` when the anchored file's language has no bundled
/// parser (§2.4 — those claims fall to the freshness churn-count path). `resolved`
/// is the claim's `file:line` (the comment/symbol line, §4.5); the digested region
/// is the bound definition, which may sit on a different line.
///
/// Binding (§4.5, EXAMPLE 8.4):
/// - `<path>#<symbol>` → the resolved symbol's definition node.
/// - `lm:<slug>` (a landmark/`stele:claim` comment) → the named definition the
///   comment IMMEDIATELY PRECEDES in source order within its enclosing scope,
///   skipping intervening comment/attribute/doc siblings. If it precedes none, the
///   bound region falls back to the strictly-enclosing named definition, then the
///   whole file.
pub fn region_digest_for_claim(
    root: &Path,
    anchor: &str,
    resolved: &str,
) -> Result<Option<RegionDigest>> {
    let (file, line) = split_resolved(resolved);
    let path = Path::new(file);
    let Some(lang) = extension(path).as_deref().and_then(lang_for_extension) else {
        return Ok(None);
    };
    let contents = std::fs::read_to_string(root.join(path))
        .map_err(|e| SteleError::internal(format!("read {file}: {e}")))?;
    let mut parser = Parser::new();
    let tree = parse(&mut parser, lang, &contents, file)?;
    // A landmark line came from `resolved`, so binding never fails here.
    Ok(region_digest_bound(
        tree.root_node(),
        contents.as_bytes(),
        lang,
        anchor,
        Some(line),
        file,
    ))
}

/// The §4.5 digest AND region name of a claim's bound definition computed against a
/// caller-supplied file `contents` (a historical `git show <sha>:<file>` blob, for
/// the `stele blame`/staling-commit walk, §5.1). `None` for a parser-less file, an
/// unparseable blob, or an `lm:` anchor whose landmark token is absent from this
/// version (the region did not yet exist → the digest is treated as divergent).
pub fn region_digest_of_source(anchor: &str, file: &str, contents: &str) -> Option<RegionDigest> {
    let lang = extension(Path::new(file))
        .as_deref()
        .and_then(lang_for_extension)?;
    let mut parser = Parser::new();
    parser.set_language(&language(lang)).ok()?;
    let tree = parser.parse(contents, None)?;
    region_digest_bound(
        tree.root_node(),
        contents.as_bytes(),
        lang,
        anchor,
        None,
        file,
    )
}

/// Bind `anchor` to its digested region and return its digest + name (§4.5). For an
/// `lm:` anchor, `lm_line` (1-based) locates the landmark comment when known (the
/// working-tree path); when `None` the token is scanned for in `src` (the historical
/// path), yielding `None` if it is absent. A `<path>#<symbol>` anchor binds to the
/// symbol's definition, falling back to the whole file.
fn region_digest_bound(
    root_node: Node,
    src: &[u8],
    lang: Lang,
    anchor: &str,
    lm_line: Option<usize>,
    file: &str,
) -> Option<RegionDigest> {
    let bound = if let Some(slug) = anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX) {
        let line = match lm_line {
            Some(line) => line,
            None => landmark_line(src, slug)?,
        };
        bind_landmark(root_node, src, lang, line)
    } else {
        let symbol = anchor.rsplit('#').next().unwrap_or(anchor);
        find_definition_node(root_node, src, lang, symbol).unwrap_or(root_node)
    };
    let mut serialized = String::new();
    serialize_structure(bound, src, &mut serialized);
    Some(RegionDigest {
        digest: sha256_hex(&serialized),
        name: region_name(bound, src, lang, file),
    })
}

/// The 1-based line of the first `stele:landmark` <slug> token in `src`, or `None`
/// (§4.5 historical binding). A whole-file scan — the historical blob is not
/// comment-parsed — which is why the working-tree path prefers the resolved line.
fn landmark_line(src: &[u8], slug: &str) -> Option<usize> {
    let text = std::str::from_utf8(src).ok()?;
    text.lines()
        .position(|line| token_values(line, LANDMARK_TOKEN).iter().any(|v| v == slug))
        .map(|index| index + 1)
}

/// The human name of a bound region (§4.5, EXAMPLE 8.4 `changeset/2`): an Elixir
/// `def`/`defmodule` renders `name/arity` or the module alias; every other language's
/// definition renders its `name` field; a whole-file fallback renders the base name.
fn region_name(node: Node, src: &[u8], lang: Lang, file: &str) -> String {
    match lang {
        Lang::Elixir if is_definition_node(lang, node, src) => {
            if let Some(name) = elixir_def_name(node, src) {
                return name;
            }
        }
        _ if is_definition_kind(lang, node.kind()) => {
            if let Some(name) = node
                .child_by_field_name(NAME_FIELD)
                .and_then(|n| n.utf8_text(src).ok())
            {
                return name.to_string();
            }
        }
        _ => {}
    }
    file.rsplit('/').next().unwrap_or(file).to_string()
}

/// The `name/arity` of an Elixir function def, or the alias of a `defmodule` (§4.5).
/// A parenthesized head (`changeset(refund, attrs)`) parses to a nested `call` whose
/// `arguments` child gives the arity; a bare head (`init`) is arity 0.
fn elixir_def_name(call: Node, src: &[u8]) -> Option<String> {
    let keyword = call.child(0)?.utf8_text(src).ok()?;
    let mut cursor = call.walk();
    let args = call
        .children(&mut cursor)
        .find(|c| c.kind() == "arguments")?;
    let head = args.named_child(0)?;
    if keyword == "defmodule" {
        return head.utf8_text(src).ok().map(str::to_string);
    }
    if head.kind() == "call" {
        let name = head.child(0)?.utf8_text(src).ok()?;
        let mut head_cursor = head.walk();
        let arity = head
            .children(&mut head_cursor)
            .find(|c| c.kind() == "arguments")
            .map_or(0, |a| a.named_child_count());
        Some(format!("{name}/{arity}"))
    } else {
        Some(format!("{}/0", head.utf8_text(src).ok()?))
    }
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

    // ─── markdown comment discipline (§2.5 0.8) ───────────────────────────────

    #[test]
    fn markdown_html_comment_token_is_scanned() {
        let data = scan_one("doc.md", "prose\n<!-- stele:landmark doc-mark -->\ntail\n").unwrap();
        let occ = &data.landmarks["doc-mark"];
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].line, 2);
    }

    #[test]
    fn markdown_fenced_code_token_is_skipped() {
        // A `stele:landmark` inside a fenced block is a quotation, not a declaration.
        let src = "text\n```\n<!-- stele:landmark fenced-fake -->\nstele:landmark bare-fake\n```\n";
        let data = scan_one("doc.md", src).unwrap();
        assert!(data.landmarks.is_empty(), "fenced token wrongly counted");
    }

    #[test]
    fn markdown_prose_token_is_skipped() {
        // Prose outside any HTML comment — including inline backticks — is a quotation.
        let src = "See `stele:landmark prose-fake` and stele:landmark bare-fake here.\n";
        let data = scan_one("doc.md", src).unwrap();
        assert!(data.landmarks.is_empty(), "prose token wrongly counted");
    }

    #[test]
    fn markdown_example_217_table_cell_is_not_a_false_positive() {
        // The EXAMPLE.md:217-style quoted string in a prose table cell must NOT declare a
        // landmark (the pre-0.8 lexical-scan false positive this rule closes).
        let src = "| 4 | `rg -n \"stele:landmark refund-cap\"` → jump to refund.ex:18 | code |\n";
        let data = scan_one("EXAMPLE.md", src).unwrap();
        assert!(
            !data.landmarks.contains_key("refund-cap"),
            "quoted table-cell token wrongly counted as a declaration"
        );
    }

    #[test]
    fn markdown_multiline_html_comment_scans_interior() {
        // A token on a continuation line of a multi-line HTML comment still counts.
        let src = "text\n<!--\nstele:landmark multi-mark\n-->\n";
        let data = scan_one("doc.md", src).unwrap();
        let occ = &data.landmarks["multi-mark"];
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].line, 3);
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

    // ─── markdown heading-slug resolution 0/1/many (§2.4) ─────────────────────

    /// A ledger like this repo's `research/claims.md`: `c1` must bind the `## C1: …`
    /// heading and NOT any of the `## C10`/`## C11` headings whose slugs merely share
    /// the `c1` prefix without the hyphen boundary.
    fn md_ledger(dir: &Path) {
        std::fs::write(
            dir.join("claims.md"),
            "# Claims ledger\n\
             ## C1: AGENTS.md is the cross-vendor standard\n\
             body\n\
             ## C10: some other claim entirely\n\
             ## C11: yet another\n\
             ## Decision log\n",
        )
        .unwrap();
    }

    #[test]
    fn markdown_symbol_resolves_on_prefix_boundary_not_c10() {
        let dir = tempfile::tempdir().unwrap();
        md_ledger(dir.path());
        // `c1` matches `## C1: …` (line 2) on the hyphen boundary, not `## C10`/`## C11`.
        assert_eq!(
            resolve_symbol(dir.path(), "claims.md", "c1").unwrap(),
            SymbolResolution::Resolved(2),
        );
        // `c10` matches only its own heading (line 4) — exact prefix, still cardinality 1.
        assert_eq!(
            resolve_symbol(dir.path(), "claims.md", "c10").unwrap(),
            SymbolResolution::Resolved(4),
        );
    }

    #[test]
    fn markdown_symbol_resolves_exact_slug() {
        let dir = tempfile::tempdir().unwrap();
        md_ledger(dir.path());
        // `decision-log` equals the slug of `## Decision log` (line 6).
        assert_eq!(
            resolve_symbol(dir.path(), "claims.md", "decision-log").unwrap(),
            SymbolResolution::Resolved(6),
        );
    }

    #[test]
    fn markdown_symbol_absent_is_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        md_ledger(dir.path());
        assert_eq!(
            resolve_symbol(dir.path(), "claims.md", "c99").unwrap(),
            SymbolResolution::Unresolved,
        );
    }

    #[test]
    fn markdown_symbol_with_two_matching_headings_is_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dup.md"), "## Setup\n## Setup steps\n").unwrap();
        // Both slugs (`setup`, `setup-steps`) match `setup` — exact and prefix-boundary.
        assert_eq!(
            resolve_symbol(dir.path(), "dup.md", "setup").unwrap(),
            SymbolResolution::Ambiguous,
        );
    }
}
