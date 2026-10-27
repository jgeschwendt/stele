//! Budget (§4.4): the `claude`/`codex`/`node` size profiles, the bundled tokenizer,
//! largest-contributor segmentation, and the render's number formatting.

use super::{Context, Finding};
use crate::config::AssertionClass;
use crate::emit;
use crate::model::{Node, Result, SYSTEM_ID};
use crate::parse::{
    self, MARKER_CLOSE, REGION_BEGIN_PREFIX, REGION_DEFAULT_NAME, REGION_END_MARKER, STELE_INFO,
};
use std::sync::OnceLock;

/// Bytes per KiB, the §4.4 `codex` cap unit.
const BYTES_PER_KIB: usize = 1024;
/// Thousands-grouping width for the §4.4 token render (`1,410 tokens`).
const THOUSANDS_GROUP: usize = 3;

/// The budget class (§4.4): three profiles over materialized AGENTS.md content, each
/// finding tagged with its profile (the class stays `budget` for `--only`/`disable`).
/// `claude` counts the root's always-loaded set in TOKENS against `budget.claude_root`;
/// `codex` counts the worst root→leaf concatenation chain in BYTES against
/// `budget.codex_cap`; `node` counts each declared-budget node's own file in TOKENS.
/// Emission order (claude, codex, node) leaves a clean `codex`+`node` pair when the
/// root fits its `claude_root` ceiling, as in EXAMPLE 8.5.
pub(super) fn budget(ctx: &Context) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    findings.extend(budget_claude(ctx)?);
    findings.extend(budget_codex(ctx)?);
    findings.extend(budget_node(ctx)?);
    Ok(findings)
}

/// `claude` profile (§4.4): the root node's always-loaded set — the root AGENTS.md
/// alone, since the root has no ancestors — counted in TOKENS against
/// `budget.claude_root` (default 2000). The always-loaded set generalizes to a node's
/// file plus its ancestors' (§4.4), but the ceiling is calibrated for the root. A graph
/// with no root AGENTS.md has no always-loaded set.
fn budget_claude(ctx: &Context) -> Result<Vec<Finding>> {
    let ceiling = ctx.config.budget.claude_root as usize;
    let mut findings = Vec::new();
    for root in ctx.graph.nodes.iter().filter(|n| n.id == SYSTEM_ID) {
        let tokens = count_tokens(&emit::materialized_content(ctx.source_root, root)?);
        if tokens > ceiling {
            findings.push(
                Finding::error(
                    AssertionClass::Budget,
                    Some(root.id.clone()),
                    format!(
                        "always-loaded set for {} renders {} tokens > claude_root {}",
                        root.id,
                        fmt_thousands(tokens),
                        fmt_thousands(ceiling),
                    ),
                )
                .profile("claude"),
            );
        }
    }
    Ok(findings)
}

/// `codex` profile (§4.4): for every node the root→node concatenation chain, counted
/// in BYTES against `budget.codex_cap` (default 32768, key `project_doc_max_bytes` in
/// the message). Reports the single WORST overflowing chain — its leaf id and size in
/// KiB to one decimal (EXAMPLE 8.5 `34.1 KiB`).
fn budget_codex(ctx: &Context) -> Result<Vec<Finding>> {
    let cap = ctx.config.budget.codex_cap as usize;
    let mut worst: Option<(&Node, usize)> = None;
    for node in &ctx.graph.nodes {
        let chain = emit::chain_files(ctx.tracked, node);
        let bytes = emit::chain_content(ctx.source_root, &chain)?.len();
        if bytes > cap && worst.is_none_or(|(_, w)| bytes > w) {
            worst = Some((node, bytes));
        }
    }
    let Some((leaf, bytes)) = worst else {
        return Ok(Vec::new());
    };
    Ok(vec![
        Finding::error(
            AssertionClass::Budget,
            Some(leaf.id.clone()),
            format!(
                "root chain {} → {} > {} default cap (project_doc_max_bytes) — \
                 Codex truncates the overflow (vendor docs report silent)",
                leaf.id,
                fmt_kib_tenths(bytes),
                fmt_kib_cap(cap),
            ),
        )
        .profile("codex"),
    ])
}

/// `node` profile (§4.4): every node with a declared `budget` whose own materialized
/// file exceeds it (in TOKENS). Names the largest contributing block — the authored
/// `stele` block, a generated region, or a free-prose section (named by its first
/// heading), whichever holds the most tokens (EXAMPLE 8.5).
fn budget_node(ctx: &Context) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for node in &ctx.graph.nodes {
        let Some(budget) = node.budget else {
            continue;
        };
        let content = emit::materialized_content(ctx.source_root, node)?;
        let tokens = count_tokens(&content);
        if tokens as u64 <= budget {
            continue;
        }
        let mut finding = Finding::error(
            AssertionClass::Budget,
            Some(node.id.clone()),
            format!(
                "{} renders {} tokens > declared budget {}",
                node.id,
                fmt_thousands(tokens),
                fmt_thousands(budget as usize),
            ),
        )
        .profile("node");
        if let Some((descriptor, block_tokens)) = largest_contributor(&content) {
            finding = finding.detail(format!(
                "largest contributor: {descriptor} ({} tokens)",
                fmt_thousands(block_tokens)
            ));
        }
        findings.push(finding);
    }
    Ok(findings)
}

// ─── the bundled tokenizer (§4.4) ──────────────────────────────────────────────

/// The bundled cl100k_base-class tokenizer (§4.4), built once and reused. Its identity
/// is folded into the lock version (`lock::LOCK_VERSION`), so a swap is a visible bump.
/// Decoding the vocab costs ~20 ms, so `check` warms it on a thread beside the graph build.
pub fn tokenizer() -> &'static tiktoken_rs::CoreBPE {
    static TOKENIZER: OnceLock<tiktoken_rs::CoreBPE> = OnceLock::new();
    TOKENIZER.get_or_init(|| tiktoken_rs::cl100k_base().expect("bundled cl100k vocab decodes"))
}

/// Token count of `text` under the bundled tokenizer (§4.4). `encode_ordinary` never
/// interprets special-token strings, so arbitrary doc content counts deterministically.
fn count_tokens(text: &str) -> usize {
    tokenizer().encode_ordinary(text).len()
}

// ─── largest-contributor segmentation (§4.4) ───────────────────────────────────

/// The largest §4.4 budget contributor in a materialized AGENTS.md: the block with the
/// most tokens among the authored `stele` block, each generated region, and each
/// free-prose section. `None` for a file with no non-blank content.
fn largest_contributor(content: &str) -> Option<(String, usize)> {
    segments(content)
        .into_iter()
        .map(|(descriptor, text)| (descriptor, count_tokens(&text)))
        .max_by_key(|(_, tokens)| *tokens)
}

/// Split a materialized AGENTS.md into its §4.4 budget segments in document order: the
/// authored `stele` fenced block, each generated `@stele`…`@end` region, and
/// each free-prose span between them. Each segment is `(descriptor, text)`.
fn segments(content: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let mut segs: Vec<(String, String)> = Vec::new();
    let mut prose = String::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(end) = stele_block_end(&lines, i) {
            flush_prose(&mut prose, &mut segs);
            segs.push((
                "authored stele block".to_string(),
                join_lines(&lines[i..=end]),
            ));
            i = end + 1;
        } else if let Some((name, end)) = generated_region(&lines, i) {
            flush_prose(&mut prose, &mut segs);
            segs.push((
                format!("generated {name} region"),
                join_lines(&lines[i..=end]),
            ));
            i = end + 1;
        } else {
            prose.push_str(lines[i]);
            prose.push('\n');
            i += 1;
        }
    }
    flush_prose(&mut prose, &mut segs);
    segs
}

/// Emit the accumulated free-prose span as a segment (named by its first heading, §4.4)
/// and reset the buffer; a blank-only span contributes nothing.
fn flush_prose(prose: &mut String, segs: &mut Vec<(String, String)>) {
    if prose.trim().is_empty() {
        prose.clear();
        return;
    }
    let descriptor = match first_heading(prose) {
        Some(heading) => format!("unmanaged prose block \"{heading}\""),
        None => "unmanaged prose block (untitled)".to_string(),
    };
    segs.push((descriptor, std::mem::take(prose)));
}

/// If line `i` opens the authored `stele` fence (the §3.1 scanner's own fence rules), the
/// index of its closing fence (or the last line for an unterminated block); else `None`.
fn stele_block_end(lines: &[&str], i: usize) -> Option<usize> {
    let (fence_char, fence_len, _) = parse::open_fence(lines[i]).filter(|f| f.2 == STELE_INFO)?;
    let close =
        (i + 1..lines.len()).find(|&j| parse::is_close_fence(lines[j], fence_char, fence_len));
    Some(close.unwrap_or(lines.len() - 1))
}

/// If line `i` opens a generated region (`<!-- @stele [name] … -->`), its name and the
/// index of the line carrying `<!-- @end -->` (or the last line if unterminated); else
/// `None`. The name is optional (§3.1 item 2), defaulting to `router`; a single-line
/// `@stele`…`@end` region closes on its own line.
fn generated_region(lines: &[&str], i: usize) -> Option<(String, usize)> {
    let after = lines[i].split(REGION_BEGIN_PREFIX).nth(1)?;
    let name = after.split(MARKER_CLOSE).next().unwrap_or_default();
    let name = name
        .split_whitespace()
        .next()
        .unwrap_or(REGION_DEFAULT_NAME);
    let end = (i..lines.len()).find(|&j| lines[j].contains(REGION_END_MARKER));
    Some((name.to_string(), end.unwrap_or(lines.len() - 1)))
}

/// The first ATX heading line in `text` (leading `#`), trimmed, for a §4.4 free-prose
/// descriptor (EXAMPLE 8.5 `"## Style notes"`).
fn first_heading(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim_start)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim().to_string())
}

/// Rejoin scanned lines into a segment body (one `\n` per line); exact bytes are
/// immaterial here — this feeds only the relative token sizing of contributors.
fn join_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

// ─── number formatting (§4.4 render) ───────────────────────────────────────────

/// A count with `,` thousands separators (EXAMPLE 8.5 `1,410 tokens`).
fn fmt_thousands(n: usize) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(THOUSANDS_GROUP) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A byte size in KiB to one decimal (EXAMPLE 8.5 `34.1 KiB`) — the overflowing size.
fn fmt_kib_tenths(bytes: usize) -> String {
    format!("{:.1} KiB", bytes as f64 / BYTES_PER_KIB as f64)
}

/// The `codex` cap in KiB (EXAMPLE 8.5 `32 KiB`): whole KiB render as an integer, a
/// non-whole configured cap to one decimal.
fn fmt_kib_cap(bytes: usize) -> String {
    if bytes.is_multiple_of(BYTES_PER_KIB) {
        format!("{} KiB", bytes / BYTES_PER_KIB)
    } else {
        format!("{:.1} KiB", bytes as f64 / BYTES_PER_KIB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4.4 render edges: a heading-less prose span is `(untitled)`; a nameless region is
    /// `router`; a line the §3.1 scanner rejects as a fence (indent 4, a 2-run, a backtick
    /// in a backtick fence's info) is prose, never the authored block; a non-whole KiB cap
    /// keeps one decimal.
    #[test]
    fn budget_segments_and_cap_render() {
        let content = "plain\n    ```stele\n``stele\n```stele `\n<!-- @stele -->\n<!-- @end -->\n";
        let names: Vec<String> = segments(content)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            [
                "unmanaged prose block (untitled)",
                "generated router region"
            ]
        );
        assert_eq!(fmt_kib_cap(1536), "1.5 KiB");
    }
}
