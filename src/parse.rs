//! Typed-block parsing from AGENTS.md sources (SPEC §3.1): locate the single
//! `stele` fenced block, then strict-deserialize it into the typed model (§2.2).
//!
//! Two layers. A markdown fence scanner finds the authored block and its
//! file-line offset; strict serde deserialization (`deny_unknown_fields`) turns
//! the block body into the model, with every §5.3 input-error condition mapped to
//! a `file:line` error. Line numbers come from the YAML crate's `location()`,
//! rebased onto the block's start line so errors report the real file position.

use crate::model::{
    Claim, ClaimKind, Edges, Node, NodeKind, PURPOSE_MAX_CHARS, Result, SteleError, derive_slug,
    normalize_id,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The fenced-block info-string language that marks the authored node block.
const STELE_INFO: &str = "stele";
/// CommonMark: a code fence is a run of at least this many fence characters.
const MIN_FENCE_LEN: usize = 3;
/// CommonMark: an opening fence may be indented at most this many spaces.
const MAX_FENCE_INDENT: usize = 3;

/// Parse one AGENTS.md. `Ok(None)` = a valid file that declares no node (§3.1:
/// zero `stele` blocks is the plain-AGENTS.md degradation). `Ok(Some)` = the
/// declared node. `Err` = a §5.3 input error with `file:line`.
///
/// `rel_path` is the repo-root-relative path: it seeds both error reporting and
/// the default (directory-derived) node id (§2.1).
pub fn parse_agents_file(rel_path: &Path, contents: &str) -> Result<Option<Node>> {
    let Some(block) = find_stele_block(rel_path, contents)? else {
        return Ok(None);
    };

    let raw: RawBlock = serde_yaml_ng::from_str(&block.body).map_err(|e| {
        // `location()` lines are 1-based within the block body; body line 1 sits at
        // file line `fence_line + 1`, so file_line = fence_line + location.line().
        let line = e
            .location()
            .map(|loc| block.fence_line + loc.line())
            .unwrap_or(block.fence_line);
        // Strip the crate's block-relative " at line L column C" suffix; the real
        // file position is carried by the error's own file:line.
        let message = e.to_string();
        let message = message.split(" at line ").next().unwrap_or(&message);
        SteleError::input(rel_path, line, message.to_string())
    })?;

    validate(rel_path, &block, &raw)?;
    assemble(rel_path, &block, raw).map(Some)
}

// ─── strict deserialization layer (§2.2, §5.3) ──────────────────────────────

/// The authored `kind` (§2.1); only these three are declarable. `adr`/`anchor`
/// are compiled, never authored — an unknown variant is a §5.3 input error, which
/// `deny_unknown_fields`-style variant rejection produces automatically.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AuthoredKind {
    Component,
    Container,
    System,
}

impl From<AuthoredKind> for NodeKind {
    fn from(kind: AuthoredKind) -> Self {
        match kind {
            AuthoredKind::Component => NodeKind::Component,
            AuthoredKind::Container => NodeKind::Container,
            AuthoredKind::System => NodeKind::System,
        }
    }
}

/// The `stele` block body, mirroring §2.2 exactly. `deny_unknown_fields` makes any
/// stray key a §5.3 input error; `kind` is the sole required field.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlock {
    kind: AuthoredKind,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    commands: BTreeMap<String, String>,
    #[serde(default)]
    invariants: Vec<RawClaim>,
    #[serde(default)]
    hazards: Vec<RawClaim>,
    #[serde(default)]
    edges: RawEdges,
    #[serde(default)]
    budget: Option<u64>,
}

/// `claim` and `anchor` are required (a missing one is a serde error); emptiness is
/// caught by [`validate`] (the hearsay gate, §2.4).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaim {
    claim: String,
    anchor: String,
    #[serde(default)]
    enforced_by: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEdges {
    #[serde(default)]
    depends: Vec<String>,
    #[serde(default)]
    decided_by: Vec<String>,
    #[serde(default)]
    allow: Vec<RawAllow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAllow {
    edge: String,
    reason: String,
}

/// Semantic gates that serde cannot express (§2.2/§2.4): the purpose ceiling, the
/// hearsay gate (non-empty claim + anchor), and the mandatory `allow` reason. These
/// have no per-field YAML span, so they anchor at the block's fence line (§5.3
/// permits computing the position from the block start).
fn validate(rel_path: &Path, block: &SteleBlock, raw: &RawBlock) -> Result<()> {
    let at = |message: String| SteleError::input(rel_path, block.fence_line, message);

    if let Some(purpose) = &raw.purpose {
        let count = purpose.chars().count();
        if count > PURPOSE_MAX_CHARS {
            return Err(at(format!(
                "purpose is {count} characters; the ceiling is {PURPOSE_MAX_CHARS} (§2.2)"
            )));
        }
    }

    for claim in raw.invariants.iter().chain(raw.hazards.iter()) {
        if claim.claim.trim().is_empty() {
            return Err(at("claim text must be non-empty (§2.4)".to_string()));
        }
        if claim.anchor.trim().is_empty() {
            return Err(at(
                "claim anchor must be non-empty — provenance or it doesn't compile (§2.4 hearsay gate)"
                    .to_string(),
            ));
        }
    }

    for allow in &raw.edges.allow {
        if allow.edge.trim().is_empty() {
            return Err(at(
                "allow entry requires a non-empty edge target (§4.2)".to_string()
            ));
        }
        if allow.reason.trim().is_empty() {
            return Err(at(
                "allow entry requires a non-empty reason (§4.2)".to_string()
            ));
        }
    }

    Ok(())
}

/// Lower a validated [`RawBlock`] into the typed [`Node`], resolving the id (§2.1)
/// and leaving compiled-only slots empty for later phases.
fn assemble(rel_path: &Path, block: &SteleBlock, raw: RawBlock) -> Result<Node> {
    let id = resolve_id(rel_path, block, raw.id.as_deref())?;
    let invariants = build_claims(rel_path, block, raw.invariants, ClaimKind::Invariant)?;
    let hazards = build_claims(rel_path, block, raw.hazards, ClaimKind::Hazard)?;
    reject_duplicate_slugs(rel_path, block, &invariants, &hazards)?;
    Ok(Node {
        kind: raw.kind.into(),
        id,
        purpose: raw.purpose,
        commands: raw.commands,
        invariants,
        hazards,
        edges: Edges {
            depends: normalize_targets(rel_path, block, "depends", raw.edges.depends)?,
            decided_by: normalize_targets(rel_path, block, "decided_by", raw.edges.decided_by)?,
            allow: raw
                .edges
                .allow
                .into_iter()
                .map(|a| {
                    Ok(crate::model::Allow {
                        edge: normalize_target(rel_path, block, "allow.edge", &a.edge)?,
                        reason: a.reason,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        },
        budget: raw.budget,
        source: rel_path.to_path_buf(),
        extracted_imports: Vec::new(),
        contains: Vec::new(),
    })
}

/// Lower a claim list into typed [`Claim`]s, deriving each slug from its anchor
/// (§2.4). A malformed derived slug is a §5.3 input error anchored at the block fence.
fn build_claims(
    rel_path: &Path,
    block: &SteleBlock,
    raw: Vec<RawClaim>,
    kind: ClaimKind,
) -> Result<Vec<Claim>> {
    raw.into_iter()
        .map(|c| {
            let slug = derive_slug(&c.anchor)
                .map_err(|message| SteleError::input(rel_path, block.fence_line, message))?;
            Ok(Claim::authored(
                kind,
                c.claim,
                c.anchor,
                c.enforced_by,
                slug,
            ))
        })
        .collect()
}

/// Two claims in one node whose derived slugs collide → exit 2 naming both (§2.4).
/// Slugs are the claim's lock id, so a collision would make the address ambiguous.
fn reject_duplicate_slugs(
    rel_path: &Path,
    block: &SteleBlock,
    invariants: &[Claim],
    hazards: &[Claim],
) -> Result<()> {
    let mut seen: std::collections::HashMap<&str, &Claim> = std::collections::HashMap::new();
    for claim in invariants.iter().chain(hazards.iter()) {
        if let Some(prior) = seen.insert(&claim.slug, claim) {
            return Err(SteleError::input(
                rel_path,
                block.fence_line,
                format!(
                    "two claims derive the same slug {:?}: anchors {:?} and {:?} collide (§2.4)",
                    claim.slug, prior.anchor, claim.anchor
                ),
            ));
        }
    }
    Ok(())
}

/// Normalize a list of edge targets (§2.1), preserving order. Every edge target is
/// compared against normalized node ids downstream (§4.2 depends/allow, §2.6 decided_by)
/// and serialized into the lock, so it MUST be normalized at parse time — an unnormalized
/// `./apps/web` or `apps/web/` would never match the node id `apps/web` (§2.1: normalize
/// before ANY comparison).
fn normalize_targets(
    rel_path: &Path,
    block: &SteleBlock,
    field: &str,
    targets: Vec<String>,
) -> Result<Vec<String>> {
    targets
        .iter()
        .map(|t| normalize_target(rel_path, block, field, t))
        .collect()
}

/// Normalize one edge target (§2.1), mapping a `..`-segment or OS-absolute rejection to
/// a §5.3 input error (exit 2) naming the offending `edges.<field>`.
fn normalize_target(rel_path: &Path, block: &SteleBlock, field: &str, raw: &str) -> Result<String> {
    normalize_id(raw).map_err(|message| {
        SteleError::input(
            rel_path,
            block.fence_line,
            format!("edges.{field}: {message}"),
        )
    })
}

/// The node id (§2.1): an explicit `id:` override normalized, else the declaring
/// directory (the AGENTS.md's parent), with the repo root mapping to `/`.
fn resolve_id(rel_path: &Path, block: &SteleBlock, override_id: Option<&str>) -> Result<String> {
    let raw = match override_id {
        Some(id) => id.to_string(),
        None => match rel_path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_string_lossy().into_owned(),
            _ => "/".to_string(),
        },
    };
    normalize_id(&raw).map_err(|message| SteleError::input(rel_path, block.fence_line, message))
}

// ─── generated-region markers (§3.1 item 2) ──────────────────────────────────

/// The opening-marker prefix: `<!-- stele:begin` then the region name (and any free
/// annotation) then `-->`.
const REGION_BEGIN_PREFIX: &str = "<!-- stele:begin";
/// The closing marker, matched exactly (after trimming) — no name, no annotation.
const REGION_END_MARKER: &str = "<!-- stele:end -->";
/// The trailing `-->` every marker line closes with.
const MARKER_CLOSE: &str = "-->";

/// A located generated region (§3.1 item 2): the single marker-fenced span `emit`
/// owns. `content_start`/`content_end` bracket the engine-owned bytes strictly
/// between the markers (the begin marker line's trailing LF through the byte before
/// the end marker line), so the markers and everything outside them stay verbatim.
pub struct Region {
    pub content_end: usize,
    pub content_start: usize,
    pub name: String,
}

/// Locate the one generated region in a node's AGENTS.md (§3.1 item 2). `Ok(None)` =
/// no region (an authored-only file `emit` reports needing `init`). Malformed —
/// begin without end, end without begin, or a second begin (two regions or nesting)
/// — is a §5.3 input error (exit 2) naming the file.
pub fn find_region(rel_path: &Path, contents: &str) -> Result<Option<Region>> {
    // (name, content_start, begin_line) of an open, not-yet-closed begin marker.
    let mut open: Option<(String, usize, usize)> = None;
    let mut region: Option<Region> = None;
    // Fenced-code state, tracked exactly like the anchor scanner (§2.5): a
    // marker-lookalike inside a ``` fence is literal content, never a real region
    // boundary — otherwise `emit` would overwrite authored bytes between them (§3.1).
    let mut fence: Option<(char, usize)> = None;
    let mut offset = 0;
    for (index, raw_line) in contents.split_inclusive('\n').enumerate() {
        let line_no = index + 1;
        let line_start = offset;
        offset += raw_line.len();
        let mut stripped = raw_line;
        if let Some(s) = stripped.strip_suffix('\n') {
            stripped = s;
        }
        if let Some(s) = stripped.strip_suffix('\r') {
            stripped = s;
        }
        if let Some((fence_char, open_len)) = fence {
            if is_close_fence(stripped, fence_char, open_len) {
                fence = None;
            }
            continue;
        }
        if let Some((fence_char, open_len, _)) = open_fence(stripped) {
            fence = Some((fence_char, open_len));
            continue;
        }
        let line = stripped.trim();
        if let Some((name, consumed)) = parse_begin_marker(line) {
            if open.is_some() || region.is_some() {
                return Err(SteleError::input(
                    rel_path,
                    line_no,
                    "a second stele:begin marker; exactly one generated region per file (§3.1)",
                ));
            }
            if line[consumed..].trim() == REGION_END_MARKER {
                // The one-line empty form (§3.1/§7): begin immediately followed by end on
                // one line. The region owns no bytes — content_start == content_end sits
                // just past the begin marker, so `emit` reproduces the whole line verbatim.
                let lead = stripped.len() - stripped.trim_start().len();
                let after_begin = line_start + lead + consumed;
                region = Some(Region {
                    content_end: after_begin,
                    content_start: after_begin,
                    name,
                });
            } else {
                open = Some((name, offset, line_no));
            }
        } else if line == REGION_END_MARKER {
            match open.take() {
                Some((name, content_start, _)) => {
                    region = Some(Region {
                        content_end: line_start,
                        content_start,
                        name,
                    });
                }
                None => {
                    return Err(SteleError::input(
                        rel_path,
                        line_no,
                        "a stele:end marker with no matching stele:begin (§3.1)",
                    ));
                }
            }
        }
    }
    if let Some((_, _, begin_line)) = open {
        return Err(SteleError::input(
            rel_path,
            begin_line,
            "a stele:begin marker with no matching stele:end (§3.1)",
        ));
    }
    Ok(region)
}

/// If `line` is a begin marker, return `(region name, byte offset within `line` just
/// past the marker's FIRST closing `-->`)`. The region name is the first whitespace
/// token of the annotation, which ends at that first `-->`; any bytes after it are
/// further content (§3.1 — the one-line empty form `…begin <name> --><!-- stele:end
/// -->` carries its end marker there, so the closer must not be swallowed as
/// annotation). `None` for any other line, the end marker included.
fn parse_begin_marker(line: &str) -> Option<(String, usize)> {
    let inner = line.strip_prefix(REGION_BEGIN_PREFIX)?;
    let close = inner.find(MARKER_CLOSE)?;
    let name = inner[..close].split_whitespace().next()?.to_string();
    Some((name, REGION_BEGIN_PREFIX.len() + close + MARKER_CLOSE.len()))
}

// ─── markdown fence scanner (§3.1 item 1) ────────────────────────────────────

/// The located authored block: its body (for YAML parsing) and the 1-based file
/// line of its opening fence (for rebasing YAML error positions).
struct SteleBlock {
    body: String,
    fence_line: usize,
}

/// One fenced code block found while scanning: its info-string language and the
/// 1-based file lines of its opening and closing fences (block-range reporting).
struct Fence {
    body: String,
    close_line: usize,
    info: String,
    open_line: usize,
}

/// Locate the single authored `stele` block (§3.1 item 1): it must be the FIRST
/// fenced code block in the file; two or more `stele` blocks → exit 2 naming both
/// ranges; a `stele` block that is not the first fence → exit 2; zero → `Ok(None)`.
fn find_stele_block(rel_path: &Path, contents: &str) -> Result<Option<SteleBlock>> {
    let fences = scan_fences(contents);
    let stele: Vec<&Fence> = fences.iter().filter(|f| f.info == STELE_INFO).collect();

    match stele.as_slice() {
        [] => Ok(None),
        [only] => {
            let first_fence = &fences[0];
            if first_fence.open_line != only.open_line {
                return Err(SteleError::input(
                    rel_path,
                    only.open_line,
                    format!(
                        "the stele block must be the first fenced code block in the file; \
                         a fenced block opens earlier at line {} (§3.1)",
                        first_fence.open_line
                    ),
                ));
            }
            Ok(Some(SteleBlock {
                body: only.body.clone(),
                fence_line: only.open_line,
            }))
        }
        [first, second, ..] => Err(SteleError::input(
            rel_path,
            first.open_line,
            format!(
                "two stele blocks in one file: lines {}–{} and {}–{}; exactly one is allowed (§3.1)",
                first.open_line, first.close_line, second.open_line, second.close_line
            ),
        )),
    }
}

/// Scan every fenced code block (CommonMark-flavored, sufficient for AGENTS.md):
/// a run of ≥3 backticks or tildes opens a block; a run of the same character at
/// least as long, with no trailing info, closes it. Bytes between markers are the
/// body verbatim. An unterminated fence runs to end-of-file.
fn scan_fences(contents: &str) -> Vec<Fence> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut fences = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((fence_char, fence_len, info)) = open_fence(lines[i]) else {
            i += 1;
            continue;
        };
        let open_line = i + 1;
        let mut body = String::new();
        let mut j = i + 1;
        let mut closed_at = None;
        while j < lines.len() {
            if is_close_fence(lines[j], fence_char, fence_len) {
                closed_at = Some(j + 1);
                break;
            }
            body.push_str(lines[j]);
            body.push('\n');
            j += 1;
        }
        fences.push(Fence {
            body,
            close_line: closed_at.unwrap_or(lines.len()),
            info,
            open_line,
        });
        // Resume after the closing fence (or at EOF for an unterminated block).
        i = closed_at.unwrap_or(j);
    }
    fences
}

/// If `line` opens a fence, return `(fence_char, run_length, info_language)`. The
/// info language is the first whitespace-delimited token after the fence run;
/// per CommonMark a backtick fence's info may not itself contain a backtick. Shared
/// with the markdown anchor scanner (§2.5), which masks fenced code before scanning.
pub(crate) fn open_fence(line: &str) -> Option<(char, usize, String)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > MAX_FENCE_INDENT {
        return None;
    }
    let rest = &line[indent..];
    let fence_char = rest.chars().next().filter(|&c| c == '`' || c == '~')?;
    let run = rest.chars().take_while(|&c| c == fence_char).count();
    if run < MIN_FENCE_LEN {
        return None;
    }
    let info_raw = &rest[run..];
    if fence_char == '`' && info_raw.contains('`') {
        return None;
    }
    let info = info_raw.split_whitespace().next().unwrap_or("").to_string();
    Some((fence_char, run, info))
}

/// A closing fence: only the fence character, run length ≥ the opener's, and no
/// trailing info (CommonMark). Shared with the markdown anchor scanner (§2.5).
pub(crate) fn is_close_fence(line: &str, fence_char: char, open_len: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > MAX_FENCE_INDENT {
        return false;
    }
    let rest = &line[indent..];
    let run = rest.chars().take_while(|&c| c == fence_char).count();
    run >= open_len && rest[run..].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExitCode;

    fn parse(body: &str) -> Result<Option<Node>> {
        let contents = format!("# node\n\n```stele\n{body}```\n");
        parse_agents_file(Path::new("apps/web/lib/billing/AGENTS.md"), &contents)
    }

    #[test]
    fn derives_and_stores_claim_slugs_on_the_node() {
        let node = parse(
            "kind: component\n\
             invariants:\n  - claim: cap enforced\n    anchor: lm:refund-cap\n\
             hazards:\n  - claim: symbol bound\n    anchor: refund.ex#changeset\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(node.invariants[0].slug, "refund-cap");
        assert_eq!(node.hazards[0].slug, "changeset");
    }

    #[test]
    fn duplicate_derived_slug_in_one_node_is_exit_2_naming_both() {
        // A landmark and a path#symbol anchor that derive the same slug collide.
        let err = parse(
            "kind: component\n\
             invariants:\n  - claim: a\n    anchor: lm:changeset\n\
             hazards:\n  - claim: b\n    anchor: refund.ex#changeset\n",
        )
        .unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
        assert!(err.message.contains("lm:changeset"), "{}", err.message);
        assert!(
            err.message.contains("refund.ex#changeset"),
            "{}",
            err.message
        );
    }

    #[test]
    fn edge_targets_are_normalized_before_storage() {
        // `./x` and `x/` denote the same node id `x` (§2.1); an unnormalized target would
        // never match the normalized node id it names — the F4 defect.
        let node = parse(
            "kind: component\nedges:\n  depends: [./apps/web, packages/shared/]\n  \
             decided_by: [./adr/0007]\n  allow:\n    - edge: apps//worker\n      \
             reason: runtime DI\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(node.edges.depends, vec!["apps/web", "packages/shared"]);
        assert_eq!(node.edges.decided_by, vec!["adr/0007"]);
        assert_eq!(node.edges.allow[0].edge, "apps/worker");
    }

    #[test]
    fn edge_target_with_parent_segment_is_exit_2_naming_the_field() {
        let err = parse("kind: component\nedges:\n  depends: [../escape]\n").unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
        assert!(err.message.contains("edges.depends"), "{}", err.message);
        assert!(err.message.contains(".."), "{}", err.message);
    }

    #[test]
    fn absolute_allow_edge_is_exit_2_naming_the_field() {
        let err =
            parse("kind: component\nedges:\n  allow:\n    - edge: /abs/path\n      reason: x\n")
                .unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
        assert!(err.message.contains("edges.allow.edge"), "{}", err.message);
    }

    #[test]
    fn malformed_derived_slug_is_exit_2() {
        let err = parse(
            "kind: component\n\
             invariants:\n  - claim: a\n    anchor: lm:Bad_Slug\n",
        )
        .unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
    }
}
