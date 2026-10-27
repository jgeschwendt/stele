//! Freshness (§4.5): region-digest drift or churn since the lock's `verified`
//! watermark, the staling-commit walk, and `stele blame` (§5.1), which reuses both.

use super::{Context, Finding, git, par_map};
use crate::anchors::{self, RegionDigest};
use crate::config::{AssertionClass, Config};
use crate::lock::{Lock, LockVerified};
use crate::model::{Claim, ClaimLookup, Graph, Node, Resolution, Result, SYSTEM_ID, SteleError};
use serde_json::{Value, json};
use std::path::Path;

/// Short-`sha` width in the freshness render (EXAMPLE 8.4 `e3f19ac`).
const SHORT_SHA_LEN: usize = 7;
/// Short-digest prefix width; rendered with a trailing `…` (EXAMPLE 8.4 `9c41…`).
const SHORT_DIGEST_LEN: usize = 4;
/// The horizontal-ellipsis (U+2026) that terminates a short digest (EXAMPLE 8.4).
const DIGEST_ELLIPSIS: char = '…';

/// The freshness class (§4.5). For every RESOLVED claim carrying a `verified` watermark
/// in the committed lock:
/// - Parser-backed (a stamped `digest`): recompute the bound region's digest from the
///   CURRENT working tree; a drift stales the claim — UNLESS `enforced_by` backs it, in
///   which case its guard (run by the same CI) is the freshness proof and the digest
///   change is exempt (EXAMPLE 8.4 note 2). Unresolved anchors are referential's
///   problem (§4.1), not freshness's, and are skipped.
/// - Parser-less (no digest): fall to a churn count — commits touching the anchored
///   file since `verified.sha`, thresholded by `freshness.churn_threshold` (prose-only)
///   or the longer `freshness.enforced_leash` (`enforced_by`-backed), with per-node
///   overrides (§3.4). An unset threshold disables the fallback for that claim.
pub(super) fn freshness(ctx: &Context) -> Result<Vec<Finding>> {
    let claims: Vec<_> = ctx
        .graph
        .nodes
        .iter()
        .flat_map(|node| {
            node.invariants
                .iter()
                .chain(&node.hazards)
                .map(move |claim| (node, claim))
        })
        .collect();
    par_map(&claims, |(node, claim)| freshness_claim(ctx, node, claim))
        .into_iter()
        .filter_map(Result::transpose)
        .collect()
}

/// The freshness verdict for one claim (§4.5), or `None` when it is fresh, unresolved,
/// unwatermarked, exempt, or (parser-less) below/without a churn threshold.
fn freshness_claim(ctx: &Context, node: &Node, claim: &Claim) -> Result<Option<Finding>> {
    // Unresolved/ambiguous anchors are §4.1's finding, never freshness's.
    let resolved = claim.resolved.as_ref();
    let Some(resolved) = resolved.filter(|_| claim.resolution == Resolution::Resolved) else {
        return Ok(None);
    };
    // The watermark lives only in the committed lock on the check path (§4.5).
    let Some(verified) = committed_verified(ctx.committed, &node.id, &claim.slug) else {
        return Ok(None);
    };
    let file = resolved_file(resolved);

    match &verified.digest {
        // Parser-backed: AST-region digest staling.
        Some(old_digest) => {
            // `enforced_by`-backed claims are exempt from digest-staling (EXAMPLE 8.4).
            if claim.enforced_by.is_some() {
                return Ok(None);
            }
            let current = anchors::region_digest_for_claim(ctx.root, &claim.anchor, resolved)?;
            let Some(current) = current.filter(|c| &c.digest != old_digest) else {
                return Ok(None);
            };
            let staling = staling_commit(ctx.root, file, &claim.anchor, &verified.sha, old_digest)?;
            Ok(Some(digest_drift_finding(
                ctx.graph, node, claim, verified, old_digest, &current, staling,
            )))
        }
        // Parser-less: churn-count fallback, region granularity = the anchored file (v1).
        None => {
            let Some(threshold) =
                churn_threshold(ctx.config, &node.id, claim.enforced_by.is_some())
            else {
                return Ok(None);
            };
            // A watermark absent from history (shallow clone) makes churn uncountable —
            // `verified_sha..HEAD` would silently yield 0 and pass a possibly-staled claim
            // (F11). Report the honest failure instead.
            if !sha_reachable(ctx.root, &verified.sha)? {
                return Ok(Some(history_unavailable_finding(
                    ctx.graph, node, claim, verified, file,
                )));
            }
            let count = churn_count(ctx.root, file, &verified.sha)?;
            if count <= threshold as usize {
                return Ok(None);
            }
            Ok(Some(churn_finding(
                ctx.graph, node, claim, verified, file, count, threshold,
            )))
        }
    }
}

/// The committed lock's `verified` watermark for a claim, matched by node id + slug
/// (§4.5); `None` when the claim was never stamped (unresolved at last `build`).
fn committed_verified<'a>(
    committed: &'a Lock,
    node_id: &str,
    slug: &str,
) -> Option<&'a LockVerified> {
    committed
        .nodes
        .get(node_id)?
        .claims
        .iter()
        .find(|c| c.id == slug)?
        .verified
        .as_ref()
}

/// The churn threshold for a parser-less claim (§3.4/§4.5): `enforced_leash` for an
/// `enforced_by`-backed claim (the longer leash — the guard is the proof), else
/// `churn_threshold`. A per-node override (`[freshness.node."<id>"]`) wins over the
/// global; both unset → `None` disables the fallback for that claim.
fn churn_threshold(config: &Config, node_id: &str, enforced: bool) -> Option<u32> {
    let node_cfg = config.freshness.node.get(node_id);
    if enforced {
        node_cfg
            .and_then(|n| n.enforced_leash)
            .or(config.freshness.enforced_leash)
    } else {
        node_cfg
            .and_then(|n| n.churn_threshold)
            .or(config.freshness.churn_threshold)
    }
}

/// The §8.4 digest-drift finding: the `AST digest … changed` message, the verified/now
/// detail, the staling-commit pointer, and the re-affirm fix.
fn digest_drift_finding(
    graph: &Graph,
    node: &Node,
    claim: &Claim,
    verified: &LockVerified,
    old_digest: &str,
    current: &RegionDigest,
    staling: Staling,
) -> Finding {
    let addr = claim_address(graph, &node.id, &claim.slug);
    let staling_line = staling.line(&addr, true);
    Finding::error(
        AssertionClass::Freshness,
        Some(node.id.clone()),
        format!("claim {addr} — AST digest of enclosing region changed"),
    )
    .detail(format!(
        "verified at {} (digest {}), region {} now digests {}",
        short_sha(&verified.sha),
        short_digest(old_digest),
        current.name,
        short_digest(&current.digest),
    ))
    .detail(staling_line)
    .fix("re-read the region, re-affirm or amend the claim, `stele build` re-stamps {sha, digest}")
}

/// The parser-less churn-fallback finding (§4.5): the file crossed its leash. Region
/// granularity is the whole anchored file (a documented v1 limitation).
fn churn_finding(
    graph: &Graph,
    node: &Node,
    claim: &Claim,
    verified: &LockVerified,
    file: &str,
    count: usize,
    threshold: u32,
) -> Finding {
    let addr = claim_address(graph, &node.id, &claim.slug);
    Finding::error(
        AssertionClass::Freshness,
        Some(node.id.clone()),
        format!("claim {addr} — anchored file churned {count} commits past its freshness leash {threshold}"),
    )
    .detail(format!(
        "verified at {}; {file} (parser-less, file-granularity) changed in {count} commit(s) > leash {threshold} — `stele blame {addr}`",
        short_sha(&verified.sha),
    ))
    .fix("re-read the region, re-affirm or amend the claim, `stele build` re-stamps {sha}")
}

/// The shallow-clone freshness finding (§4.5): the watermark commit is not in this repo's
/// history, so churn is uncountable — an honest failure rather than a silent pass (F11).
/// Points at the fetch-depth fix.
fn history_unavailable_finding(
    graph: &Graph,
    node: &Node,
    claim: &Claim,
    verified: &LockVerified,
    file: &str,
) -> Finding {
    let addr = claim_address(graph, &node.id, &claim.slug);
    Finding::error(
        AssertionClass::Freshness,
        Some(node.id.clone()),
        format!("claim {addr} — cannot verify freshness: history unavailable (shallow clone?)"),
    )
    .detail(format!(
        "verified at {}, a commit absent from this repo's history — a shallow / fetch-depth 1 clone cannot compute churn for {file} — `stele blame {addr}`",
        short_sha(&verified.sha),
    ))
    .fix("fetch full history (git fetch --unshallow, or actions/checkout with fetch-depth: 0), then re-run stele check")
}

// ─── the staling-commit walk (§4.5, `stele blame`) ─────────────────────────────

/// A commit that staled a claim: its short sha and subject (EXAMPLE 8.4
/// `b8e02d1 "loosen cap for partial captures"`).
struct StalingCommit {
    short: String,
    subject: String,
}

/// Why a digest-backed claim staled (§4.5), the three honest outcomes of the walk:
///   • `Commit`             — the oldest committed version whose digest diverges;
///   • `Uncommitted`        — no committed version diverges AND the working tree differs
///                            from HEAD, so a local uncommitted edit is the cause;
///   • `HistoryUnavailable` — the watermark commit is not in this repo's history (a
///                            shallow / fetch-depth 1 clone), so the walk cannot run — an
///                            uncommitted-edit claim would be a LIE (F11/F12).
enum Staling {
    Commit(StalingCommit),
    HistoryUnavailable,
    Uncommitted,
}

impl Staling {
    /// The `staling commit: …` attribution line (§4.5). `blame_hint` appends the
    /// `stele blame <addr>` pointer the freshness finding carries (EXAMPLE 8.4); `stele
    /// blame`'s own render omits it.
    fn line(&self, addr: &str, blame_hint: bool) -> String {
        let hint = if blame_hint {
            format!(" — `stele blame {addr}`")
        } else {
            String::new()
        };
        match self {
            Staling::Commit(c) => format!("staling commit: {} {:?}{hint}", c.short, c.subject),
            Staling::HistoryUnavailable => {
                format!("staling commit: history unavailable (shallow clone?){hint}")
            }
            Staling::Uncommitted => {
                format!("staling commit: (uncommitted working-tree change){hint}")
            }
        }
    }

    /// The `--json` `staling_commit` value (§5.3): the commit object, `null` for a local
    /// edit, or a `{status}` marker when history is unavailable.
    fn to_json(&self) -> Value {
        match self {
            Staling::Commit(c) => json!({ "sha": c.short, "subject": c.subject }),
            Staling::HistoryUnavailable => json!({ "status": "history-unavailable" }),
            Staling::Uncommitted => Value::Null,
        }
    }
}

/// The commit that staled a digest-backed claim (§4.5): walk every commit touching the
/// anchored `file` from `verified_sha` forward to `HEAD`, recomputing the bound-def
/// digest at each (`git show <sha>:<file>` → parse → digest), and return the FIRST
/// (oldest) whose digest diverges from `verified_digest`. When no committed version
/// diverges the drift is a working-tree edit — but ONLY when the tree actually differs
/// from HEAD; a `verified_sha` absent from history (shallow clone) yields
/// `HistoryUnavailable`, never a false "uncommitted" attribution (F12).
fn staling_commit(
    root: &Path,
    file: &str,
    anchor: &str,
    verified_sha: &str,
    verified_digest: &str,
) -> Result<Staling> {
    let Some(commits) = commits_touching(root, file, verified_sha)? else {
        return Ok(Staling::HistoryUnavailable);
    };
    // `git rev-list` is newest-first; the staling commit is the OLDEST divergence.
    for sha in commits.iter().rev() {
        // File absent at this commit → the region did not exist → divergent.
        let diverges = git_show(root, sha, file)?.is_none_or(|contents| {
            anchors::region_digest_of_source(anchor, file, &contents)
                .map(|region| region.digest)
                .as_deref()
                != Some(verified_digest)
        });
        if diverges {
            return Ok(Staling::Commit(commit_meta(root, sha)?));
        }
    }
    // No committed version diverges: a genuine uncommitted edit iff the tree differs from
    // HEAD; otherwise history is unavailable (never claim an edit that is not there).
    if working_tree_differs(root, file)? {
        Ok(Staling::Uncommitted)
    } else {
        Ok(Staling::HistoryUnavailable)
    }
}

/// Whether the watermark commit `sha` is present in this repo's object database (§4.5).
/// A shallow / `fetch-depth: 1` clone prunes ancestor commits, so a watermark from before
/// the shallow boundary is absent — freshness must then report "history unavailable"
/// honestly, never silently pass a churn claim (F11) nor misattribute a digest drift to
/// an uncommitted edit (F12).
fn sha_reachable(root: &Path, sha: &str) -> Result<bool> {
    Ok(git(root, "cat-file -e", &[sha])?.status.success())
}

/// Whether `file`'s working-tree bytes differ from HEAD (§4.5), via `git diff --quiet
/// HEAD -- <file>`: exit 1 = differ, exit 0 = identical. Any other status (e.g. unborn
/// HEAD) is treated as "no diff" so the caller does not assert a local edit it cannot
/// prove.
fn working_tree_differs(root: &Path, file: &str) -> Result<bool> {
    let output = git(root, "diff --quiet", &["HEAD", "--", file])?;
    Ok(output.status.code() == Some(1))
}

/// The commits touching `file` in `(verified_sha, HEAD]`, newest first (§4.5); `None`
/// when `verified_sha` is unreachable ([`sha_reachable`]: a shallow clone, or a hand-edited
/// `""` whose `..HEAD` range lists nothing) or not a commit, which the caller reports as
/// `Staling::HistoryUnavailable` — never silently swallowed, never read as "no commits".
fn commits_touching(root: &Path, file: &str, verified_sha: &str) -> Result<Option<Vec<String>>> {
    if !sha_reachable(root, verified_sha)? {
        return Ok(None);
    }
    let range = format!("{verified_sha}..HEAD");
    let output = git(root, "rev-list", &[&range, "--", file])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(output
        .status
        .success()
        .then(|| stdout.split_whitespace().map(str::to_string).collect()))
}

/// The count of commits touching `file` since `verified_sha` (§4.5 churn fallback),
/// via `git rev-list --count`; a failed `rev-list` counts 0. Callers gate on
/// [`sha_reachable`] first, so an unreachable watermark never reads as zero churn.
fn churn_count(root: &Path, file: &str, verified_sha: &str) -> Result<usize> {
    let range = format!("{verified_sha}..HEAD");
    let output = git(root, "rev-list --count", &[&range, "--", file])?;
    let count = String::from_utf8_lossy(&output.stdout).trim().parse().ok();
    Ok(count.filter(|_| output.status.success()).unwrap_or(0))
}

/// The blob of `file` at commit `sha` (`git show <sha>:<file>`), or `None` when the
/// path did not exist at that commit.
fn git_show(root: &Path, sha: &str, file: &str) -> Result<Option<Vec<u8>>> {
    let output = git(root, "show", &[&format!("{sha}:{file}")])?;
    Ok(output.status.success().then_some(output.stdout))
}

/// A commit's short sha + subject line (`git show -s --format=%s`) for the §4.5 render.
fn commit_meta(root: &Path, sha: &str) -> Result<StalingCommit> {
    let output = git(root, "show -s", &["--format=%s", sha])?;
    Ok(StalingCommit {
        short: short_sha(sha).to_string(),
        subject: String::from_utf8_lossy(&output.stdout).trim().to_string(),
    })
}

// ─── `stele blame` (§5.1) ──────────────────────────────────────────────────────

/// `stele blame <node-id>/<slug>` (§5.1, §4.5): resolve the claim (full or abbreviated
/// node-id), read its committed `verified` watermark, and report the staling-commit
/// walk — STALE with the staling commit, up-to-date otherwise, or a parser-less churn
/// count. Returns `(human summary, --json data)`; an unresolved/ambiguous address is a
/// §5.3 input error (exit 2).
pub fn blame(ctx: &Context, address: &str) -> Result<(String, Value)> {
    let claim_ref = match ctx.graph.resolve_claim(address) {
        ClaimLookup::Found(found) => found,
        ClaimLookup::Ambiguous => {
            return Err(SteleError::input_msg(format!(
                "claim address {address:?} is ambiguous (its abbreviation matches multiple nodes)"
            )));
        }
        ClaimLookup::NotFound => {
            return Err(SteleError::input_msg(format!(
                "no claim {address:?} in the graph"
            )));
        }
    };
    let node_id = claim_ref.node_id.to_string();
    let claim = claim_ref.claim;
    let addr = claim_address(ctx.graph, &node_id, &claim.slug);

    let Some(verified) = committed_verified(ctx.committed, &node_id, &claim.slug) else {
        return Ok((
            format!(
                "{addr}: not verified in the committed lock (anchor unresolved at last build) — run stele build"
            ),
            json!({ "claim": addr, "node": node_id, "status": "unverified" }),
        ));
    };
    let Some(resolved) = &claim.resolved else {
        return Ok((
            format!("{addr}: anchor no longer resolves — run stele build"),
            json!({ "claim": addr, "node": node_id, "status": "unresolved" }),
        ));
    };
    let file = resolved_file(resolved);

    match &verified.digest {
        Some(old_digest) => {
            let current = anchors::region_digest_for_claim(ctx.root, &claim.anchor, resolved)?;
            match current {
                Some(region) if &region.digest != old_digest => {
                    let staling =
                        staling_commit(ctx.root, file, &claim.anchor, &verified.sha, old_digest)?;
                    let staling_line = staling.line(&addr, false);
                    let summary = format!(
                        "{addr}: STALE — region {} digest drifted\n  verified at {} (digest {}) → now {}\n  {}",
                        region.name,
                        short_sha(&verified.sha),
                        short_digest(old_digest),
                        short_digest(&region.digest),
                        staling_line,
                    );
                    let data = json!({
                        "claim": addr,
                        "node": node_id,
                        "status": "stale",
                        "region": region.name,
                        "verified_sha": verified.sha,
                        "old_digest": old_digest,
                        "new_digest": region.digest,
                        "staling_commit": staling.to_json(),
                    });
                    Ok((summary, data))
                }
                region => {
                    let name = region.map(|r| r.name);
                    let summary = format!(
                        "{addr}: up to date — verified at {} (digest {}){}",
                        short_sha(&verified.sha),
                        short_digest(old_digest),
                        name.as_ref()
                            .map_or(String::new(), |n| format!(", region {n} unchanged")),
                    );
                    Ok((
                        summary,
                        json!({
                            "claim": addr,
                            "node": node_id,
                            "status": "fresh",
                            "region": name,
                            "verified_sha": verified.sha,
                        }),
                    ))
                }
            }
        }
        None => {
            // A watermark absent from history (shallow clone) cannot anchor a churn count
            // — `git rev-list` would silently return 0 and read as "fresh" (F11). Report
            // the honest failure instead.
            if !sha_reachable(ctx.root, &verified.sha)? {
                return Ok((
                    format!(
                        "{addr}: parser-less anchor — history unavailable (shallow clone?); cannot count churn for {file} since {}",
                        short_sha(&verified.sha),
                    ),
                    json!({
                        "claim": addr,
                        "node": node_id,
                        "status": "history-unavailable",
                        "verified_sha": verified.sha,
                    }),
                ));
            }
            let count = churn_count(ctx.root, file, &verified.sha)?;
            Ok((
                format!(
                    "{addr}: parser-less anchor — {file} touched by {count} commit(s) since {} (file-granularity)",
                    short_sha(&verified.sha),
                ),
                json!({
                    "claim": addr,
                    "node": node_id,
                    "status": "churn",
                    "verified_sha": verified.sha,
                    "count": count,
                }),
            ))
        }
    }
}

// ─── shared freshness render helpers ───────────────────────────────────────────

/// The claim's display address (EXAMPLE 8.4 `billing/refund-cap`): the final-segment
/// abbreviation when that segment names exactly one node across the graph (§2.4), else
/// the full `<node-id>/<slug>`.
fn claim_address(graph: &Graph, node_id: &str, slug: &str) -> String {
    let segment = last_segment(node_id);
    let unique = graph
        .nodes
        .iter()
        .filter(|n| last_segment(&n.id) == segment)
        .count()
        == 1;
    if unique {
        format!("{segment}/{slug}")
    } else {
        format!("{node_id}/{slug}")
    }
}

/// The final path segment of a node id (the system id `/` stays itself).
fn last_segment(id: &str) -> &str {
    if id == SYSTEM_ID {
        return id;
    }
    id.rsplit('/').next().unwrap_or(id)
}

/// The file half of a `file:line` `resolved` string (§4.5).
fn resolved_file(resolved: &str) -> &str {
    resolved.rsplit_once(':').map_or(resolved, |(file, _)| file)
}

/// The short-sha prefix for the freshness render (EXAMPLE 8.4 `e3f19ac`).
fn short_sha(sha: &str) -> &str {
    sha.get(..SHORT_SHA_LEN).unwrap_or(sha)
}

/// A digest's short prefix with a trailing ellipsis (EXAMPLE 8.4 `9c41…`).
fn short_digest(digest: &str) -> String {
    let prefix = digest.get(..SHORT_DIGEST_LEN).unwrap_or(digest);
    format!("{prefix}{DIGEST_ELLIPSIS}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeKind;

    /// P5 (F11/F12): an unreachable-watermark drift must NEVER be attributed to an
    /// uncommitted edit — the honest attribution is "history unavailable (shallow clone?)".
    #[test]
    fn staling_history_unavailable_line_is_honest() {
        let line = Staling::HistoryUnavailable.line("billing/refund-cap", true);
        assert_eq!(
            line,
            "staling commit: history unavailable (shallow clone?) — `stele blame billing/refund-cap`"
        );
        // Without the blame hint (the `stele blame` render), the pointer is dropped.
        assert_eq!(
            Staling::HistoryUnavailable.line("x/y", false),
            "staling commit: history unavailable (shallow clone?)"
        );
        // The lie the fix removes — the uncommitted phrasing — is reserved for a genuine
        // working-tree edit and must not surface for the history-unavailable case.
        assert!(!line.contains("uncommitted"));
    }

    /// The parser-less churn fallback's shallow-clone finding (F11) names the honest
    /// failure and points at the fetch-depth fix, never a silent pass.
    #[test]
    fn history_unavailable_finding_names_shallow_clone() {
        let graph = Graph::default();
        let node = Node {
            kind: NodeKind::System,
            id: "/".to_string(),
            purpose: None,
            commands: std::collections::BTreeMap::new(),
            invariants: Vec::new(),
            hazards: Vec::new(),
            edges: crate::model::Edges::default(),
            budget: None,
            source: std::path::PathBuf::from("AGENTS.md"),
            extracted_imports: Vec::new(),
            contains: Vec::new(),
        };
        let claim = Claim::authored(
            crate::model::ClaimKind::Invariant,
            "the documented rule holds".to_string(),
            "※ doc-rule".to_string(),
            None,
            "doc-rule".to_string(),
        );
        let verified = LockVerified {
            digest: None,
            sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
        };
        let finding = history_unavailable_finding(&graph, &node, &claim, &verified, "notes.txt");
        assert!(
            finding
                .message
                .contains("history unavailable (shallow clone?)")
        );
        assert!(finding.details.iter().any(|d| d.contains("notes.txt")));
        assert!(
            finding
                .fix
                .as_deref()
                .is_some_and(|f| f.contains("fetch-depth: 0"))
        );
    }
}
