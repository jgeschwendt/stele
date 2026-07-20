//! Materialization of AGENTS.md projections (SPEC §3.1/§6).
//!
//! Phase D4 needs two projections for the budget class (§4.4); Phase E's `emit` will
//! reuse them verbatim. `materialized_content` is a node's rendered AGENTS.md — for
//! now the file's entire on-disk bytes (authored `stele` block + generated regions +
//! free prose, the §4.4 "counted content"). `chain` is the root→node sequence of
//! nodes whose concatenated files form the `codex` root→leaf / `claude` always-loaded
//! set (§4.4).

use crate::model::{Node, Result, SYSTEM_ID, SteleError};
use std::path::Path;

/// A node's materialized AGENTS.md (§4.4 counted content): the file's entire on-disk
/// bytes. `check` already read every AGENTS.md to build the graph, but re-reading here
/// keeps this helper self-contained and reusable by Phase E `emit`.
pub fn materialized_content(root: &Path, node: &Node) -> Result<String> {
    std::fs::read_to_string(root.join(&node.source))
        .map_err(|e| SteleError::internal(format!("read {}: {e}", node.source.display())))
}

/// The root→`node` chain of nodes (§4.4): every node whose id is an id-ancestor of
/// `node` (the system root `/` always included), ordered root-first, with `node`
/// itself last. This is the `codex` concatenation order and the `claude` always-loaded
/// set (node's file preceded by every ancestor's file).
pub fn chain<'a>(nodes: &'a [Node], node: &'a Node) -> Vec<&'a Node> {
    let mut chain: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.id != node.id && is_id_ancestor(&n.id, &node.id))
        .collect();
    // Ancestors of one node are totally ordered by the prefix relation, and id length
    // increases monotonically along that chain (the root `/` is the shortest), so
    // sorting by id length yields exact root→node order.
    chain.sort_by_key(|n| n.id.len());
    chain.push(node);
    chain
}

/// The concatenated materialized content of a `chain` (§4.4): each node's file bytes
/// in root→node order, the exact bytes a `codex`/`claude` harness sees for that leaf.
pub fn chain_content(root: &Path, chain: &[&Node]) -> Result<String> {
    let mut out = String::new();
    for node in chain {
        out.push_str(&materialized_content(root, node)?);
    }
    Ok(out)
}

/// Whether node id `ancestor` contains node id `descendant` by path nesting (§4.2).
/// The system id `/` is an ancestor of every other node; otherwise `descendant` must
/// begin with `ancestor` followed by a `/` (so `apps/web` contains `apps/web/x` but
/// not `apps/website`).
fn is_id_ancestor(ancestor: &str, descendant: &str) -> bool {
    if ancestor == SYSTEM_ID {
        return true;
    }
    descendant
        .strip_prefix(ancestor)
        .is_some_and(|rest| rest.starts_with('/'))
}
