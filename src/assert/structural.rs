//! Structural (§4.2): extracted cross-node imports against declared `depends`, both
//! directions, minus the `.stele/freeze.json` baseline `check --freeze` writes.

use super::{Context, Finding};
use crate::config::AssertionClass;
use crate::model::{Node, Result, SteleError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::Path;

/// The committed structural baseline, relative to the repo root (§4.2 `--freeze`).
const FREEZE_PATH: &str = ".stele/freeze.json";
/// The freeze-file format version this engine reads and writes.
const FREEZE_VERSION: u32 = 1;

/// The two structural directions (§4.2). The string form is the `direction` key in
/// `.stele/freeze.json` and the discriminator in a [`FreezeKey`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    /// A declared `depends` edge with no extracted import backing it (reverse).
    Vestigial,
    /// An extracted cross-node import with no covering `depends` (forward).
    Violation,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vestigial => "vestigial",
            Self::Violation => "violation",
        }
    }
}

/// One structural violation's identity (§4.2 freeze key `{node, direction, target}`) —
/// also a `.stele/freeze.json` entry. Field order is the canonical baseline sort (node,
/// then direction, then target); the file's key order is the JSON map's (alphabetical).
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct FreezeKey {
    node: String,
    direction: String,
    target: String,
}

/// A current structural violation: its stable [`FreezeKey`] (for baselining) plus the
/// [`Finding`] it renders to.
struct StructuralViolation {
    key: FreezeKey,
    finding: Finding,
}

/// The structural class (§4.2): every current violation in both directions, minus the
/// ones baselined in `.stele/freeze.json` (a NEW violation still surfaces; a baselined
/// one that no longer occurs is simply unused).
pub(super) fn structural(ctx: &Context) -> Result<Vec<Finding>> {
    let baseline = read_freeze_baseline(ctx.home)?;
    Ok(structural_violations(ctx)
        .into_iter()
        .filter(|violation| !baseline.contains(&violation.key))
        .map(|violation| violation.finding)
        .collect())
}

/// Every CURRENT structural violation (§4.2), both directions, before any freeze
/// filtering — the set `--freeze` baselines and `check` filters against. A node with
/// empty `depends` is airtight (any cross-node import violates); an `allow` entry
/// suppresses BOTH directions for its edge.
fn structural_violations(ctx: &Context) -> Vec<StructuralViolation> {
    let mut violations = Vec::new();
    for node in &ctx.graph.nodes {
        let allowed: BTreeSet<&str> = node.edges.allow.iter().map(|a| a.edge.as_str()).collect();
        let declared: BTreeSet<&str> = node.edges.depends.iter().map(String::as_str).collect();

        // Forward (violation): an extracted cross-node import covered by neither a
        // declared depends edge nor an allow entry.
        for target in &node.extracted_imports {
            if declared.contains(target.as_str()) || allowed.contains(target.as_str()) {
                continue;
            }
            violations.push(forward_violation(ctx, node, target));
        }

        // Reverse (vestigial): a declared depends edge with no extracted import backing
        // it and no allow entry.
        for target in &node.edges.depends {
            if node.extracted_imports.iter().any(|import| import == target)
                || allowed.contains(target.as_str())
            {
                continue;
            }
            violations.push(vestigial_violation(node, target));
        }
    }
    violations
}

/// A forward violation (EXAMPLE 8.1): `<node> imports <target> — edge not declared`,
/// located at every contributing reference occurrence, with the node's declared
/// depends and the remove/declare/allow fix.
fn forward_violation(ctx: &Context, node: &Node, target: &str) -> StructuralViolation {
    let mut finding = Finding::error(
        AssertionClass::Structural,
        Some(node.id.clone()),
        format!("{} imports {target} — edge not declared", node.id),
    );
    for occ in ctx
        .graph
        .import_edges
        .get(&(node.id.clone(), target.to_string()))
        .into_iter()
        .flatten()
    {
        finding = finding.location(occ.file.clone(), occ.line, Some(occ.text.clone()));
    }
    finding = finding
        .detail(format!(
            "declared depends of {}: [{}]",
            node.id,
            node.edges.depends.join(", ")
        ))
        .fix(format!(
            "remove the import, or declare it in {} (and mean it), or \
             allow: {{edge: {target}, reason: \"...\"}} for dynamic/DI cases",
            node.source.display()
        ));
    StructuralViolation {
        key: freeze_key(node, Direction::Violation, target),
        finding,
    }
}

/// A vestigial violation (EXAMPLE 8.2): `<node> declares depends on <target> — no
/// import found`, with the doc-lied detail and the remove/allow fix.
fn vestigial_violation(node: &Node, target: &str) -> StructuralViolation {
    let finding = Finding::error(
        AssertionClass::Structural,
        Some(node.id.clone()),
        format!("{} declares depends on {target} — no import found", node.id),
    )
    .detail(
        "the signature promises a dependency the code no longer has \
         (doc lied, or dependency died)",
    )
    .fix(format!(
        "remove the edge from {}, or allow: with reason if the dep is runtime-dynamic",
        node.source.display()
    ));
    StructuralViolation {
        key: freeze_key(node, Direction::Vestigial, target),
        finding,
    }
}

fn freeze_key(node: &Node, direction: Direction, target: &str) -> FreezeKey {
    FreezeKey {
        node: node.id.clone(),
        direction: direction.as_str().to_string(),
        target: target.to_string(),
    }
}

// ─── the freeze baseline (§4.2 `--freeze`) ─────────────────────────────────────

/// `.stele/freeze.json` as read: an array of `{direction, node, target}` entries.
#[derive(Deserialize)]
struct FreezeFile {
    #[serde(default)]
    violations: Vec<FreezeKey>,
}

/// Read the committed baseline (§4.2). An absent file is an empty baseline; malformed
/// content is an input error (exit 2); a genuine IO failure is internal (exit 3).
fn read_freeze_baseline(home: &Path) -> Result<BTreeSet<FreezeKey>> {
    let text = match std::fs::read_to_string(home.join(FREEZE_PATH)) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(SteleError::internal(format!("read {FREEZE_PATH}: {e}"))),
    };
    let parsed: FreezeFile = serde_json::from_str(&text)
        .map_err(|e| SteleError::input_msg(format!("malformed {FREEZE_PATH}: {e}")))?;
    Ok(parsed.violations.into_iter().collect())
}

/// `stele check --freeze` (§4.2): baseline every CURRENT structural violation (both
/// directions) into `.stele/freeze.json`, canonically sorted with a trailing LF, and
/// return the count. Every later `check` suppresses exactly these; new violations
/// still fail.
pub fn write_freeze(ctx: &Context) -> Result<usize> {
    let keys: BTreeSet<FreezeKey> = structural_violations(ctx)
        .into_iter()
        .map(|violation| violation.key)
        .collect();
    // The canonical bytes (§4.2): 2-space pretty-print, keys in Unicode-scalar order (the
    // `Value` map is a BTreeMap), one trailing LF — a byte-stable, committed-friendly file.
    // `.stele/` exists: `check` has just read the lock from it.
    let canonical = json!({ "version": FREEZE_VERSION, "violations": keys });
    std::fs::write(ctx.home.join(FREEZE_PATH), format!("{canonical:#}\n"))
        .map_err(|e| SteleError::internal(format!("write {FREEZE_PATH}: {e}")))?;
    Ok(keys.len())
}
