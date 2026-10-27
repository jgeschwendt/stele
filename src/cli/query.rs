//! The read/query surface (§5.1, §5.3): every verb here reads the COMMITTED lock and never
//! rebuilds — a missing lock is exit 2 "run stele build", freshness is trusted (staleness is
//! `check`'s job). None writes anything.

use super::{
    CommandOutput, Workspace,
    emit::{HAZARDS_INDEX, INDEX_DIR, INVARIANTS_INDEX},
    load_committed, rebase_on_cwd,
};
use crate::emit;
use crate::lock::{Lock, LockClaim, LockNode};
use crate::model::{Result, SYSTEM_ID, SteleError, dir_contains, normalize_id};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

/// `stele root` (§6): the initialContext as text — the six items in order. Items 1–2
/// (identity, commands) render from the system node here; items 3–6 (hazards, router,
/// indexes, engine) reuse the `emit` region renderer so `root` and the materialized
/// root AGENTS.md never diverge.
pub(super) fn cmd_root(ws: &Workspace, _: &[&str]) -> Result<CommandOutput> {
    let lock = load_committed(&ws.home)?;
    let system = lock.nodes.get(SYSTEM_ID).ok_or_else(|| {
        SteleError::input_msg("the repo root declares no `kind: system` node; nothing to render")
    })?;
    Ok(CommandOutput::new(
        root_json(&lock, system),
        Some(render_root_text(&lock, system)),
    ))
}

/// `stele node <id>` (§5.1): one node, all fields, human-readable. Accepts an
/// abbreviated id when it names exactly one node (§2.4); an unknown id is exit 2
/// listing near-miss candidates.
pub(super) fn node(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let Some(query) = args.get(1) else {
        return Err(SteleError::input_msg("usage: stele node <id>"));
    };
    let lock = load_committed(&ws.home)?;
    let id = resolve_node_id(&lock, query)?;
    let node = &lock.nodes[&id];
    Ok(CommandOutput::new(node_json(node), Some(render_node(node))))
}

/// The default `unfold` radius (§5.1): the node plus its one-hop neighbours.
const DEFAULT_UNFOLD_DEPTH: u32 = 1;

/// `stele unfold <id> [--depth N]` (§5.1): the full node, then its neighbours out to
/// `depth` hops as `id · kind · purpose` summaries. A neighbour is a child (`contains`)
/// or a `depends` target; depth 2 expands each neighbour's neighbours in turn.
pub(super) fn unfold(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    // The id follows a leading `--depth N` (dispatch held argv to that grammar).
    let Some(query) = args.get(if args.get(1) == Some(&"--depth") {
        3
    } else {
        1
    }) else {
        return Err(SteleError::input_msg(
            "usage: stele unfold <id> [--depth N]",
        ));
    };
    let depth = parse_depth(args)?;
    let lock = load_committed(&ws.home)?;
    let id = resolve_node_id(&lock, query)?;
    let node = &lock.nodes[&id];
    let hops = collect_hops(&lock, &id, depth);
    Ok(CommandOutput::new(
        json!({
            "node": node_json(node),
            "depth": depth,
            "hops": hops.iter().map(|(d, nb)| json!({
                "depth": d,
                "id": nb.id,
                "kind": nb.kind,
                "purpose": nb.purpose,
            })).collect::<Vec<_>>(),
        }),
        Some(render_unfold(node, depth, &hops)),
    ))
}

/// `stele invariants [--touching <path>]` (§5.1): every invariant claim repo-wide, or
/// — with `--touching` — the claims of the node owning `<path>` PLUS all its ancestors.
/// Invariant EXPOSURE surfaces upward (a root invariant reaches every descendant), the
/// §4.2 contrast to structural permission, which never inherits.
pub(super) fn invariants(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let lock = load_committed(&ws.home)?;
    let touching = flag_value(args, "--touching")?.map(|path| repo_relative(&ws.work, path));
    let scope = touching.transpose()?.map(|path| owning_chain(&lock, &path));
    let rows = collect_claims(&lock, "invariant", scope.as_deref());
    Ok(CommandOutput::new(
        claims_data(&rows),
        Some(render_claim_rows("invariants", &rows)),
    ))
}

/// `stele hazards [--node <id>]` (§5.1): every active hazard repo-wide, or just the
/// hazards declared by `<id>` (abbreviations accepted). No upward exposure — a hazard
/// is reported by the node that owns it.
pub(super) fn hazards(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let lock = load_committed(&ws.home)?;
    let scope: Option<Vec<String>> = match flag_value(args, "--node")? {
        Some(query) => Some(vec![resolve_node_id(&lock, query)?]),
        None => None,
    };
    let rows = collect_claims(&lock, "hazard", scope.as_deref());
    Ok(CommandOutput::new(
        claims_data(&rows),
        Some(render_claim_rows("hazards", &rows)),
    ))
}

/// `stele nodes [--kind <kind>]` (§5.1): every node as `id · kind · purpose`, optionally
/// filtered to a single kind.
pub(super) fn nodes(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let lock = load_committed(&ws.home)?;
    let kind = flag_value(args, "--kind")?;
    // A typo is a §5.3 bad flag, never an empty listing; the auxiliary kinds (§2.1) are
    // valid and list no authored node.
    let kinds = ["adr", "anchor", "component", "container", "system"];
    if let Some(k) = kind.filter(|k| !kinds.contains(k)) {
        return Err(SteleError::input_msg(format!(
            "unknown --kind {k:?}; expected one of {}",
            kinds.join("|")
        )));
    }
    let rows: Vec<&LockNode> = lock
        .nodes
        .values()
        .filter(|n| kind.is_none_or(|k| n.kind == k))
        .collect();
    let data = json!({
        "nodes": rows.iter().map(|n| json!({
            "id": n.id, "kind": n.kind, "purpose": n.purpose,
        })).collect::<Vec<_>>(),
    });
    let mut text = String::new();
    for n in &rows {
        text.push_str(&format!(
            "{} · {} · {}\n",
            n.id,
            n.kind,
            emit::flat(n.purpose.as_deref().unwrap_or(""))
        ));
    }
    Ok(CommandOutput::new(data, Some(text)))
}

// ─── query rendering + resolution helpers ────────────────────────────────────

/// Resolve a node query to a canonical lock id (§2.4): an exact id (including the
/// system root's `/`·`.`·`` forms) wins; else the query must abbreviate exactly one
/// node by final path segment. Ambiguous or unknown queries are exit-2 errors, the
/// unknown case listing near-miss candidates.
fn resolve_node_id(lock: &Lock, query: &str) -> Result<String> {
    let canonical = normalize_id(query).unwrap_or_else(|_| query.to_string());
    // Lock ids are normalized (§3.2 strict parse) and normalization is idempotent, so a raw
    // query that is itself a lock id always normalizes to that id.
    if lock.nodes.contains_key(&canonical) {
        return Ok(canonical);
    }
    let abbrev: Vec<&String> = lock
        .nodes
        .keys()
        .filter(|id| last_segment(id) == query)
        .collect();
    match abbrev.as_slice() {
        [only] => return Ok((*only).clone()),
        [] => {}
        many => {
            return Err(SteleError::input_msg(format!(
                "node {query:?} is ambiguous — it abbreviates {}",
                many.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    let near: Vec<&str> = lock
        .nodes
        .keys()
        .filter(|id| id.contains(query))
        .map(String::as_str)
        .collect();
    let candidates = if near.is_empty() {
        lock.nodes.keys().map(String::as_str).collect::<Vec<_>>()
    } else {
        near
    };
    Err(SteleError::input_msg(format!(
        "no node {query:?}; candidates: {}",
        candidates.join(", ")
    )))
}

/// The final path segment of a node id (§2.4 abbreviation); the system id `/` is its
/// own segment.
fn last_segment(id: &str) -> &str {
    if id == SYSTEM_ID {
        return id;
    }
    id.rsplit('/').next().unwrap_or(id)
}

/// The value following `name` in `args`, or `None` when the flag is absent. A flag
/// present with no following value is an exit-2 error.
fn flag_value<'a>(args: &'a [&str], name: &str) -> Result<Option<&'a str>> {
    match args.iter().position(|a| *a == name) {
        None => Ok(None),
        Some(i) => args
            .get(i + 1)
            .copied()
            .map(Some)
            .ok_or_else(|| SteleError::input_msg(format!("{name} requires a value"))),
    }
}

/// `path` relative to the work-tree top level `work`, lexically (§3.5): an absolute path is
/// folded, then stripped of its shortest ancestor that canonicalizes to the top level — so a
/// symlinked spelling of the tree (macOS `/var` → `/private/var`) lands inside it while an
/// in-tree symlink scopes by its own location, never its target. A relative path is already
/// folded top-level-relative (`run` rebased it on the cwd). One escaping the tree is exit 2.
fn repo_relative(work: &Path, path: &str) -> Result<String> {
    let relative = if Path::new(path).is_relative() {
        (!Path::new(path).starts_with("..")).then(|| path.to_string())
    } else {
        let (folded, root) = (
            Path::new("/").join(rebase_on_cwd("", path)),
            work.canonicalize().ok(),
        );
        let top = folded
            .ancestors()
            .filter(|a| a.canonicalize().ok() == root)
            .last();
        top.and_then(|top| Some(folded.strip_prefix(top).ok()?.to_string_lossy().into()))
    };
    relative.ok_or_else(|| {
        SteleError::input_msg(format!("--touching {path:?} is outside the work tree"))
    })
}

/// Parse `unfold --depth N` (§5.1): a positive integer, defaulting to
/// [`DEFAULT_UNFOLD_DEPTH`]. A non-integer or zero is an exit-2 error.
fn parse_depth(args: &[&str]) -> Result<u32> {
    let Some(value) = flag_value(args, "--depth")? else {
        return Ok(DEFAULT_UNFOLD_DEPTH);
    };
    match value.parse::<u32>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(SteleError::input_msg(format!(
            "--depth expects a positive integer, got {value:?}"
        ))),
    }
}

/// A node's one-hop neighbours (§5.1 unfold): its child nodes (`contains`) and its
/// `depends` targets, id-sorted and de-duplicated.
fn neighbours(node: &LockNode) -> Vec<String> {
    let mut ids: Vec<String> = node.contains.clone();
    ids.extend(node.declared.depends.clone());
    ids.sort();
    ids.dedup();
    ids
}

/// BFS out to `depth` hops from `start`, returning `(hop-distance, node)` for each
/// node reachable through the neighbour relation, nearest first, each node once.
fn collect_hops<'a>(lock: &'a Lock, start: &str, depth: u32) -> Vec<(u32, &'a LockNode)> {
    let mut seen: BTreeSet<String> = BTreeSet::from([start.to_string()]);
    let mut frontier = vec![&lock.nodes[start]];
    let mut hops = Vec::new();
    for d in 1..=depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for node in frontier {
            for nb in neighbours(node) {
                if seen.insert(nb.clone())
                    && let Some(neighbour) = lock.nodes.get(&nb)
                {
                    hops.push((d, neighbour));
                    next.push(neighbour);
                }
            }
        }
        frontier = next;
    }
    hops
}

/// The human render of `unfold`: the full node, then a hop list indented by distance.
fn render_unfold(node: &LockNode, depth: u32, hops: &[(u32, &LockNode)]) -> String {
    let mut out = render_node(node);
    out.push_str(&format!("\nneighbours (depth {depth}):\n"));
    if hops.is_empty() {
        out.push_str("  (none)\n");
    }
    for (d, nb) in hops {
        out.push_str(&format!(
            "{}{} · {} · {}\n",
            "  ".repeat(*d as usize),
            nb.id,
            nb.kind,
            emit::flat(nb.purpose.as_deref().unwrap_or(""))
        ));
    }
    out
}

/// A claim table row for the transpose queries: the owning node, the claim slug, its
/// prose, and its anchor.
struct ClaimRow {
    anchor: String,
    node: String,
    slug: String,
    text: String,
}

/// Collect claims of one kind (`invariant`/`hazard`) across the lock, optionally
/// restricted to the node ids in `scope`. Ordered by node id then slug.
fn collect_claims(lock: &Lock, kind: &str, scope: Option<&[String]>) -> Vec<ClaimRow> {
    let mut rows = Vec::new();
    for (id, node) in &lock.nodes {
        if scope.is_some_and(|s| !s.iter().any(|n| n == id)) {
            continue;
        }
        for claim in &node.claims {
            if claim.kind == kind {
                rows.push(ClaimRow {
                    anchor: claim.anchor.clone(),
                    node: id.clone(),
                    slug: claim.id.clone(),
                    text: claim.text.clone(),
                });
            }
        }
    }
    rows.sort_by(|a, b| (&a.node, &a.slug).cmp(&(&b.node, &b.slug)));
    rows
}

/// The node ids whose invariants a `--touching <path>` query surfaces (§4.2 upward
/// exposure): the node whose declaring directory (its lock `source`'s parent, never its
/// id) is the deepest to enclose `path`, plus every node enclosing that one (the repo
/// root included). Empty when no node owns the path.
fn owning_chain(lock: &Lock, path: &str) -> Vec<String> {
    let path = normalize_id(path).unwrap_or_else(|_| path.to_string());
    let path = if path == SYSTEM_ID { "" } else { &path };
    let Some(owner) = lock
        .nodes
        .values()
        .filter(|n| dir_contains(n.dir(), path))
        .max_by_key(|n| n.dir().len())
    else {
        return Vec::new();
    };
    lock.nodes
        .values()
        .filter(|n| dir_contains(n.dir(), owner.dir()))
        .map(|n| n.id.clone())
        .collect()
}

/// The `--json` `data` for a claim query: the rows as `{node, claim, text, anchor}`.
fn claims_data(rows: &[ClaimRow]) -> Value {
    json!({
        "claims": rows.iter().map(|r| json!({
            "node": r.node, "claim": r.slug, "text": r.text, "anchor": r.anchor,
        })).collect::<Vec<_>>(),
    })
}

/// The human render of a claim query: one line per row, `node · claim → anchor`.
fn render_claim_rows(title: &str, rows: &[ClaimRow]) -> String {
    let mut out = format!("{title} ({}):\n", rows.len());
    if rows.is_empty() {
        out.push_str("  (none)\n");
    }
    for row in rows {
        out.push_str(&format!(
            "  {} · {} — {} (→ {})\n",
            row.node,
            row.slug,
            emit::flat(&row.text),
            row.anchor
        ));
    }
    out
}

/// `stele root` items 1–6 as text: identity line, command table, then the `emit`
/// region body (hazards banner, router, index pointers, engine lines).
fn render_root_text(lock: &Lock, system: &LockNode) -> String {
    let mut out = String::new();
    out.push_str(&emit::flat(system.purpose.as_deref().unwrap_or("")));
    out.push('\n');
    out.push('\n');
    out.push_str(&render_commands_section(&system.commands));
    // Items 3–6 reuse the shared `emit` renderer (leading `\n`, no trailing newline).
    out.push_str(&emit::render_region(lock, system));
    out.push('\n');
    out
}

/// The §6 item 2 command table for `stele root` (not oracle-pinned): a compact
/// `| command | run |` markdown table, or a `(none)` note when the node declares none.
fn render_commands_section(commands: &std::collections::BTreeMap<String, String>) -> String {
    let mut out = String::from("## Commands\n\n");
    if commands.is_empty() {
        out.push_str("(none)\n");
        return out;
    }
    out.push_str("| command | run |\n| --- | --- |\n");
    for (name, cmd) in commands {
        out.push_str(&format!("| {} | {} |\n", emit::cell(name), emit::cell(cmd)));
    }
    out
}

/// The `--json` `data` for `stele root`: the six items as structured fields.
fn root_json(lock: &Lock, system: &LockNode) -> Value {
    let router: Vec<Value> = neighbours(system)
        .iter()
        .filter_map(|id| lock.nodes.get(id))
        .map(|n| json!({ "id": n.id, "kind": n.kind, "purpose": n.purpose }))
        .collect();
    let hazards = collect_claims(lock, "hazard", None);
    json!({
        "purpose": system.purpose,
        "commands": system.commands,
        "hazards": hazards.iter().map(|h| json!({
            "node": h.node, "claim": h.slug, "text": h.text, "anchor": h.anchor,
        })).collect::<Vec<_>>(),
        "router": router,
        "indexes": [
            format!("{INDEX_DIR}/{INVARIANTS_INDEX}"),
            format!("{INDEX_DIR}/{HAZARDS_INDEX}"),
        ],
    })
}

/// The human render of one node (`stele node`): kind header, then every populated
/// field — purpose, commands, edges, claims, containment, extracted imports, budget.
fn render_node(node: &LockNode) -> String {
    let mut out = format!("{} ({})\n", node.id, node.kind);
    if let Some(purpose) = &node.purpose {
        out.push_str(&format!("purpose: {}\n", emit::flat(purpose)));
    }
    if !node.commands.is_empty() {
        out.push_str("commands:\n");
        for (name, cmd) in &node.commands {
            out.push_str(&format!("  {name}: {cmd}\n"));
        }
    }
    out.push_str(&format!(
        "depends: {}\n",
        join_or_none(&node.declared.depends)
    ));
    if !node.declared.decided_by.is_empty() {
        out.push_str(&format!(
            "decided_by: {}\n",
            node.declared.decided_by.join(", ")
        ));
    }
    for allow in &node.declared.allow {
        out.push_str(&format!(
            "allow: {} — {}\n",
            allow.edge,
            emit::flat(&allow.reason)
        ));
    }
    render_claim_group(&mut out, node, "invariant", "invariants");
    render_claim_group(&mut out, node, "hazard", "hazards");
    if !node.contains.is_empty() {
        out.push_str(&format!("contains: {}\n", node.contains.join(", ")));
    }
    if !node.extracted.imports.is_empty() {
        out.push_str(&format!("imports: {}\n", node.extracted.imports.join(", ")));
    }
    if let Some(budget) = node.budget {
        out.push_str(&format!("budget: {budget}\n"));
    }
    out
}

/// Append a node's claims of one kind to the human render, each as
/// `- [slug] text (anchor → resolved)`.
fn render_claim_group(out: &mut String, node: &LockNode, kind: &str, heading: &str) {
    let group: Vec<&LockClaim> = node.claims.iter().filter(|c| c.kind == kind).collect();
    if group.is_empty() {
        return;
    }
    out.push_str(&format!("{heading}:\n"));
    for claim in group {
        let resolved = claim
            .resolved
            .as_deref()
            .map(|r| format!(" → {r}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  - [{}] {} ({}{})\n",
            claim.id,
            emit::flat(&claim.text),
            claim.anchor,
            resolved
        ));
    }
}

/// The `--json` `data` for `stele node`: every field of the lock node.
fn node_json(node: &LockNode) -> Value {
    json!({
        "id": node.id,
        "kind": node.kind,
        "purpose": node.purpose,
        "budget": node.budget,
        "commands": node.commands,
        "depends": node.declared.depends,
        "decided_by": node.declared.decided_by,
        "allow": node.declared.allow.iter().map(|a| json!({
            "edge": a.edge, "reason": a.reason,
        })).collect::<Vec<_>>(),
        "contains": node.contains,
        "imports": node.extracted.imports,
        "invariants": claims_json(node, "invariant"),
        "hazards": claims_json(node, "hazard"),
    })
}

/// A node's claims of one kind as JSON objects (`stele node` / `unfold` payload).
fn claims_json(node: &LockNode, kind: &str) -> Vec<Value> {
    node.claims
        .iter()
        .filter(|c| c.kind == kind)
        .map(|c| {
            json!({
                "claim": c.id,
                "text": c.text,
                "anchor": c.anchor,
                "enforced_by": c.enforced_by,
                "resolved": c.resolved,
            })
        })
        .collect()
}

/// Join a string list with `, `, or `(none)` when empty.
fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join(", ")
    }
}
