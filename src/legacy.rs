//! The pre-0.3.0 notation and the `stele migrate` rewrite (SPEC §5.1).
//!
//! One grammar ships (§2.5); this module is the ONLY place the retired one is spelled,
//! so nothing else in the engine can accidentally speak it. [`rewrite`] is pure — it
//! takes a file's bytes and a [`SourceKind`] and returns the migrated bytes, or `None`
//! when the file already reads as glyph notation (the idempotence §5.3 requires).
//!
//! Scoping mirrors the scanner that compiled the old grammar, so a migration never
//! changes what a token MEANS:
//!
//! - **markdown** ([`SourceKind::Markdown`]/[`SourceKind::Node`]) — comment tokens are
//!   rewritten only inside HTML comments (§2.5: in markdown the native comment is
//!   `<!-- -->`; a token in a fence or in prose is a quotation and stays one).
//! - **node files** ([`SourceKind::Node`], an `AGENTS.md`) — additionally the authored
//!   block's `anchor:`/`decided_by:` fields (inside the FIRST ```` ```stele ```` fence,
//!   rewritten textually so comments and key order survive byte-for-byte) and the §3.1
//!   region markers outside every fence.
//! - **everything else** ([`SourceKind::Code`]) — a lexical per-line rewrite. The old
//!   scanner counted tokens only inside comment NODES, so a token in a string literal
//!   was never a declaration; rewriting it anyway is harmless (a glyph in a string
//!   literal is not a declaration either) and cannot MISS a comment the way a
//!   re-parse of a file with syntax errors could.
//!
//! Backtick discipline in this file's own comments, and the reason it now matters twice:
//! a retired token is always written with a backtick glued immediately after it, so
//! neither the §2.5 scanner nor THIS rewrite (which self-hosts over stele's own sources)
//! reads the engine's prose as notation. Test fixtures build the retired tokens from the
//! constants below rather than spelling them, for the same reason.

use crate::model::{DECISION_PREFIX, LANDMARK_ANCHOR_PREFIX};
use crate::parse::{
    MARKER_CLOSE, REGION_BEGIN_PREFIX, REGION_DEFAULT_NAME, REGION_END_MARKER, STELE_INFO,
    is_close_fence, open_fence,
};

// ─── the retired notation (pre-0.3.0) ────────────────────────────────────────

/// The retired landmark comment token, replaced by `※` (§2.5).
pub const LANDMARK_TOKEN: &str = "stele:landmark";
/// The retired claim comment token, replaced by `⊨` (§2.5).
pub const CLAIM_TOKEN: &str = "stele:claim";
/// The retired `anchor:` landmark prefix, replaced by `※ ` (§2.4).
pub const ANCHOR_PREFIX: &str = "lm:";
/// The retired generated-region opening marker prefix, replaced by `<!-- @stele` (§3.1).
pub const REGION_BEGIN: &str = "<!-- stele:begin";
/// The retired generated-region closing marker, replaced by `<!-- @end -->` (§3.1).
pub const REGION_END: &str = "<!-- stele:end -->";

/// The `anchor:` key whose value carries a landmark reference (§2.2).
const ANCHOR_KEY: &str = "anchor:";
/// The `decided_by:` key whose entries carry ADR references (§2.6).
const DECIDED_BY_KEY: &str = "decided_by:";

// ─── the rewrite (§5.1) ──────────────────────────────────────────────────────

/// How a file in `migrate`'s scan scope is read (§5.1). The kind decides which of the
/// three notation slots can appear in it; see the module docs for the scoping rationale.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SourceKind {
    /// Any file with no bundled markdown handling: a lexical per-line token rewrite.
    Code,
    /// Markdown that is not a node file: HTML-comment tokens only.
    Markdown,
    /// A node file (`AGENTS.md`): HTML-comment tokens, the authored block's fields, and
    /// the §3.1 region markers.
    Node,
}

/// Rewrite `contents` from the pre-0.3.0 notation to the glyph notation (§2.5/§5.1),
/// or `None` when nothing changed — the idempotence a second `stele migrate` relies on.
pub fn rewrite(kind: SourceKind, contents: &str) -> Option<String> {
    let out = match kind {
        SourceKind::Code => rewrite_code(contents),
        SourceKind::Markdown => rewrite_markdown(contents, false),
        SourceKind::Node => rewrite_markdown(contents, true),
    };
    (out != contents).then_some(out)
}

/// Every line of a non-markdown file, token-rewritten lexically. `split_inclusive`
/// preserves each line's own terminator (and a missing final newline) byte-for-byte.
fn rewrite_code(contents: &str) -> String {
    let mut out = String::with_capacity(contents.len());
    for raw in contents.split_inclusive('\n') {
        let (line, eol) = split_eol(raw);
        out.push_str(&rewrite_tokens(line));
        out.push_str(eol);
    }
    out
}

/// A markdown file, with `node` selecting the extra node-file slots (block fields and
/// region markers). Fence state and HTML-comment state are tracked exactly as the §2.5
/// scanner tracks them, so the two agree on what is a declaration.
fn rewrite_markdown(contents: &str, node: bool) -> String {
    let mut out = String::with_capacity(contents.len());
    let mut fence: Option<(char, usize)> = None;
    let mut in_comment = false;
    let mut in_block = false;
    let mut block_seen = false;
    let mut in_decided_list = false;

    for raw in contents.split_inclusive('\n') {
        let (line, eol) = split_eol(raw);
        let rewritten = if let Some((fence_char, open_len)) = fence {
            if is_close_fence(line, fence_char, open_len) {
                fence = None;
                in_block = false;
                in_decided_list = false;
                line.to_string()
            } else if in_block {
                rewrite_block_line(line, &mut in_decided_list)
            } else {
                line.to_string()
            }
        } else if !in_comment && let Some((fence_char, open_len, info)) = open_fence(line) {
            fence = Some((fence_char, open_len));
            if node && info == STELE_INFO && !block_seen {
                block_seen = true;
                in_block = true;
            }
            line.to_string()
        } else {
            let markers = if node && !in_comment {
                rewrite_region_markers(line)
            } else {
                line.to_string()
            };
            rewrite_comment_spans(&markers, &mut in_comment)
        };
        out.push_str(&rewritten);
        out.push_str(eol);
    }
    out
}

/// Split a `split_inclusive('\n')` chunk into its content and its line terminator
/// (`"\n"`, `"\r\n"`, or `""` at an unterminated end of file).
fn split_eol(raw: &str) -> (&str, &str) {
    match raw.strip_suffix('\n') {
        Some(body) => match body.strip_suffix('\r') {
            Some(body) => (body, "\r\n"),
            None => (body, "\n"),
        },
        None => (raw, ""),
    }
}

// ─── comment tokens (§2.5) ───────────────────────────────────────────────────

/// Rewrite every retired comment token in one fragment (a line, or the in-comment part
/// of one). A token is the retired literal followed by at least one space/tab and a
/// non-empty payload — the old grammar's own rule, so exactly what the old scanner
/// counted is what moves. The retired literal AND its following whitespace run collapse
/// to `glyph + one ASCII space`: §2.5 fixes the token shape at one space, so a
/// `stele:landmark`-plus-three-spaces line must not migrate into silent prose. Every
/// other byte on the fragment is preserved.
fn rewrite_tokens(fragment: &str) -> String {
    const TOKENS: [(&str, &str); 2] = [
        (LANDMARK_TOKEN, LANDMARK_ANCHOR_PREFIX),
        (CLAIM_TOKEN, crate::anchors::CLAIM_TOKEN),
    ];
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while !rest.is_empty() {
        let hit = TOKENS
            .iter()
            .find(|(token, _)| rest.starts_with(token))
            .and_then(|(token, glyph)| {
                let after = &rest[token.len()..];
                let spaces = after.len() - after.trim_start_matches([' ', '\t']).len();
                let payload = &after[spaces..];
                let has_payload = payload.chars().next().is_some_and(|c| !c.is_whitespace());
                (spaces > 0 && has_payload).then_some((token.len() + spaces, *glyph))
            });
        match hit {
            Some((consumed, glyph)) => {
                out.push_str(glyph);
                rest = &rest[consumed..];
            }
            None => {
                let ch = rest.chars().next().unwrap_or_default();
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

/// Rewrite the parts of one markdown line that lie inside an HTML comment, carrying the
/// open/closed state to the next line. Bytes outside a comment are copied verbatim — a
/// token in prose or in a fence was never a declaration and must not become one.
fn rewrite_comment_spans(line: &str, in_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        if *in_comment {
            match rest.find(crate::anchors::HTML_COMMENT_CLOSE) {
                Some(close) => {
                    out.push_str(&rewrite_tokens(&rest[..close]));
                    out.push_str(crate::anchors::HTML_COMMENT_CLOSE);
                    rest = &rest[close + crate::anchors::HTML_COMMENT_CLOSE.len()..];
                    *in_comment = false;
                }
                None => {
                    out.push_str(&rewrite_tokens(rest));
                    return out;
                }
            }
        } else {
            match rest.find(crate::anchors::HTML_COMMENT_OPEN) {
                Some(open) => {
                    let consumed = open + crate::anchors::HTML_COMMENT_OPEN.len();
                    out.push_str(&rest[..consumed]);
                    rest = &rest[consumed..];
                    *in_comment = true;
                }
                None => {
                    out.push_str(rest);
                    return out;
                }
            }
        }
    }
}

// ─── region markers (§3.1 item 2) ────────────────────────────────────────────

/// Rewrite the retired region markers on one line. The closing marker maps straight to
/// `<!-- @end -->`; the opening marker keeps its name and annotation, except that a bare
/// `router` — the default name (§3.1) — drops to `<!-- @stele -->`. An annotation always
/// survives verbatim, `router` included, so the root's `· generated … · do not hand-edit`
/// note is never silently deleted. Bytes before the marker (indentation) and after its
/// first `-->` (the one-line empty form's closer) are preserved.
fn rewrite_region_markers(line: &str) -> String {
    let out = line.replace(REGION_END, REGION_END_MARKER);
    let Some(start) = out.find(REGION_BEGIN) else {
        return out;
    };
    let after = &out[start + REGION_BEGIN.len()..];
    if !after.starts_with([' ', '\t']) {
        return out;
    }
    let Some(close) = after.find(MARKER_CLOSE) else {
        return out;
    };
    let inner = after[..close].trim();
    let (name, annotation) = match inner.find(char::is_whitespace) {
        Some(end) => (&inner[..end], inner[end..].trim()),
        None => (inner, ""),
    };
    let name = if name.is_empty() {
        REGION_DEFAULT_NAME
    } else {
        name
    };
    let marker = match (name == REGION_DEFAULT_NAME, annotation.is_empty()) {
        (true, true) => format!("{REGION_BEGIN_PREFIX} {MARKER_CLOSE}"),
        (_, true) => format!("{REGION_BEGIN_PREFIX} {name} {MARKER_CLOSE}"),
        (_, false) => format!("{REGION_BEGIN_PREFIX} {name} {annotation} {MARKER_CLOSE}"),
    };
    format!(
        "{}{marker}{}",
        &out[..start],
        &after[close + MARKER_CLOSE.len()..]
    )
}

// ─── authored block fields (§2.2/§2.6) ───────────────────────────────────────

/// Rewrite one line inside the authored ```` ```stele ```` block: an `anchor:` whose value
/// is a retired `lm:` landmark reference, or a `decided_by:` entry in either YAML list
/// form. The rewrite is textual — never a YAML re-serialization — so comments, quoting
/// style, and key order survive byte-for-byte (§5.1). `in_list` carries the block-list
/// state (`decided_by:` with its entries on following `- ` lines) across calls.
fn rewrite_block_line(line: &str, in_list: &mut bool) -> String {
    let indent = line.len() - line.trim_start().len();
    let (head, trimmed) = line.split_at(indent);

    if *in_list {
        if trimmed.is_empty() {
            return line.to_string();
        }
        if let Some(item) = trimmed.strip_prefix('-')
            && item.starts_with([' ', '\t'])
        {
            let marker = &trimmed[..1];
            return format!("{head}{marker}{}", rewrite_decision_entry(item));
        }
        *in_list = false;
    }

    if let Some(value) = trimmed.strip_prefix(DECIDED_BY_KEY) {
        if value.trim().is_empty() {
            *in_list = true;
            return line.to_string();
        }
        if let (Some(open), Some(close)) = (value.find('['), value.rfind(']'))
            && open < close
        {
            let entries: Vec<String> = value[open + 1..close]
                .split(',')
                .map(rewrite_decision_entry)
                .collect();
            return format!(
                "{head}{DECIDED_BY_KEY}{}[{}]{}",
                &value[..open],
                entries.join(","),
                &value[close + 1..]
            );
        }
        return format!("{head}{DECIDED_BY_KEY}{}", rewrite_decision_entry(value));
    }

    rewrite_anchor_line(head, trimmed).unwrap_or_else(|| line.to_string())
}

/// Rewrite `anchor: lm:<slug>` to `anchor: ※ <slug>` (§2.4), preserving indentation, an
/// optional `- ` list marker, the spacing after the colon, any quoting, and any trailing
/// text. `None` when the line is not an `anchor:` field or its value is not a retired
/// landmark reference (a `path#symbol` anchor is untouched).
fn rewrite_anchor_line(head: &str, trimmed: &str) -> Option<String> {
    let mut key_head = head.to_string();
    let mut rest = trimmed;
    if let Some(item) = rest.strip_prefix('-')
        && item.starts_with([' ', '\t'])
    {
        let spaces = item.len() - item.trim_start().len();
        key_head.push_str(&rest[..1 + spaces]);
        rest = &item[spaces..];
    }
    let value = rest.strip_prefix(ANCHOR_KEY)?;
    let spaces = value.len() - value.trim_start().len();
    if spaces == 0 {
        return None;
    }
    let payload = &value[spaces..];
    let token_end = payload.find(char::is_whitespace).unwrap_or(payload.len());
    let (token, tail) = payload.split_at(token_end);
    let (quote, body) = split_quotes(token);
    let slug = body.strip_prefix(ANCHOR_PREFIX)?;
    let migrated = requote(quote, &format!("{LANDMARK_ANCHOR_PREFIX}{slug}"));
    Some(format!(
        "{key_head}{ANCHOR_KEY}{}{migrated}{tail}",
        &value[..spaces]
    ))
}

/// Rewrite one `decided_by` entry in place, preserving the whitespace around it. A
/// retired entry is the ADR directory (whatever it is — `adr/`, `doc/adr/`, `docs/adr/`)
/// plus the record's number; the number is kept verbatim so the zero padding still
/// matches the filename (§2.6). Anything else — an already-migrated `§ <NNNN>` included
/// — is returned untouched, which is what makes a second run a no-op.
fn rewrite_decision_entry(raw: &str) -> String {
    let lead = raw.len() - raw.trim_start().len();
    let core_end = lead + raw[lead..].trim_end().len();
    let core = &raw[lead..core_end];
    let (quote, body) = split_quotes(core);
    let Some((dir, number)) = body.rsplit_once('/') else {
        return raw.to_string();
    };
    if dir.is_empty() || number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return raw.to_string();
    }
    if !dir
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        return raw.to_string();
    }
    let migrated = requote(quote, &format!("{DECISION_PREFIX}{number}"));
    format!("{}{migrated}{}", &raw[..lead], &raw[core_end..])
}

/// Split a fully-quoted scalar into its quote character and its body; an unquoted (or
/// half-quoted) scalar comes back as `(None, scalar)`.
fn split_quotes(scalar: &str) -> (Option<char>, &str) {
    let mut chars = scalar.chars();
    match chars.next() {
        Some(quote @ ('"' | '\'')) if scalar.len() > 1 && scalar.ends_with(quote) => (
            Some(quote),
            &scalar[quote.len_utf8()..scalar.len() - quote.len_utf8()],
        ),
        _ => (None, scalar),
    }
}

/// Restore the quoting [`split_quotes`] removed.
fn requote(quote: Option<char>, body: &str) -> String {
    match quote {
        Some(quote) => format!("{quote}{body}{quote}"),
        None => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The retired tokens are never spelled literally in this repo's sources (module
    /// docs): `migrate` self-hosts over them, and a literal followed by a space is a
    /// token it would rewrite. Fixtures build them from the constants instead.
    fn landmark(slug: &str) -> String {
        format!("{LANDMARK_TOKEN} {slug}")
    }

    fn claim(addr: &str) -> String {
        format!("{CLAIM_TOKEN} {addr}")
    }

    fn code(contents: &str) -> String {
        rewrite(SourceKind::Code, contents).unwrap_or_else(|| contents.to_string())
    }

    fn node(contents: &str) -> String {
        rewrite(SourceKind::Node, contents).unwrap_or_else(|| contents.to_string())
    }

    // ─── comment tokens (§2.5) ───────────────────────────────────────────────

    #[test]
    fn code_tokens_become_glyphs_and_keep_the_rest_of_the_line() {
        let src = format!(
            "  # {} — the cap\n  # {}\n",
            landmark("refund-cap"),
            claim("billing/refund-cap")
        );
        assert_eq!(
            code(&src),
            "  # ※ refund-cap — the cap\n  # ⊨ billing/refund-cap\n"
        );
    }

    #[test]
    fn a_widened_token_gap_collapses_to_the_one_space_shape() {
        let src = format!("// {}   refund-cap\n", LANDMARK_TOKEN);
        assert_eq!(code(&src), "// ※ refund-cap\n");
    }

    #[test]
    fn a_token_with_no_payload_is_not_a_token() {
        let src = format!("// {}\n// {}`\n", LANDMARK_TOKEN, LANDMARK_TOKEN);
        assert!(rewrite(SourceKind::Code, &src).is_none(), "{}", code(&src));
    }

    #[test]
    fn crlf_and_a_missing_final_newline_survive() {
        let src = format!("// {}\r\n// {}", landmark("a"), landmark("b"));
        assert_eq!(code(&src), "// ※ a\r\n// ※ b");
    }

    #[test]
    fn already_glyph_notation_is_a_no_op() {
        assert!(rewrite(SourceKind::Code, "// ※ refund-cap\n").is_none());
    }

    // ─── markdown scoping (§2.5) ─────────────────────────────────────────────

    #[test]
    fn markdown_rewrites_inside_html_comments_only() {
        let src = format!(
            "quoting `{}` in prose\n\n```\n# {}\n```\n\n<!-- {} -->\n",
            LANDMARK_TOKEN,
            landmark("fenced"),
            landmark("declared")
        );
        let out = rewrite(SourceKind::Markdown, &src).expect("the comment token migrates");
        assert!(
            out.contains(&format!("`{LANDMARK_TOKEN}` in prose")),
            "{out}"
        );
        assert!(
            out.contains(&format!("# {}\n", landmark("fenced"))),
            "{out}"
        );
        assert!(out.contains("<!-- ※ declared -->"), "{out}");
    }

    #[test]
    fn markdown_multiline_comment_interior_migrates() {
        let src = format!("text\n<!--\n{}\n-->\n", landmark("multi"));
        let out = rewrite(SourceKind::Markdown, &src).expect("migrated");
        assert_eq!(out, "text\n<!--\n※ multi\n-->\n");
    }

    // ─── region markers (§3.1 item 2) ────────────────────────────────────────

    #[test]
    fn a_bare_router_region_drops_its_name() {
        let src = format!("{REGION_BEGIN} router -->\n{REGION_END}\n");
        assert_eq!(node(&src), "<!-- @stele -->\n<!-- @end -->\n");
    }

    #[test]
    fn a_nameless_region_is_the_router_region() {
        let src = format!("{REGION_BEGIN} -->\n{REGION_END}\n");
        assert_eq!(node(&src), "<!-- @stele -->\n<!-- @end -->\n");
    }

    #[test]
    fn a_router_annotation_survives_verbatim() {
        let src = format!("{REGION_BEGIN} router · generated · do not hand-edit -->\n");
        assert_eq!(
            node(&src),
            "<!-- @stele router · generated · do not hand-edit -->\n"
        );
    }

    #[test]
    fn a_non_router_name_is_kept() {
        let src = format!("{REGION_BEGIN} index -->\n");
        assert_eq!(node(&src), "<!-- @stele index -->\n");
    }

    #[test]
    fn the_one_line_empty_form_migrates_whole() {
        let src = format!("{REGION_BEGIN} router -->{REGION_END}\n");
        assert_eq!(node(&src), "<!-- @stele --><!-- @end -->\n");
    }

    #[test]
    fn a_region_marker_inside_a_fence_is_quoted_code() {
        let src = format!("```markdown\n{REGION_BEGIN} router -->\n```\n");
        assert!(rewrite(SourceKind::Node, &src).is_none(), "{}", node(&src));
    }

    // ─── authored block fields (§2.2/§2.6) ───────────────────────────────────

    #[test]
    fn an_anchor_field_takes_the_landmark_glyph() {
        let src = "```stele\ninvariants:\n  - claim: x\n    anchor: lm:refund-cap\n```\n";
        assert!(node(src).contains("anchor: ※ refund-cap"), "{}", node(src));
    }

    #[test]
    fn a_symbol_anchor_is_untouched() {
        let src = "```stele\ninvariants:\n  - claim: x\n    anchor: money.ts#Money\n```\n";
        assert!(rewrite(SourceKind::Node, src).is_none());
    }

    #[test]
    fn a_quoted_anchor_keeps_its_quotes() {
        let src = "```stele\ninvariants:\n  - claim: x\n    anchor: \"lm:refund-cap\"\n```\n";
        assert!(
            node(src).contains("anchor: \"※ refund-cap\""),
            "{}",
            node(src)
        );
    }

    #[test]
    fn decided_by_flow_entries_take_the_decision_glyph() {
        let src = "```stele\nedges:\n  decided_by: [adr/0007, docs/adr/0012]\n```\n";
        assert!(
            node(src).contains("decided_by: [§ 0007, § 0012]"),
            "{}",
            node(src)
        );
    }

    #[test]
    fn decided_by_block_entries_take_the_decision_glyph() {
        let src = "```stele\nedges:\n  decided_by:\n    - doc/adr/0007\n    - adr/0012\n  allow: []\n```\n";
        let out = node(src);
        assert!(out.contains("    - § 0007\n    - § 0012\n"), "{out}");
        assert!(out.contains("  allow: []"), "{out}");
    }

    #[test]
    fn a_block_list_ends_at_the_next_key() {
        // `- packages/shared` under a later `depends:` must never be read as a decision.
        let src = "```stele\nedges:\n  decided_by:\n    - adr/0007\n  depends:\n    - apps/web/0001\n```\n";
        let out = node(src);
        assert!(out.contains("    - § 0007\n"), "{out}");
        assert!(out.contains("    - apps/web/0001\n"), "{out}");
    }

    #[test]
    fn an_already_migrated_decision_is_untouched() {
        let src = "```stele\nedges:\n  decided_by: [§ 0007]\n```\n";
        assert!(rewrite(SourceKind::Node, src).is_none());
    }

    #[test]
    fn block_fields_are_rewritten_only_in_the_stele_block() {
        let src = "```yaml\nanchor: lm:not-a-node\n```\n\n```stele\nkind: system\n```\n";
        assert!(rewrite(SourceKind::Node, src).is_none(), "{}", node(src));
    }

    #[test]
    fn a_comment_in_the_block_survives_the_field_rewrite() {
        let src = "```stele\nedges:\n  decided_by: [adr/0007] # integer cents\n```\n";
        assert!(
            node(src).contains("decided_by: [§ 0007] # integer cents"),
            "{}",
            node(src)
        );
    }
}
