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
            depends: raw.edges.depends,
            decided_by: raw.edges.decided_by,
            allow: raw
                .edges
                .allow
                .into_iter()
                .map(|a| crate::model::Allow {
                    edge: a.edge,
                    reason: a.reason,
                })
                .collect(),
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
/// per CommonMark a backtick fence's info may not itself contain a backtick.
fn open_fence(line: &str) -> Option<(char, usize, String)> {
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
/// trailing info (CommonMark).
fn is_close_fence(line: &str, fence_char: char, open_len: usize) -> bool {
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
    fn malformed_derived_slug_is_exit_2() {
        let err = parse(
            "kind: component\n\
             invariants:\n  - claim: a\n    anchor: lm:Bad_Slug\n",
        )
        .unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
    }
}
