//! The gate verbs (§5.1): `build` writes the lock, `check` byte-compares it and runs the
//! §4 assertions, `blame` reports a claim's watermark (§4.5).

use super::{
    CHECK_USAGE, CommandOutput, FREEZE_FLAG, LOCK_DIR, LOCK_FILE, Mode, REPORT_FLAG,
    RUN_BUILD_HINT, RUN_COMMANDS_FLAG, TREE_DIR, Workspace,
    graph::{build_graph, build_graph_and_warn, stamp_verified, tracked_files},
    init::{ensure_info_exclude, reject_shared_graph_conflict, write_atomic},
    read_committed_lock, require_lock_version,
};
use crate::assert::{self, Context};
use crate::config::{self, AssertionClass};
use crate::emit;
use crate::lock::{self, Lock};
use crate::model::{Graph, Result, SteleError};
use serde_json::json;
use std::path::PathBuf;

/// The one-line stderr notice `build` prints on an unborn HEAD (§7 greenfield: `git
/// init` → `stele init` → `stele build` with no commit yet). Every claim stamps
/// `verified: null` and build still succeeds — there is simply no commit to anchor a
/// watermark to; the first `stele build` after the first commit stamps them.
const UNBORN_HEAD_NOTICE: &str = "stele build: no commit yet (unborn HEAD) — every claim left verified:null; commit, then \
     re-run stele build to stamp watermarks";

/// `stele build` (§5.1): sources → in-memory graph → atomically written lock.
/// Any input error aborts before a single byte is written (§5.3 atomicity).
/// `build` never reads `.stele/config.toml` — config tunes `check`/`emit`, not the
/// graph (§3.4).
pub(super) fn build(ws: &Workspace, _: &[&str]) -> Result<CommandOutput> {
    // Mutual exclusion (§3.5): a tracked shared graph coexisting with the undercover marker is
    // exit 2. Runs BEFORE `build_graph` and the lock write so no partial lock is ever left on
    // disk (§5.3 build atomicity) — an undercover repo that later adopts a shared committed
    // graph fails fast rather than rewriting the private lock. Normal mode never reaches here.
    if ws.mode == Mode::Undercover {
        // Self-heal (§3.5): re-install the `info/exclude` block before anything can fail, so a
        // historically-partial or hand-deleted block is repaired and the lock never surfaces in
        // `git status`. No-ops when the block is already present.
        ensure_info_exclude(&ws.common)?;
    }
    let tracked = tracked_files(&ws.work)?;
    if ws.mode == Mode::Undercover {
        reject_shared_graph_conflict(&ws.work, &tracked)?;
    }
    // Diagnostic only (§2.4/§3.5): flag AGENTS.md nodes the build's node roster will never
    // read. stderr-only — the lock, stdout, and exit code below stay byte-identical. Normal:
    // an untracked stele-block AGENTS.md the tracked scan misses. Undercover: an untracked
    // work-tree stele-block AGENTS.md authored outside the overlay (build reads only
    // `.stele/tree/`, §3.5) — the mode-conflict guard above already covers the tracked case.
    let mut graph = build_graph_and_warn(ws, &tracked)?;
    if !stamp_verified(&ws.work, &mut graph)? {
        // Unborn HEAD: notice goes to stderr so the `--json` stdout envelope stays a
        // single object (§5.3), and build proceeds to write a fully-null-watermark lock.
        eprintln!("{UNBORN_HEAD_NOTICE}");
    }
    let lock = Lock::from_graph(&graph);
    // Any input error has aborted above, so the lock lands whole or not at all (§5.3).
    let dir = ws.home.join(LOCK_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| SteleError::internal(format!("create {LOCK_DIR}: {e}")))?;
    write_atomic(
        &dir.join(LOCK_FILE),
        &lock::to_canonical_string(&lock),
        Some(&ws.home),
    )?;
    let nodes = graph.nodes.len();
    Ok(CommandOutput::new(
        json!({ "lock": format!("{LOCK_DIR}/{LOCK_FILE}"), "nodes": nodes }),
        Some(format!(
            "stele build: wrote {LOCK_DIR}/{LOCK_FILE} ({nodes} node(s))"
        )),
    ))
}

/// `stele check` (§5.1, §5.3): load config (§3.4), require a committed lock, rebuild
/// the graph in-memory, and byte-compare the canonical serialization against the
/// on-disk file. `verified` watermarks are carried over from the committed lock,
/// never re-stamped (§4.5). The six assertion classes run over the rebuilt graph after
/// the compare succeeds; `--run-commands` additionally executes each declared command
/// (§4.6). A clean repo exits 0.
pub(super) fn check(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let only = parse_only(args)?;
    if only.is_none_or(|class| class == AssertionClass::Budget) {
        // A warm-up only: a refused thread leaves the first count to load it.
        let _ = std::thread::Builder::new().spawn(assert::tokenizer);
    }
    let freeze = args.contains(&FREEZE_FLAG);
    if freeze && args.len() > 2 {
        return Err(SteleError::input_msg(format!(
            "--freeze takes no other flag; usage: {CHECK_USAGE} [--json]"
        )));
    }
    let run_commands = args.contains(&RUN_COMMANDS_FLAG);
    let config = config::load(&ws.home)?;

    let on_disk = read_committed_lock(&ws.home)?;

    require_lock_version(&on_disk)?;
    let committed = lock::parse_lock(&on_disk)?;

    let tracked = tracked_files(&ws.work)?;
    // Mutual exclusion (§3.5): a tracked shared graph coexisting with the undercover marker is
    // exit 2, surfaced BEFORE the assertion suite runs. Normal mode never reaches here.
    if ws.mode == Mode::Undercover {
        // Self-heal (§3.5): re-ensure the `info/exclude` block (no-op when unchanged) so a lost
        // block is repaired on the next `check`, keeping the work tree clean.
        ensure_info_exclude(&ws.common)?;
        reject_shared_graph_conflict(&ws.work, &tracked)?;
    }
    // Diagnostic only (§2.4/§3.5): flag AGENTS.md nodes the build's node roster will never
    // read — an omitted node leaves the committed lock in-spec, so the byte-compare below would
    // pass while silently short a node. stderr-only; findings/exit are untouched. Normal: an
    // untracked stele-block AGENTS.md the tracked scan misses. Undercover: an untracked
    // work-tree stele-block AGENTS.md authored outside the overlay (§3.5); the guard above
    // covers the tracked case.
    let graph = build_graph_and_warn(ws, &tracked)?;
    let mut rebuilt = Lock::from_graph(&graph);
    rebuilt.carry_over_verified(&committed);

    if lock::to_canonical_string(&rebuilt) != on_disk {
        return Err(SteleError::input_msg(format!(
            "committed lock does not match the freshly-built graph; {RUN_BUILD_HINT}"
        )));
    }

    // The six assertion classes (§4) run over the rebuilt graph AFTER the byte-compare
    // (§5.3); any finding maps to exit 1, none to exit 0. `root` is the WORK tree — the
    // freshness/blame/liveness checks all spawn git or shells in the invoking checkout
    // (§4.5 HEAD is per-work-tree).
    let source_root = node_source_root(ws);
    let ctx = Context {
        committed: &committed,
        config: &config,
        graph: &graph,
        home: &ws.home,
        root: &ws.work,
        run_commands,
        source_root: &source_root,
        tracked: &tracked,
    };

    // `--freeze` baselines the current structural violations and exits 0 (§4.2),
    // instead of running the assertion suite.
    if freeze {
        let count = assert::write_freeze(&ctx)?;
        return Ok(CommandOutput::new(
            json!({ "frozen": count }),
            Some(format!(
                "stele check --freeze: baselined {count} structural violation(s)"
            )),
        ));
    }

    let findings = assert::run(&ctx, only)?;
    let report_requested = args.contains(&REPORT_FLAG);
    let allow = collect_allow_entries(&graph);
    let data = if report_requested {
        json!({
            "nodes": graph.nodes.len(),
            "allow": allow
                .iter()
                .map(|(node, edge, reason)| json!({ "node": node, "edge": edge, "reason": reason }))
                .collect::<Vec<_>>(),
        })
    } else {
        json!({ "nodes": graph.nodes.len() })
    };
    Ok(CommandOutput {
        data,
        summary: None,
        findings,
        report: report_requested.then(|| render_allow_report(&allow)),
    })
}

/// Every `allow` entry across the graph (§4.2), as `(node id, edge target, reason)`,
/// ordered by node id then declared order — the `check --report` payload.
fn collect_allow_entries(graph: &Graph) -> Vec<(String, String, String)> {
    let mut entries = Vec::new();
    for node in &graph.nodes {
        for allow in &node.edges.allow {
            entries.push((node.id.clone(), allow.edge.clone(), allow.reason.clone()));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// The `check --report` human section (§4.2): one line per `allow` entry, reason
/// verbatim. A leading blank line separates it from any findings above; an empty set
/// still prints the header so the governance surface is always visible.
fn render_allow_report(entries: &[(String, String, String)]) -> String {
    let mut out = format!("\nallow entries ({}):\n", entries.len());
    if entries.is_empty() {
        out.push_str("  (none)\n");
    }
    for (node, edge, reason) in entries {
        out.push_str(&format!("  {node} · {edge} · {}\n", emit::flat(reason)));
    }
    out
}

/// `stele blame <node-id>/<slug>` (§5.1, §4.5): require a committed lock, rebuild the
/// graph in-memory to resolve the claim's current anchor, and walk history to the
/// staling commit — reporting STALE (with the staling commit), up-to-date, or a
/// parser-less churn count. Like `emit`/`node` it trusts the committed lock and does
/// NOT byte-compare (staleness detection is `check`'s job, §5.3); it reads `verified`
/// from that lock, the sole watermark carrier. Accepts an abbreviated node-id when
/// unambiguous (§2.4).
pub(super) fn blame(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let Some(address) = args.get(1) else {
        return Err(SteleError::input_msg("usage: stele blame <node-id>/<slug>"));
    };
    let config = config::load(&ws.home)?;
    let on_disk = read_committed_lock(&ws.home)?;
    require_lock_version(&on_disk)?;
    let committed = lock::parse_lock(&on_disk)?;
    let tracked = tracked_files(&ws.work)?;
    let graph = build_graph(ws, &tracked)?;
    let source_root = node_source_root(ws);
    let ctx = Context {
        committed: &committed,
        config: &config,
        graph: &graph,
        home: &ws.home,
        root: &ws.work,
        run_commands: false,
        source_root: &source_root,
        tracked: &tracked,
    };
    let (summary, data) = assert::blame(&ctx, address)?;
    Ok(CommandOutput::new(data, Some(summary)))
}

/// Parse `check --only <class>` (§5.1): the value must name one of the six classes
/// (§4), else it is an exit-2 bad-flag error (§5.3). Absent `--only` runs every
/// enabled class.
fn parse_only(args: &[&str]) -> Result<Option<AssertionClass>> {
    let Some(index) = args.iter().position(|a| *a == "--only") else {
        return Ok(None);
    };
    match args.get(index + 1) {
        Some(name) => AssertionClass::parse(name).map(Some).ok_or_else(|| {
            SteleError::input_msg(format!(
                "unknown --only class {name:?}; expected one of \
                 referential|structural|exhaustiveness|budget|freshness|liveness"
            ))
        }),
        None => Err(SteleError::input_msg(
            "--only requires a class argument \
             (referential|structural|exhaustiveness|budget|freshness|liveness)",
        )),
    }
}

/// The directory the budget class (§4.4) resolves each node's `source` path against for
/// its file reads (§3.5), mode-aware. Normal: the work tree (`ws.work == "."`, so a
/// node's relative `source` joins byte-identically to today). Undercover: the overlay
/// `<home>/.stele/tree`, where node sources live off the work tree. Distinct from
/// `ctx.root` (always the work tree): only node-source reads move to the overlay; every
/// code scan and freshness/liveness spawn stays in the invoking checkout.
fn node_source_root(ws: &Workspace) -> PathBuf {
    match ws.mode {
        Mode::Normal => ws.work.clone(),
        Mode::Undercover => ws.home.join(TREE_DIR),
    }
}
