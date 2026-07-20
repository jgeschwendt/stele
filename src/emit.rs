//! Materialization of AGENTS.md projections (SPEC §3.1/§6).
//!
//! Phase D4 needs two projections for the budget class (§4.4); Phase E's `emit` will
//! reuse them verbatim. `materialized_content` is a node's rendered AGENTS.md — for
//! now the file's entire on-disk bytes (authored `stele` block + generated regions +
//! free prose, the §4.4 "counted content"). `chain` is the root→node sequence of
//! nodes whose concatenated files form the `codex` root→leaf / `claude` always-loaded
//! set (§4.4).

use crate::lock::{Lock, LockNode};
use crate::model::{Node, Result, SYSTEM_ID, SteleError};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ─── generated-region rendering (§3.1 items 2–3, §6, §6.1) ───────────────────
//
// `emit` reads the committed lock (never rebuilds, §5.1) and renders each node's
// marker-fenced region in place. The acme fixture is the byte-identity oracle:
// `render_region` reproduces `tests/fixtures/acme/AGENTS.md`'s region (root) and
// `apps/web/lib/billing/AGENTS.md`'s (interior) exactly.
//
// Region shape by node kind — the rule the fixture pins (a container renders EMPTY
// even when it owns landmarks, e.g. `apps/worker`/`packages/shared`; only a
// `component` renders its anchor table):
//   • system    → root contract (§6 items 3–6): Hazards, Map, Indexes, Engine.
//   • component → `## Anchors in this territory` (landmarks it owns) + a child
//                 router where it has child nodes (unexercised by acme — no acme
//                 component has children — so the fixture cannot pin its bytes).
//   • container → empty region (routing is carried by the parent's Map).
//
// The `claim`/`invariant` discriminator, the glyphs (⚠ · →), and the two Engine
// lines are the §6 contract; the fixture bytes are their oracle.

/// The hazard-banner bullet glyph (§6 item 3), a fixture-pinned byte.
const HAZARD_GLYPH: &str = "⚠";
/// §6 item 5 index-pointer line, verbatim from the root fixture.
const INDEXES_LINE: &str =
    "All invariants: `.stele/index/invariants.md` · all hazards: `.stele/index/hazards.md`";
/// §6 item 6 engine instructions, both lines verbatim from the root fixture.
const ENGINE_LINE_ENGINE: &str = "`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards | nodes --kind <k>`. MCP: `stele serve`.";
const ENGINE_LINE_NO_ENGINE: &str = "No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).";

/// The lock `kind` strings this renderer branches on (§3.2 wire form).
const KIND_COMPONENT: &str = "component";
const KIND_SYSTEM: &str = "system";
/// The lock `claims[].kind` discriminators (§3.2).
const CLAIM_HAZARD: &str = "hazard";
const CLAIM_INVARIANT: &str = "invariant";

/// The engine-owned bytes strictly between a node's region markers (§3.1 item 2).
/// Deterministic: the same lock always yields byte-identical output — the property
/// `emit --check`'s byte-diff relies on. Returns `""` for kinds that render nothing.
pub fn render_region(lock: &Lock, node: &LockNode) -> String {
    match node.kind.as_str() {
        KIND_SYSTEM => render_root(lock, node),
        KIND_COMPONENT => render_component(lock, node),
        _ => String::new(),
    }
}

/// The root contract region (§6 items 3–6): Hazards banner, Map router, Indexes
/// pointers, Engine instructions — in that order. Leading blank line, sections
/// separated by a blank line, NO trailing blank (the root fixture's exact shape).
fn render_root(lock: &Lock, root: &LockNode) -> String {
    let hazards = collect_hazards(lock);
    let hazard_body: Vec<String> = hazards
        .iter()
        .map(|h| format!("- {HAZARD_GLYPH} `{}`: {} (→ {})", h.node, h.text, h.anchor))
        .collect();
    let hazards_block = section(&format!("Hazards ({} active)", hazards.len()), &hazard_body);

    let map_block = section("Map", &router_table(&first_hop_rows(lock, root)));
    let indexes_block = section("Indexes", &[INDEXES_LINE.to_string()]);
    let engine_block = section(
        "Engine",
        &[
            ENGINE_LINE_ENGINE.to_string(),
            ENGINE_LINE_NO_ENGINE.to_string(),
        ],
    );

    let blocks = [hazards_block, map_block, indexes_block, engine_block];
    format!("\n{}", blocks.join("\n"))
}

/// An interior component region (§3.1 item 2): the anchor table for landmarks it
/// owns, plus a child router where it has child nodes. Leading + trailing blank
/// line wrapping the block(s); empty string when it has neither (the `store`
/// fixture's empty region).
fn render_component(lock: &Lock, node: &LockNode) -> String {
    let mut blocks: Vec<String> = Vec::new();

    // Landmarks in this node's territory (§4.2 owner == this node), slug-sorted —
    // `lock.landmarks` is a BTreeMap, so iteration is already slug order.
    let anchor_body: Vec<String> = lock
        .landmarks
        .iter()
        .filter(|(_, lm)| lm.node == node.id)
        .map(|(slug, lm)| {
            format!(
                "- lm:{slug} → {}:{}",
                relativize(&lm.file, &node.id),
                lm.line
            )
        })
        .collect();
    if !anchor_body.is_empty() {
        blocks.push(section("Anchors in this territory", &anchor_body));
    }

    // A child router where the component nests child nodes. No acme component has
    // children, so the fixture does not pin these bytes; the Map table format is
    // reused for consistency with the root router.
    if !node.contains.is_empty() {
        blocks.push(section("Router", &router_table(&child_rows(lock, node))));
    }

    if blocks.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", blocks.join("\n"))
    }
}

/// `.stele/index/invariants.md` (§6.1): the repo-wide invariant table. Design is not
/// oracle-constrained; kept a compact markdown table, byte-stable, ordered by node
/// id then claim slug.
pub fn render_invariants_index(lock: &Lock) -> String {
    render_claims_index(lock, "Invariants", CLAIM_INVARIANT)
}

/// `.stele/index/hazards.md` (§6.1): the repo-wide hazard table, same shape as
/// [`render_invariants_index`].
pub fn render_hazards_index(lock: &Lock) -> String {
    render_claims_index(lock, "Hazards", CLAIM_HAZARD)
}

/// The first line of every `emit --claude-rules` output (§3.3): an HTML comment naming
/// stele and forbidding hand edits. It is the overwrite gate — `emit` regenerates a
/// `.claude/rules/*.md` file only when it is absent or already opens with this exact
/// line; a file without it is hand-authored and refused (never clobbered). HTML-comment
/// so it renders invisibly in the materialized markdown.
pub const CLAUDE_RULE_MARKER: &str =
    "<!-- stele:generated — do not edit; regenerated by `stele emit --claude-rules` -->";

/// A path-scoped `.claude/rules/<slug>.md` for one node's claims (§3.3, opt-in via
/// `emit --claude-rules`). Opens with [`CLAUDE_RULE_MARKER`] (the overwrite gate), then
/// frontmatter naming the node dir; claims are listed in the lock's slug order. Not
/// oracle-constrained.
pub fn render_claude_rule(id: &str, node: &LockNode) -> String {
    let mut out = format!("{CLAUDE_RULE_MARKER}\n---\nnode: {id}\n---\n\n# {id} claims\n\n");
    for claim in &node.claims {
        out.push_str(&format!(
            "- [{}] {} (`{}`)\n",
            claim.kind, claim.text, claim.anchor
        ));
    }
    out
}

/// The `.claude/rules/` filename stem for a node id (§3.3): the repo root maps to
/// `root`, every other id has its `/` separators flattened to `-`.
pub fn rule_slug(id: &str) -> String {
    if id == SYSTEM_ID {
        "root".to_string()
    } else {
        id.replace('/', "-")
    }
}

// ─── rendering helpers ───────────────────────────────────────────────────────

/// One `## Heading` section: the heading, a blank line, then each body line — no
/// leading or trailing blank line (the caller joins sections with a blank line).
fn section(heading: &str, body: &[String]) -> String {
    let mut out = format!("## {heading}\n\n");
    for line in body {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A hazard banner entry (§6 item 3): the declaring node, the claim prose, its
/// anchor, and the slug the banner sorts on.
struct HazardRow {
    anchor: String,
    node: String,
    slug: String,
    text: String,
}

/// Every active hazard across the graph (§6 item 3), sorted by claim slug — the
/// order the root fixture pins (`dunning-batch` before `webhook-verify`; node-id
/// order would flip them).
fn collect_hazards(lock: &Lock) -> Vec<HazardRow> {
    let mut hazards: Vec<HazardRow> = Vec::new();
    for (id, node) in &lock.nodes {
        for claim in &node.claims {
            if claim.kind == CLAIM_HAZARD {
                hazards.push(HazardRow {
                    anchor: claim.anchor.clone(),
                    node: id.clone(),
                    slug: claim.id.clone(),
                    text: claim.text.clone(),
                });
            }
        }
    }
    hazards.sort_by(|a, b| a.slug.cmp(&b.slug));
    hazards
}

/// The root Map's first-hop rows (§6 item 4): the root's child nodes plus its
/// depended nodes, id-sorted and de-duplicated, each as `(id, kind, purpose)`.
fn first_hop_rows(lock: &Lock, root: &LockNode) -> Vec<(String, String, String)> {
    let mut ids: Vec<String> = root.contains.clone();
    ids.extend(root.declared.depends.clone());
    ids.sort();
    ids.dedup();
    ids.iter().filter_map(|id| node_row(lock, id)).collect()
}

/// A component's child-router rows: its `contains[]` child nodes as `(id, kind,
/// purpose)`, in the lock's (sorted) `contains` order.
fn child_rows(lock: &Lock, node: &LockNode) -> Vec<(String, String, String)> {
    node.contains
        .iter()
        .filter_map(|id| node_row(lock, id))
        .collect()
}

/// `(id, kind, purpose)` for a node id present in the lock, else `None`.
fn node_row(lock: &Lock, id: &str) -> Option<(String, String, String)> {
    lock.nodes.get(id).map(|n| {
        (
            id.to_string(),
            n.kind.clone(),
            n.purpose.clone().unwrap_or_default(),
        )
    })
}

/// The number of columns in a router table (node · kind · purpose · unfold).
const ROUTER_COLUMNS: usize = 4;
/// The router table's fixed header cells (§6 item 4).
const ROUTER_HEADERS: [&str; ROUTER_COLUMNS] = ["node", "kind", "purpose", "unfold"];

/// A router table (§6 item 4) as markdown rows: header, separator, then one row per
/// node. Every column is padded to its widest cell (the fixture's scheme). The
/// `unfold` cell is the `stele unfold`/read-AGENTS.md pointer for the node id.
fn router_table(rows: &[(String, String, String)]) -> Vec<String> {
    let mut cells: Vec<[String; ROUTER_COLUMNS]> = vec![ROUTER_HEADERS.map(str::to_string)];
    for (id, kind, purpose) in rows {
        cells.push([
            id.clone(),
            kind.clone(),
            purpose.clone(),
            format!("`stele unfold {id}` · or read `{id}/AGENTS.md`"),
        ]);
    }

    let mut widths = [0usize; ROUTER_COLUMNS];
    for row in &cells {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let mut out = Vec::with_capacity(cells.len() + 1);
    out.push(render_row(&cells[0], &widths));
    let separator: [String; ROUTER_COLUMNS] = std::array::from_fn(|i| "-".repeat(widths[i]));
    out.push(render_row(&separator, &widths));
    for row in &cells[1..] {
        out.push(render_row(row, &widths));
    }
    out
}

/// One markdown table row, each cell left-padded with spaces to its column width:
/// `| c0 | c1 | … |`.
fn render_row(cells: &[String; ROUTER_COLUMNS], widths: &[usize; ROUTER_COLUMNS]) -> String {
    let mut out = String::from("|");
    for (cell, width) in cells.iter().zip(widths) {
        out.push(' ');
        out.push_str(cell);
        for _ in 0..width - cell.chars().count() {
            out.push(' ');
        }
        out.push_str(" |");
    }
    out
}

/// A repo-relative file path re-rooted at the node's directory (§3.1 interior
/// anchors show `charge.ex`, not the full path): strip the `<node-id>/` prefix.
fn relativize(file: &str, node_id: &str) -> String {
    file.strip_prefix(&format!("{node_id}/"))
        .unwrap_or(file)
        .to_string()
}

/// The transpose claim index body (§6.1) for one claim kind: a compact
/// `claim · node · anchor` table ordered by node id then slug, one trailing LF.
fn render_claims_index(lock: &Lock, title: &str, kind: &str) -> String {
    let mut rows: Vec<(String, String, String, String)> = Vec::new();
    for (id, node) in &lock.nodes {
        for claim in &node.claims {
            if claim.kind == kind {
                rows.push((
                    id.clone(),
                    claim.id.clone(),
                    claim.text.clone(),
                    claim.anchor.clone(),
                ));
            }
        }
    }
    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

    let mut out = format!("# {title}\n\n| claim | node | anchor |\n| --- | --- | --- |\n");
    for (node, _slug, text, anchor) in &rows {
        out.push_str(&format!("| {text} | {node} | {anchor} |\n"));
    }
    out
}

/// A node's materialized AGENTS.md (§4.4 counted content): the file's entire on-disk
/// bytes. `check` already read every AGENTS.md to build the graph, but re-reading here
/// keeps this helper self-contained and reusable by Phase E `emit`.
pub fn materialized_content(root: &Path, node: &Node) -> Result<String> {
    std::fs::read_to_string(root.join(&node.source))
        .map_err(|e| SteleError::internal(format!("read {}: {e}", node.source.display())))
}

/// The degradation-file name every AGENTS.md on a directory path carries (SPEC §3.1).
const DOC_NAME: &str = "AGENTS.md";

/// The root→`node` chain of VCS-tracked AGENTS.md files (§4.4): every degradation file on
/// the DIRECTORY path from the repo root down to `node`'s own directory, in root→leaf
/// order — node-backed OR plain (SPEC §3.1). A `codex`/`claude` harness loads every
/// AGENTS.md on the path it walks, not only the ones that declare a `stele` node, so the
/// budget chain must count them all; walking node ids alone would drop a plain AGENTS.md
/// sitting between two nodes and let its bytes escape the truncation check (§4.4).
pub fn chain_files(tracked: &[PathBuf], node: &Node) -> Vec<String> {
    let tracked: BTreeSet<String> = tracked
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    let node_dir = node
        .source
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    let mut files = Vec::new();
    let mut prefix = String::new();
    push_doc_if_tracked(&tracked, &prefix, &mut files);
    for segment in node_dir.split('/').filter(|s| !s.is_empty()) {
        prefix = if prefix.is_empty() {
            segment.to_string()
        } else {
            format!("{prefix}/{segment}")
        };
        push_doc_if_tracked(&tracked, &prefix, &mut files);
    }
    files
}

/// Push `<dir>/AGENTS.md` (`AGENTS.md` at the root) onto `files` when it is VCS-tracked.
fn push_doc_if_tracked(tracked: &BTreeSet<String>, dir: &str, files: &mut Vec<String>) {
    let candidate = if dir.is_empty() {
        DOC_NAME.to_string()
    } else {
        format!("{dir}/{DOC_NAME}")
    };
    if tracked.contains(&candidate) {
        files.push(candidate);
    }
}

/// The concatenated content of a [`chain_files`] chain (§4.4): each AGENTS.md's on-disk
/// bytes in root→leaf order, the exact bytes a `codex`/`claude` harness sees for that leaf.
pub fn chain_content(root: &Path, chain: &[String]) -> Result<String> {
    let mut out = String::new();
    for rel in chain {
        out.push_str(
            &std::fs::read_to_string(root.join(rel))
                .map_err(|e| SteleError::internal(format!("read {rel}: {e}")))?,
        );
    }
    Ok(out)
}
