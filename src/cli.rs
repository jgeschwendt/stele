//! Command-line surface and subcommand dispatch (SPEC §5.1, §5.3).
//!
//! `build` is the sole writer of `.stele/graph.lock`: it walks the repo, parses
//! every VCS-tracked AGENTS.md into an in-memory graph, and atomically writes the
//! canonical lock (§5.1 pipeline). `check` requires a committed lock, rebuilds
//! in-memory, and byte-compares the canonical serialization against the on-disk
//! file (§5.3); the six assertion classes land in Phase D, after that compare.
//! `emit` reads the committed lock and renders each AGENTS.md generated region in
//! place plus the transpose indexes (§3.1/§6.1); `node` is still a lock-gated stub.
//! `build_graph` resolves comment anchors and the ADR index (Phase C1) and derives
//! import edges (Phase C2), so `build` stamps `resolved` and `verified {sha}` and
//! fills `landmarks{}`/`adrs{}`/`extracted.imports`; the freshness `digest` (C3) is
//! the last compiled slot still `null`.

use crate::anchors::{self, SymbolResolution};
use crate::assert::{self, Context, Finding};
use crate::config::{self, AssertionClass};
use crate::emit;
use crate::extract;
use crate::lock::{self, Lock, LockClaim, LockNode};
use crate::model::{
    AdrEntry, ExitCode, Graph, Node, NodeKind, PURPOSE_MAX_CHARS, Resolution, Result, SYSTEM_ID,
    SteleError, Verified, normalize_id,
};
use crate::parse;
use crate::steleignore::Steleignore;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The committed lock, relative to the repo root.
const LOCK_DIR: &str = ".stele";
const LOCK_FILE: &str = "graph.lock";
/// Temp-name prefix a fresh lock is written to before the atomic rename (§5.3 build
/// atomicity: never a partial lock on disk). A per-writer `.{pid}.{seq}` suffix is
/// appended so concurrent builds never collide on one temp file.
const LOCK_TEMP: &str = ".graph.lock.tmp";

/// The message every read command emits when the committed lock is missing or
/// stale (§5.3); tests assert on the `run stele build` substring.
const RUN_BUILD_HINT: &str = "run stele build";

/// The machine-output flag (§5.3): exactly one JSON envelope on stdout.
const JSON_FLAG: &str = "--json";

/// The `check --freeze` flag (§4.2): baseline current structural violations.
const FREEZE_FLAG: &str = "--freeze";

/// The `check --run-commands` flag (§4.6): execute each declared command, not only
/// resolve it (off by default — the bonfires tier).
const RUN_COMMANDS_FLAG: &str = "--run-commands";

/// The `check --report` flag (§4.2): after the findings (or on a clean repo), print
/// every `allow` entry with its node, edge, and verbatim reason — the sole governance
/// against `# noqa`-style accumulation (reason-plus-visibility).
const REPORT_FLAG: &str = "--report";

/// The stable machine contract (§5.3 `--json` envelope). `findings` is reserved for
/// assertion results (Phase D populates it); input/internal errors surface via
/// `ok:false` + `exit` + `data.error`, never as findings.
#[derive(Serialize)]
struct Envelope<'a> {
    stele: &'static str,
    command: &'a str,
    ok: bool,
    exit: i32,
    data: Value,
    findings: Vec<Value>,
}

/// A command's result: the `--json` `data` payload, an optional human-readable line
/// (printed only when `--json` is absent), any assertion findings (§4), and an
/// optional trailing human-readable report block (`check --report`, §4.2). Findings
/// are `check`-only today; a non-empty list makes the process exit 1 (§5.3). The
/// `report` block prints (non-`--json`) after the summary/findings regardless of
/// outcome; under `--json` its content is folded into `data` instead.
struct CommandOutput {
    data: Value,
    summary: Option<String>,
    findings: Vec<Finding>,
    report: Option<String>,
}

impl CommandOutput {
    /// A data+summary success with no findings (the shape build/emit/node/clean-check
    /// return).
    fn new(data: Value, summary: Option<String>) -> Self {
        Self {
            data,
            summary,
            findings: Vec::new(),
            report: None,
        }
    }
}

// ─── workspace resolution (§3.5) ─────────────────────────────────────────────
//
// Every verb runs against a resolved [`Workspace`], not a bare root path: `work` is the
// invoking work tree (all git spawns and source/code scanning run here) and `home` is
// the graph home where the engine's artifacts live — the lock, `.stele/index/`, node
// sources (§3.2/§3.5). In normal mode the two are the SAME verbatim path (today's
// `Path::new(".")`), so every join is byte-identical and no §2–§7 behavior moves.
// Undercover mode (§3.5) drives them apart: `home` becomes the parent of the git common
// dir, outside the tracked tree. Phase 2 lands the split with [`Mode::Normal`] the only
// reachable mode — nothing writes a marker yet, so [`resolve_workspace`] reaches
// [`Mode::Undercover`] only against a marker that already exists on disk, and no code
// branches on the mode (Phase 3+ gates behavior on it).

/// The two graph modes (§3.5): a committed graph at the repo root, or a private on-disk
/// graph rooted at the parent of the git common dir. Carried on every [`Workspace`] but
/// not yet branched on.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Undercover,
}

/// The resolved graph roots for one invocation (§3.5): `work` is the invoking work tree
/// (git spawns, anchor/import scanning, freshness), `home` is where the engine's
/// artifacts live (the lock, the transpose indexes, node sources). In [`Mode::Normal`]
/// `home` and `work` are the identical verbatim path, so threading them apart is a strict
/// no-op; only undercover mode separates them.
pub struct Workspace {
    pub home: PathBuf,
    pub mode: Mode,
    pub work: PathBuf,
}

/// The private-graph marker (§3.5), relative to the graph home: its presence at
/// `<home>/.stele/undercover` selects undercover mode. A deliberately-uncommitted source
/// artifact (contrast `.stele/config.toml`, §3.4) — nothing in Phase 2 writes it, so
/// [`resolve_workspace`] can reach [`Mode::Undercover`] only against a pre-existing marker.
const UNDERCOVER_MARKER: &str = ".stele/undercover";

/// The overlay node-source root (§3.5), relative to the graph home: undercover node
/// sources live under `<home>/.stele/tree/<dirpath>/AGENTS.md`, mirroring tree paths.
/// Declared with the split; the overlay walk that consumes it lands in Phase 3.
#[allow(dead_code)] // consumed in Phase 3 (the undercover overlay walk); declared now so the split lands whole.
const TREE_DIR: &str = ".stele/tree";

/// The git common directory of the work tree at `work` (`git rev-parse --git-common-dir`),
/// canonicalized — `<repo>/.git` in a normal checkout, the shared bare `.git` under a
/// linked-worktree layout. Relative git output is joined onto `work` before canonicalizing.
/// Returns `None` on ANY failure (a non-repo cwd, a git that will not spawn, an
/// uncanonicalizable path) so [`resolve_workspace`] degrades to normal mode rather than
/// erroring.
fn git_common_dir(work: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(work)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(trimmed);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        work.join(path)
    };
    absolute.canonicalize().ok()
}

/// Resolve the [`Workspace`] for `work` once at dispatch entry (§3.5). The graph home is
/// the canonicalized parent of the git common dir; when `<home>/.stele/undercover` exists
/// the result is [`Mode::Undercover`] rooted there. On ANY failure to locate the home
/// (non-repo cwd, no parent) OR an absent marker the result is [`Mode::Normal`] with
/// `home == work == work` verbatim — the exact value threaded before the split, so every
/// downstream join stays byte-identical. NEVER errors: a non-repo cwd falls through to the
/// verbs' own exit-2 paths (`git ls-files` reports "not a git repository").
pub fn resolve_workspace(work: &Path) -> Workspace {
    let normal = || Workspace {
        home: work.to_path_buf(),
        mode: Mode::Normal,
        work: work.to_path_buf(),
    };
    let Some(common) = git_common_dir(work) else {
        return normal();
    };
    let Some(home) = common.parent() else {
        return normal();
    };
    if home.join(UNDERCOVER_MARKER).exists() {
        Workspace {
            home: home.to_path_buf(),
            mode: Mode::Undercover,
            work: work.to_path_buf(),
        }
    } else {
        normal()
    }
}

/// Run the CLI, returning the §5.3 process exit code (`0` success, else the error's
/// exit class). With `--json`, exactly one envelope prints to stdout regardless of
/// outcome; otherwise a success prints its human line to stdout and an error prints
/// `file:line: message` to stderr.
pub fn run(args: &[String]) -> i32 {
    // `--version`/`-V` short-circuits every other verb: print `stele <version>` (the
    // engine's own version, from CARGO_PKG_VERSION) to stdout and exit 0. It is what
    // scripts/install.sh calls to confirm the landed binary runs, so it must never
    // require a lock or a git repo.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("stele {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    // `serve` (§5.2) owns stdin/stdout for MCP JSON-RPC framing, so it is intercepted
    // BEFORE the `--json` envelope machinery — its stdout must carry only protocol
    // messages, never a CLI envelope.
    if args.first().map(String::as_str) == Some("serve") {
        return crate::serve::run(Path::new("."), args);
    }
    let json = args.iter().any(|a| a == JSON_FLAG);
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != JSON_FLAG)
        .collect();
    // Resolve the home/work split once (§3.5), after the serve/--version short-circuits;
    // in normal mode this is `home == work == Path::new(".")` verbatim (byte-identical).
    let workspace = resolve_workspace(Path::new("."));
    let command = rest.first().copied().unwrap_or("");

    match dispatch(&workspace, &rest) {
        Ok(output) => {
            // Assertion findings (§4) make `check` exit 1 ("repo out of spec", §5.3);
            // an empty list keeps the 0-success path.
            let exit = if output.findings.is_empty() { 0 } else { 1 };
            if json {
                let findings = output.findings.iter().map(Finding::to_json).collect();
                print_envelope(command, exit == 0, exit, output.data, findings);
            } else {
                if output.findings.is_empty() {
                    if let Some(summary) = &output.summary {
                        println!("{summary}");
                    }
                } else {
                    print!("{}", assert::render_human(&output.findings));
                }
                // The `check --report` block trails the summary/findings on the human
                // path (§4.2 reason-plus-visibility); under `--json` it rides in `data`.
                if let Some(report) = &output.report {
                    print!("{report}");
                }
            }
            exit
        }
        Err(error) => {
            let exit = error.exit as i32;
            if json {
                print_envelope(command, false, exit, error_data(&error), Vec::new());
            } else {
                eprintln!("{error}");
            }
            exit
        }
    }
}

fn dispatch(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    match args.first().copied() {
        Some("blame") => blame(ws, args),
        Some("build") => build(ws, args),
        Some("check") => check(ws, args),
        Some("emit") => emit(ws, args),
        Some("hazards") => hazards(ws, args),
        Some("init") => init(ws, args),
        Some("invariants") => invariants(ws, args),
        Some("node") => node(ws, args),
        Some("nodes") => nodes(ws, args),
        Some("root") => cmd_root(ws),
        Some("unfold") => unfold(ws, args),
        Some(other) => Err(SteleError::input_msg(format!("unknown command: {other:?}"))),
        None => Err(SteleError::input_msg(
            "usage: stele root | node <id> | unfold <id> | invariants | hazards | nodes | \
             check | emit | blame | build | init | serve | --version   (add --json for the \
             machine envelope; serve speaks MCP on stdio and takes no --json)",
        )),
    }
}

/// One read/query verb rendered for the MCP tier (§5.2): `text` is byte-identical to the
/// CLI human-path stdout of `stele <verb>`, and `exit` is the §5.3 process class. serve
/// builds a verb argv from a tool's named parameters, calls [`serve_render`], and packages
/// this as an MCP tool result.
pub struct VerbRender {
    pub exit: i32,
    pub text: String,
}

/// Dispatch one verb and render it exactly as [`run`]'s human path would print to stdout —
/// the single code path the CLI and `serve` share (§5.2: same engine, tools mirror the
/// verbs, no duplicated rendering). A success prints its summary line (findings render
/// instead when present) plus any `--report` block; an input/internal error becomes
/// `text = <error message>` with the error's exit class. serve maps exit ≥ 2 to an
/// `isError` tool result (§5.2).
pub fn serve_render(ws: &Workspace, argv: &[&str]) -> VerbRender {
    match dispatch(ws, argv) {
        Ok(output) => {
            let exit = if output.findings.is_empty() { 0 } else { 1 };
            let mut text = String::new();
            if output.findings.is_empty() {
                if let Some(summary) = &output.summary {
                    // `run` prints the summary with `println!`, appending one newline.
                    text.push_str(summary);
                    text.push('\n');
                }
            } else {
                text.push_str(&assert::render_human(&output.findings));
            }
            if let Some(report) = &output.report {
                text.push_str(report);
            }
            VerbRender { exit, text }
        }
        Err(error) => VerbRender {
            exit: error.exit as i32,
            text: format!("{error}"),
        },
    }
}

/// Confirm `root` is inside a git work tree (§2.4 scan scope) — the precondition
/// `serve` checks at startup before entering its MCP loop (a non-repo cwd surfaces as
/// the same exit-2 input error the CLI verbs give). Reuses the tracked-file probe.
pub fn ensure_git_repo(root: &Path) -> Result<()> {
    git_tracked_files(root)?;
    Ok(())
}

/// Print the one §5.3 JSON envelope. `findings` carries the assertion results (§4);
/// input/internal errors pass an empty list and surface via `data.error` instead.
fn print_envelope(command: &str, ok: bool, exit: i32, data: Value, findings: Vec<Value>) {
    let envelope = Envelope {
        stele: env!("CARGO_PKG_VERSION"),
        command,
        ok,
        exit,
        data,
        findings,
    };
    // Serialization of this fixed-shape struct cannot fail.
    println!("{}", serde_json::to_string(&envelope).unwrap());
}

/// The §5.3 `data.error` payload for an input/internal error: message plus, where
/// known, the offending `file:line`. Distinct from `findings` (assertion results).
fn error_data(error: &SteleError) -> Value {
    json!({
        "error": {
            "file": error.file.as_ref().map(|p| p.display().to_string()),
            "line": error.line,
            "message": error.message,
        }
    })
}

/// The one-line stderr notice `build` prints on an unborn HEAD (§7 greenfield: `git
/// init` → `stele init` → `stele build` with no commit yet). Every claim stamps
/// `verified: null` and build still succeeds — there is simply no commit to anchor a
/// watermark to; the first `stele build` after the first commit stamps them.
const UNBORN_HEAD_NOTICE: &str = "stele build: no commit yet (unborn HEAD) — every claim left verified:null; commit, then \
     re-run stele build to stamp watermarks";

/// The mutating verbs (§5.3 `stele build | init`) accept ONLY the global `--json` flag
/// (already stripped from `args` before dispatch) and no other token. `args[0]` is the
/// verb; any further argument is a §5.3 input error (exit 2) surfaced BEFORE any
/// filesystem effect — an unknown flag must never silently run the mutation (e.g. a
/// mistyped `stele init --help` overwriting a live tree).
fn reject_extra_args(args: &[&str], usage: &str) -> Result<()> {
    if let Some(extra) = args.get(1) {
        return Err(SteleError::input_msg(format!(
            "unexpected argument {extra:?}; usage: {usage}"
        )));
    }
    Ok(())
}

/// `stele build` (§5.1): sources → in-memory graph → atomically written lock.
/// Any input error aborts before a single byte is written (§5.3 atomicity).
/// `build` never reads `.stele/config.toml` — config tunes `check`/`emit`, not the
/// graph (§3.4).
fn build(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    reject_extra_args(args, "stele build [--json]")?;
    let mut graph = build_graph(&ws.work)?;
    // Diagnostic only (§2.4): flag untracked AGENTS.md nodes the tracked-file scan cannot
    // see. stderr-only — the lock, stdout, and exit code below stay byte-identical.
    warn_untracked_node_files(&ws.work)?;
    if !stamp_verified(&ws.work, &mut graph)? {
        // Unborn HEAD: notice goes to stderr so the `--json` stdout envelope stays a
        // single object (§5.3), and build proceeds to write a fully-null-watermark lock.
        eprintln!("{UNBORN_HEAD_NOTICE}");
    }
    let lock = Lock::from_graph(&graph);
    let bytes = lock::to_canonical_string(&lock);
    write_lock_atomic(&ws.home, &bytes)?;
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
fn check(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let only = parse_only(args)?;
    let freeze = args.contains(&FREEZE_FLAG);
    let run_commands = args.contains(&RUN_COMMANDS_FLAG);
    let config = config::load(&ws.home)?;

    let on_disk = read_committed_lock(&ws.home)?;

    let version = lock::read_version(&on_disk)?;
    if version != lock::LOCK_VERSION {
        return Err(SteleError::input_msg(format!(
            "committed lock is version {version}; this engine writes version {}. {RUN_BUILD_HINT}",
            lock::LOCK_VERSION
        )));
    }
    let committed = lock::parse_lock(&on_disk)?;

    let tracked = tracked_files(&ws.work)?;
    let graph = build_graph(&ws.work)?;
    // Diagnostic only (§2.4): flag untracked AGENTS.md nodes the tracked-file scan cannot
    // see — an untracked node leaves the committed lock in-spec, so the byte-compare below
    // would pass while silently omitting it. stderr-only; findings/exit are untouched.
    warn_untracked_node_files(&ws.work)?;
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
    let ctx = Context {
        committed: &committed,
        config: &config,
        graph: &graph,
        root: &ws.work,
        run_commands,
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
        out.push_str(&format!("  {node} · {edge} · {reason}\n"));
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
fn blame(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let Some(address) = args.get(1) else {
        return Err(SteleError::input_msg("usage: stele blame <node-id>/<slug>"));
    };
    let config = config::load(&ws.home)?;
    let on_disk = read_committed_lock(&ws.home)?;
    let version = lock::read_version(&on_disk)?;
    if version != lock::LOCK_VERSION {
        return Err(SteleError::input_msg(format!(
            "committed lock is version {version}; this engine writes version {}. {RUN_BUILD_HINT}",
            lock::LOCK_VERSION
        )));
    }
    let committed = lock::parse_lock(&on_disk)?;
    let tracked = tracked_files(&ws.work)?;
    let graph = build_graph(&ws.work)?;
    let ctx = Context {
        committed: &committed,
        config: &config,
        graph: &graph,
        root: &ws.work,
        run_commands: false,
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

/// The `emit --check` flag (§3.1): render to memory and diff against on-disk regions
/// and index files, failing CI on divergence instead of rewriting.
const EMIT_CHECK_FLAG: &str = "--check";
/// The `emit --claude-rules` flag (§3.3): additionally render path-scoped
/// `.claude/rules/*.md` from node claims (opt-in).
const EMIT_CLAUDE_RULES_FLAG: &str = "--claude-rules";
/// The transpose-index directory (§6.1), relative to the repo root.
const INDEX_DIR: &str = ".stele/index";
const INVARIANTS_INDEX: &str = "invariants.md";
const HAZARDS_INDEX: &str = "hazards.md";
/// The `.claude/rules/` directory (§3.3), relative to the repo root.
const CLAUDE_RULES_DIR: &str = ".claude/rules";
/// The CLAUDE.md shim (§3.3): one line pointing Claude Code at AGENTS.md.
const CLAUDE_SHIM: &str = "CLAUDE.md";
const CLAUDE_SHIM_CONTENT: &str = "@AGENTS.md\n";

/// `stele emit` (§5.1): read the COMMITTED lock (never rebuild, §3.2/§5.1) and render
/// every node's generated region in place — only between its markers — plus the
/// transpose indexes (§6.1) and the CLAUDE.md shim (§3.3). `--check` diffs instead of
/// writing (divergence → exit 1, §3.1); `--claude-rules` additionally renders
/// path-scoped `.claude/rules/*.md` (§3.3, opt-in). Like `node`/`blame` it trusts the
/// committed lock and does not byte-compare — staleness is `check`'s job (§5.3).
fn emit(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let _config = config::load(&ws.home)?;
    let on_disk = read_committed_lock(&ws.home)?;
    let version = lock::read_version(&on_disk)?;
    if version != lock::LOCK_VERSION {
        return Err(SteleError::input_msg(format!(
            "committed lock is version {version}; this engine writes version {}. {RUN_BUILD_HINT}",
            lock::LOCK_VERSION
        )));
    }
    let lock = lock::parse_lock(&on_disk)?;

    // All emit targets live at the graph home (§3.5): node regions, the transpose indexes,
    // the shim, and any `.claude/rules/*.md` all render under `home`.
    if args.contains(&EMIT_CHECK_FLAG) {
        return emit_check(&ws.home, &lock);
    }
    emit_write(&ws.home, &lock, args.contains(&EMIT_CLAUDE_RULES_FLAG))
}

/// Render every region in place, write both transpose indexes, ensure the CLAUDE.md
/// shim, and (when `claude_rules`) render `.claude/rules/*.md`. `emit` rewrites strictly
/// between markers and under `.stele/`/`.claude/` — never the marker lines or authored
/// prose (§5.1).
fn emit_write(root: &Path, lock: &Lock, claude_rules: bool) -> Result<CommandOutput> {
    for (id, node) in &lock.nodes {
        let path = node_agents_path(id);
        let contents = read_node_agents(root, &path)?;
        let region = require_region(&path, &contents)?;
        let rendered = emit::render_region(lock, node);
        let updated = format!(
            "{}{}{}",
            &contents[..region.content_start],
            rendered,
            &contents[region.content_end..]
        );
        if updated != contents {
            std::fs::write(root.join(&path), &updated)
                .map_err(|e| SteleError::internal(format!("write {}: {e}", path.display())))?;
        }
    }

    write_under_root(
        root,
        INDEX_DIR,
        INVARIANTS_INDEX,
        &emit::render_invariants_index(lock),
    )?;
    write_under_root(
        root,
        INDEX_DIR,
        HAZARDS_INDEX,
        &emit::render_hazards_index(lock),
    )?;
    ensure_claude_shim(root, lock)?;
    if claude_rules {
        write_claude_rules(root, lock)?;
    }

    let regions = lock.nodes.len();
    Ok(CommandOutput::new(
        json!({ "regions": regions, "indexes": 2 }),
        Some(format!(
            "stele emit: rendered {regions} region(s) and 2 index file(s)"
        )),
    ))
}

/// Render to memory and byte-diff against on-disk regions and index files (§3.1); any
/// divergence is an assertion failure (exit 1) naming each divergent file. Missing
/// regions and malformed markers remain input errors (exit 2), same as `emit`.
fn emit_check(root: &Path, lock: &Lock) -> Result<CommandOutput> {
    let mut divergent: Vec<String> = Vec::new();
    for (id, node) in &lock.nodes {
        let path = node_agents_path(id);
        let contents = read_node_agents(root, &path)?;
        let region = require_region(&path, &contents)?;
        if contents[region.content_start..region.content_end] != *emit::render_region(lock, node) {
            divergent.push(path.display().to_string());
        }
    }
    check_index(
        root,
        INVARIANTS_INDEX,
        &emit::render_invariants_index(lock),
        &mut divergent,
    );
    check_index(
        root,
        HAZARDS_INDEX,
        &emit::render_hazards_index(lock),
        &mut divergent,
    );

    if divergent.is_empty() {
        return Ok(CommandOutput::new(
            json!({ "checked": lock.nodes.len() }),
            Some("stele emit --check: every generated region and index is up to date".to_string()),
        ));
    }
    Err(SteleError {
        exit: ExitCode::Assertion,
        file: None,
        line: None,
        message: format!(
            "emit --check: {} generated file(s) diverge from the graph; run stele emit:\n{}",
            divergent.len(),
            divergent.join("\n")
        ),
    })
}

/// The AGENTS.md path for a node id (§2.1 default id ⇔ declaring directory): the repo
/// root's node lives at `AGENTS.md`, every other at `<id>/AGENTS.md`.
fn node_agents_path(id: &str) -> PathBuf {
    if id == crate::model::SYSTEM_ID {
        PathBuf::from("AGENTS.md")
    } else {
        PathBuf::from(format!("{id}/AGENTS.md"))
    }
}

/// Read a node's AGENTS.md (an IO failure is exit 3).
fn read_node_agents(root: &Path, path: &Path) -> Result<String> {
    std::fs::read_to_string(root.join(path))
        .map_err(|e| SteleError::internal(format!("read {}: {e}", path.display())))
}

/// Locate a node's generated region, mapping its absence to the §3.1 exit-2 error that
/// points the user at `stele init` (`emit` never scaffolds the region itself).
fn require_region(path: &Path, contents: &str) -> Result<parse::Region> {
    parse::find_region(path, contents)?.ok_or_else(|| {
        SteleError::input(
            path,
            1,
            "node AGENTS.md has no generated region; run stele init to scaffold it (§3.1)",
        )
    })
}

/// Write a file under a repo-root-relative directory, creating the directory tree.
fn write_under_root(root: &Path, dir: &str, name: &str, content: &str) -> Result<()> {
    let dir_path = root.join(dir);
    std::fs::create_dir_all(&dir_path)
        .map_err(|e| SteleError::internal(format!("create {dir}: {e}")))?;
    std::fs::write(dir_path.join(name), content)
        .map_err(|e| SteleError::internal(format!("write {dir}/{name}: {e}")))
}

/// Ensure `CLAUDE.md` exists as the `@AGENTS.md` shim when the repo root declares a
/// node (§3.3). A CLAUDE.md that already exists is the team's and is never overwritten,
/// even when it differs.
fn ensure_claude_shim(root: &Path, lock: &Lock) -> Result<()> {
    if !lock.nodes.contains_key(crate::model::SYSTEM_ID) {
        return Ok(());
    }
    let path = root.join(CLAUDE_SHIM);
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, CLAUDE_SHIM_CONTENT)
        .map_err(|e| SteleError::internal(format!("write {CLAUDE_SHIM}: {e}")))
}

/// Render one `.claude/rules/<slug>.md` per node that declares claims (§3.3, opt-in).
/// Each write is gated on the stele marker: a target that is absent or already opens with
/// [`emit::CLAUDE_RULE_MARKER`] is (re)generated; a foreign file (a hand-authored rule
/// with no marker) is NEVER clobbered — it is an exit-2 input error naming the file.
fn write_claude_rules(root: &Path, lock: &Lock) -> Result<()> {
    for (id, node) in &lock.nodes {
        if node.claims.is_empty() {
            continue;
        }
        write_claude_rule_file(
            root,
            &format!("{}.md", emit::rule_slug(id)),
            &emit::render_claude_rule(id, node),
        )?;
    }
    Ok(())
}

/// Write one generated `.claude/rules/<name>` (§3.3), guarding a hand-authored file. When
/// the target exists and does NOT open with [`emit::CLAUDE_RULE_MARKER`] it is a foreign
/// file — refuse with an exit-2 input error naming it (never overwrite a team's rule); an
/// absent or marker-carrying target is (re)generated.
fn write_claude_rule_file(root: &Path, name: &str, content: &str) -> Result<()> {
    let path = root.join(CLAUDE_RULES_DIR).join(name);
    if let Ok(existing) = std::fs::read_to_string(&path)
        && !existing.starts_with(emit::CLAUDE_RULE_MARKER)
    {
        return Err(SteleError::input(
            format!("{CLAUDE_RULES_DIR}/{name}"),
            1,
            "refusing to overwrite a hand-authored .claude/rules file (no stele marker line); \
             move it aside or delete it, then re-run stele emit --claude-rules",
        ));
    }
    write_under_root(root, CLAUDE_RULES_DIR, name, content)
}

/// Byte-diff one on-disk index file against its freshly-rendered content; a missing or
/// divergent file is recorded for the `emit --check` failure list.
fn check_index(root: &Path, name: &str, expected: &str, divergent: &mut Vec<String>) {
    let path = root.join(INDEX_DIR).join(name);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(expected) {
        divergent.push(format!("{INDEX_DIR}/{name}"));
    }
}

// ─── the read/query surface (§5.1, §5.3) ─────────────────────────────────────
//
// Every verb below reads the COMMITTED lock and never rebuilds (§5.3): a missing
// lock is exit 2 "run stele build", they trust the lock's freshness (staleness is
// `check`'s job). None writes anything.

/// Load + version-check + strict-parse the committed lock (§5.3), the shared front
/// door for the read/query verbs. A missing lock → exit 2 "run stele build"; an
/// unknown `version` → exit 2 (never best-effort parsed, §3.2).
fn load_committed(root: &Path) -> Result<Lock> {
    let on_disk = read_committed_lock(root)?;
    let version = lock::read_version(&on_disk)?;
    if version != lock::LOCK_VERSION {
        return Err(SteleError::input_msg(format!(
            "committed lock is version {version}; this engine writes version {}. {RUN_BUILD_HINT}",
            lock::LOCK_VERSION
        )));
    }
    lock::parse_lock(&on_disk)
}

/// `stele root` (§6): the initialContext as text — the six items in order. Items 1–2
/// (identity, commands) render from the system node here; items 3–6 (hazards, router,
/// indexes, engine) reuse the `emit` region renderer so `root` and the materialized
/// root AGENTS.md never diverge.
fn cmd_root(ws: &Workspace) -> Result<CommandOutput> {
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
fn node(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
fn unfold(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let Some(query) = args.get(1) else {
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
fn invariants(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let lock = load_committed(&ws.home)?;
    let touching = flag_value(args, "--touching")?;
    let scope: Option<Vec<String>> = touching.map(|path| owning_chain(&lock, path));
    let rows = collect_claims(&lock, "invariant", scope.as_deref());
    Ok(CommandOutput::new(
        claims_data(&rows),
        Some(render_claim_rows("invariants", &rows)),
    ))
}

/// `stele hazards [--node <id>]` (§5.1): every active hazard repo-wide, or just the
/// hazards declared by `<id>` (abbreviations accepted). No upward exposure — a hazard
/// is reported by the node that owns it.
fn hazards(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
fn nodes(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let lock = load_committed(&ws.home)?;
    let kind = flag_value(args, "--kind")?;
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
            n.purpose.as_deref().unwrap_or("")
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
    if lock.nodes.contains_key(&canonical) {
        return Ok(canonical);
    }
    if lock.nodes.contains_key(query) {
        return Ok(query.to_string());
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
    let mut frontier = vec![start.to_string()];
    let mut hops = Vec::new();
    for d in 1..=depth {
        let mut next = Vec::new();
        for id in &frontier {
            let Some(node) = lock.nodes.get(id) else {
                continue;
            };
            for nb in neighbours(node) {
                if seen.insert(nb.clone())
                    && let Some(neighbour) = lock.nodes.get(&nb)
                {
                    hops.push((d, neighbour));
                    next.push(nb);
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
            nb.purpose.as_deref().unwrap_or("")
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
/// exposure): the node owning `path` plus every ancestor node (the system root `/`
/// included). Empty when no node owns the path.
fn owning_chain(lock: &Lock, path: &str) -> Vec<String> {
    let path = normalize_id(path).unwrap_or_else(|_| path.to_string());
    let Some(owner) = lock
        .nodes
        .keys()
        .filter(|id| id_contains(id, &path))
        .max_by_key(|id| id_depth(id))
    else {
        return Vec::new();
    };
    lock.nodes
        .keys()
        .filter(|id| id_contains(id, owner))
        .cloned()
        .collect()
}

/// Whether node id `container` covers `path` by territory nesting: the system id `/`
/// covers everything; otherwise `path` equals or is nested under `container`.
fn id_contains(container: &str, path: &str) -> bool {
    if container == SYSTEM_ID {
        return true;
    }
    path == container || path.starts_with(&format!("{container}/"))
}

/// A node id's nesting depth (the system root is shallowest at 0), for deepest-owner
/// selection.
fn id_depth(id: &str) -> usize {
    if id == SYSTEM_ID {
        0
    } else {
        id.split('/').count()
    }
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
            row.node, row.slug, row.text, row.anchor
        ));
    }
    out
}

/// `stele root` items 1–6 as text: identity line, command table, then the `emit`
/// region body (hazards banner, router, index pointers, engine lines).
fn render_root_text(lock: &Lock, system: &LockNode) -> String {
    let mut out = String::new();
    out.push_str(system.purpose.as_deref().unwrap_or(""));
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
        out.push_str(&format!("| {name} | {cmd} |\n"));
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
        out.push_str(&format!("purpose: {purpose}\n"));
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
        out.push_str(&format!("allow: {} — {}\n", allow.edge, allow.reason));
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
            claim.id, claim.text, claim.anchor, resolved
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

// ─── init: the adoption scaffold (§7) ────────────────────────────────────────

/// The empty generated region `init` writes (§3.1/§7): the router markers with no body
/// between them, for `emit` to fill later.
const EMPTY_REGION: &str = "<!-- stele:begin router -->\n<!-- stele:end -->\n";

/// `stele init` (§7): scan the tracked tree, propose node boundaries, and scaffold a
/// skeleton `stele` block + empty generated region for each proposed node that has no
/// stele block yet. Proposals: the repo root (system) plus each top-level directory
/// that holds tracked files, is not the ADR directory, and is not already covered by an
/// existing node (component/import-cluster proposals are out of scope for v1). A file
/// that already carries a stele block is left byte-identical (the §7 idempotency
/// promise); a prose-only file gets the skeleton prepended and its prose preserved.
/// A proposed target that a `.gitignore` matches is skipped with a stderr notice (never
/// scaffolded into an untrackable path). All writes happen first; then the whole set is
/// staged in ONE `git add` batch so the next `stele build` (tracked-files scan, §2.4)
/// sees them — a staging failure is a warning plus a `git add` instruction, never a
/// post-write abort. `init` never deletes content, never writes inside a generated
/// region, and never writes the lock.
fn init(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    reject_extra_args(args, "stele init [--json]")?;
    // Normal-mode init scaffolds and stages the WORK tree in place (§7); the git spawns,
    // the tracked reads, and the source writes all target the invoking checkout. (Phase 3
    // forks `init --undercover` to scaffold the `<home>/.stele/tree/` overlay instead, with
    // no `git add`.)
    let work = &ws.work;
    let tracked = tracked_files(work)?;
    let adr_dir = detect_adr_dir(&tracked);
    let existing = existing_node_dirs(work, &tracked)?;

    let mut written: Vec<String> = Vec::new();
    if !existing.contains("") && !skip_if_ignored(work, "AGENTS.md")? {
        scaffold_node(work, "AGENTS.md", NodeKind::System, &adr_dir, &mut written)?;
    }
    for dir in proposable_top_dirs(&tracked, &existing, &adr_dir) {
        let path = format!("{dir}/AGENTS.md");
        if skip_if_ignored(work, &path)? {
            continue;
        }
        scaffold_node(work, &path, NodeKind::Container, &adr_dir, &mut written)?;
    }
    // An already-authored AGENTS.md that carries a stele block but no generated region
    // gets an empty region appended (§3.1: `emit` exits 2 pointing here — `init` must
    // actually scaffold it). Authored bytes above are byte-preserved; a file with BOTH
    // block and region is left byte-identical (the acme idempotency oracle). These are
    // already VCS-tracked, so they never hit the gitignore pre-filter.
    for rel in &tracked {
        ensure_region_for_block(work, rel, &mut written)?;
    }
    // One batch, after every write — staging is best-effort (a warning, never an abort).
    if !written.is_empty() {
        git_add(work, &written);
    }
    Ok(CommandOutput::new(
        json!({ "wrote": written }),
        Some(format!(
            "stele init: scaffolded {} node(s), staged for the next stele build",
            written.len()
        )),
    ))
}

/// Whether `path` (a repo-root-relative POSIX path `init` is about to scaffold) is
/// matched by a `.gitignore`, printing a one-line stderr notice when it is. A scaffold
/// written to an ignored path could never be VCS-tracked, so `stele build` would never
/// see it (§2.4) — skipping keeps `init` from leaving an orphaned, unstageable file (and
/// from aborting mid-scaffold when the later `git add` rejects it).
fn skip_if_ignored(root: &Path, path: &str) -> Result<bool> {
    if git_check_ignore(root, path)? {
        eprintln!(
            "stele init: {path} is matched by a .gitignore — skipped (an ignored path can never \
             be tracked, so stele build would never see it)"
        );
        return Ok(true);
    }
    Ok(false)
}

/// Whether `git check-ignore` matches `path` (§2.4). Exit 0 = ignored, 1 = not ignored;
/// any other status is treated as "not ignored" so the scaffold proceeds. A spawn failure
/// is a §5.3 internal error.
fn git_check_ignore(root: &Path, path: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["check-ignore", "-q", "--", path])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git check-ignore`: {e}")))?;
    Ok(output.status.code() == Some(0))
}

/// Append an empty generated region to an already-authored AGENTS.md that carries a
/// stele block but no region (§3.1). A non-AGENTS.md file, a blockless file (nothing to
/// route), or a file that already has a region is left byte-identical. Records the path
/// for staging so the next `stele build` sees the change.
fn ensure_region_for_block(root: &Path, rel: &Path, written: &mut Vec<String>) -> Result<()> {
    if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
        return Ok(());
    }
    let contents = std::fs::read_to_string(root.join(rel))
        .map_err(|e| SteleError::internal(format!("read {}: {e}", rel.display())))?;
    // Ok(None) is "no stele block" — a blockless file declares no node and needs no
    // region; Ok(Some)/Err both mean a block is present.
    if matches!(parse::parse_agents_file(rel, &contents), Ok(None)) {
        return Ok(());
    }
    if parse::find_region(rel, &contents)?.is_some() {
        return Ok(());
    }
    std::fs::write(root.join(rel), append_empty_region(&contents))
        .map_err(|e| SteleError::internal(format!("write {}: {e}", rel.display())))?;
    written.push(rel.to_string_lossy().replace('\\', "/"));
    Ok(())
}

/// `contents` plus a trailing empty region (§3.1): a blank-line separator then the router
/// markers, authored bytes byte-preserved above.
fn append_empty_region(contents: &str) -> String {
    let mut out = contents.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(EMPTY_REGION);
    out
}

/// The declaring directories of every tracked AGENTS.md that carries a stele block
/// (`""` = repo root). A file whose block is malformed still counts as carrying one, so
/// `init` leaves it untouched (§7 non-destructive) rather than prepending a second.
fn existing_node_dirs(root: &Path, tracked: &[PathBuf]) -> Result<BTreeSet<String>> {
    let mut dirs = BTreeSet::new();
    for rel in tracked {
        if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
            continue;
        }
        let contents = std::fs::read_to_string(root.join(rel))
            .map_err(|e| SteleError::internal(format!("read {}: {e}", rel.display())))?;
        // Ok(None) is "no stele block"; Ok(Some)/Err both mean a block is present.
        if !matches!(parse::parse_agents_file(rel, &contents), Ok(None)) {
            dirs.insert(declaring_dir_str(rel));
        }
    }
    Ok(dirs)
}

/// The top-level directories `init` proposes as containers: each first-level directory
/// holding ≥1 tracked file that is neither an engine-reserved dir (§4.3 `.git`/`.stele`),
/// the ADR directory (nor its ancestor), nor already covered by an existing node at or
/// below it. `.stele/` is the load-bearing exclusion here: its `graph.lock`/index files
/// are VCS-tracked, so it surfaces in the tracked-file listing and would otherwise be
/// proposed as a node — the same §4.3 discipline that keeps it out of the exhaustiveness
/// walk keeps it out of the init proposal. (VCS-ignored and `.steleignore`d paths are
/// already absent from `tracked`, filtered at its source in `tracked_files`.)
fn proposable_top_dirs(
    tracked: &[PathBuf],
    existing: &BTreeSet<String>,
    adr_dir: &str,
) -> Vec<String> {
    let mut tops: BTreeSet<String> = BTreeSet::new();
    for path in tracked {
        let rel = path.to_string_lossy().replace('\\', "/");
        if let Some((top, _)) = rel.split_once('/') {
            tops.insert(top.to_string());
        }
    }
    tops.into_iter()
        .filter(|top| {
            let reserved = assert::IGNORED_DIRS.contains(&top.as_str());
            let is_adr = top == adr_dir || adr_dir.starts_with(&format!("{top}/"));
            let covered = existing
                .iter()
                .any(|dir| dir == top || dir.starts_with(&format!("{top}/")));
            !reserved && !is_adr && !covered
        })
        .collect()
}

/// Scaffold one proposed node's AGENTS.md (§7). No file → write the heading + skeleton
/// block + empty region. Prose-only file → prepend the skeleton (after any leading `#`
/// heading), preserve every existing byte, and append an empty region if the file has
/// none. Records the written path for staging.
fn scaffold_node(
    root: &Path,
    rel_path: &str,
    kind: NodeKind,
    adr_dir: &str,
    written: &mut Vec<String>,
) -> Result<()> {
    let full = root.join(rel_path);
    let contents = match std::fs::read_to_string(&full) {
        Ok(existing) => prepend_skeleton(rel_path, &existing, kind, adr_dir)?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            new_skeleton_file(rel_path, root, kind, adr_dir)
        }
        Err(e) => return Err(SteleError::internal(format!("read {rel_path}: {e}"))),
    };
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SteleError::internal(format!("create dir for {rel_path}: {e}")))?;
    }
    std::fs::write(&full, contents)
        .map_err(|e| SteleError::internal(format!("write {rel_path}: {e}")))?;
    written.push(rel_path.to_string());
    Ok(())
}

/// A brand-new node AGENTS.md (§7): `# <name>` heading, the skeleton block, then an
/// empty generated region.
fn new_skeleton_file(rel_path: &str, root: &Path, kind: NodeKind, adr_dir: &str) -> String {
    let name = node_display_name(rel_path, root);
    format!(
        "# {name}\n\n{}\n{EMPTY_REGION}",
        skeleton_block(kind, adr_dir)
    )
}

/// Prepend the skeleton block to a prose-only file (§7): keep a leading `# heading`
/// first, then the block, then every existing byte of prose, then an empty region if
/// the file carries none. Preserves all prior content.
fn prepend_skeleton(rel_path: &str, prose: &str, kind: NodeKind, adr_dir: &str) -> Result<String> {
    let (heading, body) = split_leading_heading(prose);
    let mut out = String::new();
    if let Some(heading) = heading {
        out.push_str(heading);
        out.push_str("\n\n");
    }
    out.push_str(&skeleton_block(kind, adr_dir));
    out.push('\n');
    out.push_str(body);
    // Append an empty region only when the (now-combined) file has none.
    if parse::find_region(Path::new(rel_path), &out)?.is_none() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(EMPTY_REGION);
    }
    Ok(out)
}

/// Split a leading `# heading` line off the front of a file: `(Some(heading), rest)`
/// when the first non-empty line is an ATX heading, else `(None, whole)`. The rest
/// keeps its bytes verbatim (leading blank lines after the heading are dropped from
/// `rest` only to the extent the reassembler re-adds a single separator).
fn split_leading_heading(prose: &str) -> (Option<&str>, &str) {
    let trimmed = prose.trim_start_matches(['\n', '\r']);
    let Some(first_end) = trimmed.find('\n') else {
        // Single line: a heading with no body, or bare prose.
        return if trimmed.starts_with('#') {
            (Some(trimmed.trim_end()), "")
        } else {
            (None, prose)
        };
    };
    let first = trimmed[..first_end].trim_end();
    if first.starts_with('#') {
        let rest = trimmed[first_end + 1..].trim_start_matches(['\n', '\r']);
        (Some(first), rest)
    } else {
        (None, prose)
    }
}

/// The skeleton `stele` block (§7): the required `kind` active, every other typed field
/// present but commented out so nothing is auto-filled (purpose/invariants stay the
/// human's to write, §7). The `decided_by` hint names the detected ADR directory.
fn skeleton_block(kind: NodeKind, adr_dir: &str) -> String {
    format!(
        "```stele\nkind: {}\n# purpose:            # \u{2264}{PURPOSE_MAX_CHARS}-char scent \u{2014} fill it in (never auto-generated, \u{00A7}7)\n# commands: {{}}\n# invariants: []\n# hazards: []\n# edges:\n#   depends: []\n#   decided_by: [{adr_dir}/0001]\n# budget:\n```\n",
        kind.as_str()
    )
}

/// The `# heading` name for a scaffolded node: the declaring directory's final segment,
/// or the repo directory name for the root system node.
fn node_display_name(rel_path: &str, root: &Path) -> String {
    match rel_path.rsplit_once('/') {
        Some((dir, _)) => dir.rsplit('/').next().unwrap_or(dir).to_string(),
        None => root
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "root".to_string()),
    }
}

/// A tracked AGENTS.md's declaring directory as a string (`""` = repo root).
fn declaring_dir_str(rel: &Path) -> String {
    match rel.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_string_lossy().replace('\\', "/"),
        _ => String::new(),
    }
}

/// Detect the repo's ADR directory (§10.5): the first of [`ADR_DIRS`] holding a tracked
/// `NNNN-*.md`, else the greenfield default `adr/` for the skeleton's `decided_by` hint.
fn detect_adr_dir(tracked: &[PathBuf]) -> String {
    let paths: Vec<String> = tracked
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    ADR_DIRS
        .iter()
        .find(|dir| paths.iter().any(|p| adr_number(dir, p).is_some()))
        .map(|d| d.to_string())
        .unwrap_or_else(|| ADR_DIRS[0].to_string())
}

/// Stage the files `init` wrote (`git add`) so the next `stele build` — which scans
/// VCS-tracked files only (§2.4) — compiles them. Staging is best-effort: the scaffold is
/// already on disk, so a `git add` failure (or a git that will not spawn) is a stderr
/// warning naming the paths to stage by hand, NEVER a post-write abort (`init` still
/// exits 0). Ignored targets were pre-filtered by [`skip_if_ignored`], so the common
/// failure mode is already gone.
fn git_add(root: &Path, paths: &[String]) {
    let result = Command::new("git")
        .arg("add")
        .arg("--")
        .args(paths)
        .current_dir(root)
        .output();
    let warning = match result {
        Ok(output) if output.status.success() => return,
        Ok(output) => format!(
            "`git add` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(e) => format!("could not run `git add`: {e}"),
    };
    eprintln!(
        "stele init: {warning}\n  stage the scaffold yourself so stele build sees it: git add -- {}",
        paths.join(" ")
    );
}

// ─── the source pipeline (§5.1) ──────────────────────────────────────────────

/// Sources → in-memory graph (§5.1): parse every VCS-tracked AGENTS.md into nodes,
/// scan the tracked tree for comment anchors (§2.4), resolve each claim's anchor to
/// a `file:line` (§2.4), index the repo's ADRs (§2.6), and derive import edges
/// (§2.3/§4.2). Shared by `build` and `check`, so every derived slot is recomputed
/// identically on both paths. `verified` is stamped only by `build` afterward (§4.5).
fn build_graph(root: &Path) -> Result<Graph> {
    let tracked = tracked_files(root)?;

    let mut graph = Graph::default();
    for rel in &tracked {
        if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
            continue;
        }
        let contents = read_node_source(root, rel)?;
        if let Some(node) = parse::parse_agents_file(rel, &contents)? {
            graph.insert(node)?;
        }
    }

    graph.anchors = anchors::scan(root, &tracked)?;
    resolve_claim_anchors(root, &mut graph.nodes, &graph.anchors)?;
    graph.adrs = build_adr_index(root, &tracked)?;

    // Import extraction (§2.3/§4.2): attribute each cross-boundary reference to its
    // owning node via the territory index, then fill each node's `extracted.imports`
    // and retain the per-edge reference occurrences for the structural class (§4.2).
    let territory = graph.territory();
    let extraction = extract::extract_imports(root, &tracked, &territory)?;
    for node in &mut graph.nodes {
        node.extracted_imports = extraction
            .per_node
            .get(&node.id)
            .cloned()
            .unwrap_or_default();
    }
    graph.import_edges = extraction.edges;
    Ok(graph)
}

/// Read a node source (AGENTS.md, §3.1). Unlike the scan's binary-skip (§2.4), a
/// declared node source that is not valid UTF-8 is a hard §5.3 input error naming the
/// file — a node whose own bytes cannot be read is a repo-out-of-spec condition, never
/// silently dropped. A genuine IO failure stays a §5.3 internal error.
fn read_node_source(root: &Path, rel: &Path) -> Result<String> {
    match std::fs::read(root.join(rel)) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| {
            SteleError::input_msg(format!(
                "{}: node source is not valid UTF-8 (§3.1)",
                rel.display()
            ))
        }),
        Err(e) => Err(SteleError::internal(format!("read {}: {e}", rel.display()))),
    }
}

/// Resolve every claim's anchor to a `file:line` (§2.4), recomputed every build. An
/// `lm:<slug>` anchor resolves to the winning landmark occurrence (or stays `null`
/// when the slug has zero occurrences — build stays 0, §4.1 fails it later). A
/// `<path>#<symbol>` anchor resolves via tree-sitter: exactly one definition →
/// its line; zero or many → `null` plus the §4.1 unresolved-vs-ambiguous marker.
fn resolve_claim_anchors(
    root: &Path,
    nodes: &mut [Node],
    anchors: &crate::model::AnchorData,
) -> Result<()> {
    for node in nodes.iter_mut() {
        for claim in node.invariants.iter_mut().chain(node.hazards.iter_mut()) {
            if let Some(slug) = claim
                .anchor
                .strip_prefix(crate::model::LANDMARK_ANCHOR_PREFIX)
            {
                match anchors.winner(slug) {
                    Some(occ) => {
                        claim.resolved = Some(format!("{}:{}", occ.file, occ.line));
                        claim.resolution = Resolution::Resolved;
                    }
                    None => claim.resolution = Resolution::Unresolved,
                }
            } else if let Some((path, symbol)) = claim.anchor.rsplit_once('#') {
                match anchors::resolve_symbol(root, path, symbol)? {
                    SymbolResolution::Resolved(line) => {
                        claim.resolved = Some(format!("{path}:{line}"));
                        claim.resolution = Resolution::Resolved;
                    }
                    SymbolResolution::Ambiguous => claim.resolution = Resolution::Ambiguous,
                    SymbolResolution::Unresolved => claim.resolution = Resolution::Unresolved,
                }
            } else {
                // derive_slug already accepted the anchor, so it is one of the two
                // namespaces; this arm is unreachable in practice.
                claim.resolution = Resolution::Unresolved;
            }
        }
    }
    Ok(())
}

/// Stamp `verified = {sha, digest}` on every claim whose anchor RESOLVES (§4.5),
/// using the full 40-hex `HEAD` sha (deterministic within a commit, so two builds
/// byte-match) and the tree-sitter structural digest of the claim's bound definition
/// (§4.5; `null` only where the anchored file's language has no bundled parser).
/// Unresolved claims stay `verified:null`. `build` is the sole stamper — `check`
/// carries `verified` over instead (§4.5). Returns `false` when HEAD is unborn (no
/// commit yet): every claim is left `verified:null` and the caller prints one notice
/// — a greenfield `stele build` still succeeds (§7), it simply has no watermark to
/// anchor to until the first commit.
fn stamp_verified(root: &Path, graph: &mut Graph) -> Result<bool> {
    let Some(sha) = head_sha(root)? else {
        return Ok(false);
    };
    for node in &mut graph.nodes {
        for claim in node.invariants.iter_mut().chain(node.hazards.iter_mut()) {
            if let Some(resolved) = claim.resolved.clone() {
                claim.verified = Some(Verified {
                    sha: sha.clone(),
                    digest: anchors::digest_for_claim(root, &claim.anchor, &resolved)?,
                });
            }
        }
    }
    Ok(true)
}

/// The full `HEAD` commit sha (§4.5 watermark), or `None` when HEAD is unborn — a repo
/// with no commit yet (§7 greenfield). `git rev-parse --verify HEAD` fails on an unborn
/// HEAD; since `build_graph` already ran `git ls-files` successfully, that failure means
/// "no commit", never "not a git repo". A genuine spawn failure stays a §5.3 internal
/// error.
fn head_sha(root: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git rev-parse --verify HEAD`: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

/// The source-scan file set (§2.4 scan scope): VCS-tracked files (so `.gitignore` is
/// honored by construction) MINUS every path a committed root `.steleignore` matches.
/// The single choke point every scan reads — node discovery, anchors, extraction, and
/// the §4.3 directory walk all consume this list — so filtering here makes an ignored
/// subtree invisible to all of them at once (`steleignore` module). Repo-root-relative
/// and sorted for deterministic iteration.
fn tracked_files(root: &Path) -> Result<Vec<PathBuf>> {
    let ignore = Steleignore::load(root)?;
    Ok(git_tracked_files(root)?
        .into_iter()
        .filter(|rel| !ignore.is_ignored(&rel.to_string_lossy().replace('\\', "/")))
        .collect())
}

/// The untracked-but-not-gitignored file list (`git ls-files --others
/// --exclude-standard`): files present in the work tree that are neither VCS-tracked nor
/// matched by a `.gitignore`. Repo-root-relative and sorted, NUL-delimited to survive
/// unusual filenames. Feeds only [`warn_untracked_node_files`] — a spawn or non-zero-exit
/// failure is a §5.3 internal error.
fn git_untracked_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git ls-files --others`: {e}")))?;
    if !output.status.success() {
        return Err(SteleError::internal(format!(
            "`git ls-files --others` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<PathBuf> = listing
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect();
    files.sort();
    Ok(files)
}

/// Warn (stderr only) about untracked AGENTS.md files that declare a stele node — a real
/// adoption trap (§2.4): `build`/`check` scan VCS-tracked files ONLY, so an agent that
/// hand-creates an AGENTS.md with a stele block but never `git add`s it leaves the graph
/// silently short a node with no other signal. For each untracked, non-gitignored,
/// non-steleignored AGENTS.md whose content carries a stele block, print ONE warning line.
/// This is purely diagnostic: stdout, the process exit code, and the `--json` envelope are
/// all untouched (machine consumers read stdout/exit, unaffected). Excluded from the warning
/// (each for a reason): a gitignored path (`--exclude-standard` drops it; it could never be
/// tracked), a steleignored path (already invisible to every scan, §2.4), and a blockless
/// AGENTS.md (`Ok(None)` = plain-markdown degradation, not a node).
fn warn_untracked_node_files(root: &Path) -> Result<()> {
    let ignore = Steleignore::load(root)?;
    for rel in git_untracked_files(root)? {
        if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
            continue;
        }
        let posix = rel.to_string_lossy().replace('\\', "/");
        if ignore.is_ignored(&posix) {
            continue;
        }
        // Non-UTF-8 bytes cannot carry a parseable stele block, so a read failure is a
        // silent skip (never a node). `Ok(None)` = blockless; `Ok(Some)`/`Err` both mean
        // a block is present (a malformed block still declares a node the scan omits).
        let Ok(contents) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        if !matches!(parse::parse_agents_file(&rel, &contents), Ok(None)) {
            eprintln!(
                "warning: {posix} declares a stele node but is not tracked — run git add \
                 (build scans tracked files only, §2.4)"
            );
        }
    }
    Ok(())
}

/// The raw VCS-tracked file list (§2.4), repo-root-relative and sorted. `git ls-files
/// -z` is NUL-delimited to survive unusual filenames.
fn git_tracked_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git ls-files`: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Outside a work tree git prints "fatal: not a git repository …" — a user
        // mistake (§5.3 input, exit 2), not an internal fault. Surface a one-liner
        // instead of the raw git text so `stele <verb>` outside a repo fails cleanly.
        if stderr.contains("not a git repository") {
            return Err(SteleError::input_msg(
                "not a git repository (stele scans VCS-tracked files; run inside a git repo)",
            ));
        }
        return Err(SteleError::internal(format!(
            "`git ls-files` failed: {}",
            stderr.trim()
        )));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<PathBuf> = listing
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect();
    files.sort();
    Ok(files)
}

// ─── the ADR index (§2.6) ─────────────────────────────────────────────────────

/// The ADR directories stele probes, in precedence order (§10 item 5); the first
/// that holds a tracked `NNNN-*.md` file is the repo's ADR dir.
const ADR_DIRS: [&str; 3] = ["adr", "doc/adr", "docs/adr"];

/// Index the repo's ADRs (§2.6): detect the ADR dir (the first of [`ADR_DIRS`] with
/// a tracked `NNNN-*.md` file), then parse each such file into an [`AdrEntry`] —
/// number and zero-padded id from the filename, status from its `Status:` line.
fn build_adr_index(root: &Path, tracked: &[PathBuf]) -> Result<Vec<AdrEntry>> {
    let paths: Vec<String> = tracked
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    let Some(dir) = ADR_DIRS
        .iter()
        .find(|dir| paths.iter().any(|p| adr_number(dir, p).is_some()))
    else {
        return Ok(Vec::new());
    };

    let mut entries = Vec::new();
    for path in &paths {
        let Some(nnnn) = adr_number(dir, path) else {
            continue;
        };
        let number: i64 = nnnn.parse().map_err(|_| {
            SteleError::input(path, 1, format!("ADR number {nnnn:?} is not an integer"))
        })?;
        let contents = std::fs::read_to_string(root.join(path))
            .map_err(|e| SteleError::internal(format!("read {path}: {e}")))?;
        entries.push(AdrEntry {
            id: format!("{dir}/{nnnn}"),
            number,
            status: adr_status(&contents),
            path: path.clone(),
        });
    }
    Ok(entries)
}

/// The zero-padded number of a tracked ADR file `<dir>/NNNN-*.md` (e.g. `"0007"`),
/// or `None` when the path is not an ADR file directly under `dir`.
fn adr_number<'a>(dir: &str, path: &'a str) -> Option<&'a str> {
    let name = path.strip_prefix(dir)?.strip_prefix('/')?;
    if name.contains('/') || !name.ends_with(".md") {
        return None;
    }
    let digits = name.split('-').next().filter(|d| !d.is_empty())?;
    digits.bytes().all(|b| b.is_ascii_digit()).then_some(digits)
}

/// The ADR status (§2.6): the lowercased status token, drawn from whichever of the three
/// conventions in the wild the file uses, or `"unknown"` when none is present. §4.1
/// checks `≠ superseded`.
///   • MADR YAML frontmatter — a `status:` key inside a leading `---`-fenced block;
///   • adr-tools/Nygard inline — a `Status: X` on any line (the original form);
///   • adr-tools/Nygard heading — a `## Status` ATX heading, the value on the first
///     non-empty line after it (markdown links stripped, first word token decides).
/// Frontmatter outranks the inline form, which outranks the heading form.
fn adr_status(contents: &str) -> String {
    const UNKNOWN: &str = "unknown";
    frontmatter_status(contents)
        .or_else(|| inline_status(contents))
        .or_else(|| heading_status(contents))
        .unwrap_or_else(|| UNKNOWN.to_string())
}

/// The MADR YAML-frontmatter status: the `status:` key inside a leading `---` fence.
/// `None` when the file opens with no frontmatter or the block carries no `status:` key.
fn frontmatter_status(contents: &str) -> Option<String> {
    const FENCE: &str = "---";
    let mut lines = contents.lines();
    if lines.next()?.trim_end() != FENCE {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == FENCE {
            return None;
        }
        if let Some(value) = trimmed.strip_prefix("status:") {
            return value.split_whitespace().next().map(str::to_ascii_lowercase);
        }
    }
    None
}

/// The inline `Status: X` form (adr-tools/Nygard, the original): the lowercased first
/// whitespace token after the first `Status:` on any line. `None` when no line carries
/// a `Status:` with a following token.
fn inline_status(contents: &str) -> Option<String> {
    contents
        .lines()
        .find_map(|line| line.split("Status:").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_ascii_lowercase)
}

/// The `## Status` heading form (adr-tools/Nygard): the first non-empty line after a
/// `## Status` ATX heading, reduced to its status word. `None` when the file has no such
/// heading or nothing follows it.
fn heading_status(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    while let Some(line) = lines.next() {
        if is_status_heading(line) {
            return lines
                .by_ref()
                .find(|body| !body.trim().is_empty())
                .and_then(status_word);
        }
    }
    None
}

/// Whether `line` is a `## Status` ATX heading (any heading depth; case-insensitive) —
/// one or more leading `#`, then only the word `Status`.
fn is_status_heading(line: &str) -> bool {
    let trimmed = line.trim();
    let body = trimmed.trim_start_matches('#');
    body.len() < trimmed.len() && body.trim().eq_ignore_ascii_case("status")
}

/// The status word of a heading-form value line: markdown-link syntax reduced to its
/// text by taking the first maximal run of ASCII letters, lowercased (so
/// `[Accepted](0009.md)` → `accepted`, `Superseded by [ADR-9](…)` → `superseded`).
/// `None` for a line with no letters.
fn status_word(line: &str) -> Option<String> {
    let word: String = line
        .chars()
        .skip_while(|c| !c.is_ascii_alphabetic())
        .take_while(char::is_ascii_alphabetic)
        .collect();
    (!word.is_empty()).then(|| word.to_ascii_lowercase())
}

// ─── the committed lock (§5.3) ────────────────────────────────────────────────

/// Read the committed lock, mapping a missing file to the §5.3 input error (exit
/// 2 + `run stele build`) that `check`/`emit`/`node` share. A genuine IO failure
/// is an internal error (exit 3).
fn read_committed_lock(root: &Path) -> Result<String> {
    let path = root.join(LOCK_DIR).join(LOCK_FILE);
    std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => SteleError::input_msg(format!(
            "no committed lock at {LOCK_DIR}/{LOCK_FILE}; {RUN_BUILD_HINT}"
        )),
        _ => SteleError::internal(format!("read {LOCK_DIR}/{LOCK_FILE}: {e}")),
    })
}

/// Write the lock via temp-file + rename so an interrupted write never leaves a
/// partial lock (§5.3). The caller has already produced the full byte string, so
/// any input error has aborted before this point. The temp name is unique per
/// writer (pid + process-local counter) in the SAME directory as the target, so two
/// concurrent builds never share a temp file — otherwise the loser's rename races on
/// a temp the winner already renamed away (ENOENT, exit 3). The temp is cleaned up
/// best-effort if the rename fails.
fn write_lock_atomic(root: &Path, bytes: &str) -> Result<()> {
    static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let dir = root.join(LOCK_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| SteleError::internal(format!("create {LOCK_DIR}: {e}")))?;
    let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_name = format!("{LOCK_TEMP}.{}.{seq}", std::process::id());
    let temp = dir.join(&temp_name);
    std::fs::write(&temp, bytes)
        .map_err(|e| SteleError::internal(format!("write {LOCK_DIR}/{temp_name}: {e}")))?;
    if let Err(e) = std::fs::rename(&temp, dir.join(LOCK_FILE)) {
        let _ = std::fs::remove_file(&temp);
        return Err(SteleError::internal(format!(
            "rename into {LOCK_DIR}/{LOCK_FILE}: {e}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A normal-mode workspace rooted at `root` — `home` and `work` the identical path,
    /// the only mode a direct verb call exercises (§3.5). Mirrors what [`resolve_workspace`]
    /// returns for a normal checkout, without the `resolve_workspace` git spawn.
    fn normal_workspace(root: &Path) -> Workspace {
        Workspace {
            home: root.to_path_buf(),
            mode: Mode::Normal,
            work: root.to_path_buf(),
        }
    }

    /// Run `git` in `root`, asserting success — the tracked-file/staging surface `init`
    /// needs (§2.4). Kept minimal: `init` reads `git ls-files` and calls `git add`.
    fn git(root: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("run git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    #[test]
    fn adr_status_reads_all_three_conventions() {
        // MADR YAML frontmatter (F5).
        assert_eq!(
            adr_status("---\nstatus: superseded\ndate: 2026-01-01\n---\n# t\n"),
            "superseded"
        );
        // Inline `Status: X` — the original form still works, unchanged.
        assert_eq!(
            adr_status("# 7\n\nDate: 2026-03-02 · Status: Accepted\n"),
            "accepted"
        );
        // Nygard/adr-tools `## Status` heading: value on the first non-empty line after,
        // markdown links stripped, first word token decides.
        assert_eq!(adr_status("# 1\n\n## Status\n\nAccepted\n"), "accepted");
        assert_eq!(
            adr_status("# 1\n\n## Status\n\n[Superseded](0009-x.md) by ADR 9\n"),
            "superseded"
        );
        // No status anywhere → unknown.
        assert_eq!(adr_status("# t\n\n## Context\n\nnothing here\n"), "unknown");
    }

    /// A file with a stele block but NO generated region gets an empty region appended at
    /// EOF (§3.1), authored bytes byte-preserved; a second `init` is byte-identical.
    #[test]
    fn init_appends_region_to_block_file_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.email", "t@example.com"]);
        git(root, &["config", "user.name", "t"]);

        let authored = "# proj\n\n```stele\nkind: system\npurpose: demo\n```\n\nsome prose\n";
        std::fs::write(root.join("AGENTS.md"), authored).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "seed"]);

        let ws = normal_workspace(root);
        init(&ws, &["init"]).unwrap();
        let after = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        // Authored bytes are preserved verbatim as a prefix; a region now exists.
        assert!(
            after.starts_with(authored),
            "authored bytes not preserved:\n{after}"
        );
        assert!(
            parse::find_region(Path::new("AGENTS.md"), &after)
                .unwrap()
                .is_some(),
            "no region appended:\n{after}"
        );
        // Second init leaves the file byte-identical (region already present).
        init(&ws, &["init"]).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            after
        );
    }

    /// `append_empty_region` inserts a blank-line separator then the router markers, and
    /// its output carries a locatable region.
    #[test]
    fn append_empty_region_separates_and_is_findable() {
        let out = append_empty_region("body\n");
        assert_eq!(out, format!("body\n\n{EMPTY_REGION}"));
        assert!(
            parse::find_region(Path::new("AGENTS.md"), &out)
                .unwrap()
                .is_some()
        );
    }
}
