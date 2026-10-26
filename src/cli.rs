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
use crate::legacy;
use crate::lock::{self, Lock, LockClaim, LockNode};
use crate::model::{
    AdrEntry, DECISION_PREFIX, ExitCode, Graph, Node, NodeKind, PURPOSE_MAX_CHARS, Resolution,
    Result, SYSTEM_ID, SteleError, Verified, dir_contains, normalize_id,
};
use crate::parse;
use crate::steleignore::Steleignore;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The committed lock, relative to the repo root.
const LOCK_DIR: &str = ".stele";
const LOCK_FILE: &str = "graph.lock";

/// The message every read command emits when the committed lock is missing or
/// stale (§5.3); tests assert on the `run stele build` substring.
const RUN_BUILD_HINT: &str = "run stele build";

/// The machine-output flag (§5.3): exactly one JSON envelope on stdout.
const JSON_FLAG: &str = "--json";

/// The `check --freeze` flag (§4.2): baseline current structural violations.
const FREEZE_FLAG: &str = "--freeze";
/// `check`'s §5.1 grammar: `--freeze` is a run of its own.
const CHECK_USAGE: &str =
    "stele check [--only <class>] [--run-commands] [--report] | stele check --freeze";

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
// invoking work-tree top level (all git spawns and source/code scanning run here) and `home` is
// the graph home where the engine's artifacts live — the lock, `.stele/index/`, node
// sources (§3.2/§3.5). In normal mode the two are the SAME path (`Path::new(".")` when
// invoked at the top level), so every join is byte-identical and no §2–§7 behavior moves.
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
    /// The git common directory (`git rev-parse --git-common-dir`, canonicalized) — the
    /// directory whose `info/exclude` carries the undercover managed block (§3.5). Read only
    /// in [`Mode::Undercover`] (for the self-healing exclude re-install); in [`Mode::Normal`]
    /// it holds `work` verbatim (the cheapest value — never read).
    pub common: PathBuf,
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
const TREE_DIR: &str = ".stele/tree";

/// `(common, home)` for the work tree at `work` (§3.5): the git common directory (`git
/// rev-parse --git-common-dir`, joined onto `work` when relative, canonicalized) — `<repo>/.git`
/// in a normal checkout, the shared bare `.git` under a linked-worktree layout — and its parent,
/// the graph home. `None` on ANY failure (a non-repo cwd, a git that will not spawn, an
/// uncanonicalizable or parentless path) so [`resolve_workspace`] degrades to normal mode.
fn graph_home(work: &Path) -> Option<(PathBuf, PathBuf)> {
    let common = match git_stdout(work, "--git-common-dir") {
        out if out.is_empty() => return None,
        out => work.join(out).canonicalize().ok()?,
    };
    let home = common.parent()?.to_path_buf();
    Some((common, home))
}

/// The top level of the work tree containing `cwd` (`git rev-parse --show-cdup` joined onto
/// it), or `cwd` verbatim at the top level or on any git failure. Every verb runs from here
/// (§3.5): a subdirectory invocation must neither scan a cwd-relative subtree nor write a
/// partial lock (or the undercover `CLAUDE.local.md` shim) beside the cwd. In a linked
/// worktree this is that worktree's own root.
fn work_tree_root(cwd: &Path) -> PathBuf {
    match git_stdout(cwd, "--show-cdup") {
        cdup if cdup.is_empty() => cwd.to_path_buf(),
        cdup => cwd.join(cdup),
    }
}

/// The trimmed stdout of `git rev-parse <flag>` in `cwd`, empty on any git failure.
fn git_stdout(cwd: &Path, flag: &str) -> String {
    Command::new("git")
        .args(["rev-parse", flag])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(out.stdout.trim_ascii()).into_owned())
        .unwrap_or_default()
}

/// `path`, typed relative to the cwd, as a top-level-relative path: joined onto the cwd's
/// `--show-prefix` with `.`/`..` folded lexically (a `..` above the top level is kept, for
/// [`repo_relative`] to refuse).
fn rebase_on_cwd(prefix: &str, path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for segment in prefix.split('/').chain(path.split('/')) {
        match segment {
            "" | "." => {}
            ".." if out.last().is_some_and(|s| *s != "..") => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// Resolve the [`Workspace`] for the cwd once at dispatch entry (§3.5). `work` is the
/// work-tree top level ([`work_tree_root`]); the graph home is the canonicalized parent of
/// the git common dir; when `<home>/.stele/undercover` exists the result is
/// [`Mode::Undercover`] rooted there. On ANY failure to locate the home (non-repo cwd, no
/// parent) OR an absent marker the result is [`Mode::Normal`] with `home == work` — at the
/// top level the cwd verbatim, so every downstream join stays byte-identical. NEVER errors:
/// a non-repo cwd falls through to the verbs' own exit-2 paths (`git ls-files` reports "not
/// a git repository").
pub fn resolve_workspace(cwd: &Path) -> Workspace {
    let work = work_tree_root(cwd);
    let (common, home, mode) = match graph_home(&work) {
        Some((common, home)) if home.join(UNDERCOVER_MARKER).exists() => {
            (common, home, Mode::Undercover)
        }
        _ => (work.clone(), work.clone(), Mode::Normal),
    };
    Workspace {
        common,
        home,
        mode,
        work,
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
    // messages, never a CLI envelope. It takes no argument (its own `--json` refusal aside):
    // any other is a §5.3 bad flag, refused before the server starts.
    if args.first().map(String::as_str) == Some("serve") {
        if let Some(token) = args.get(1).filter(|a| *a != JSON_FLAG) {
            eprintln!("unexpected argument {token:?}; usage: stele serve");
            return ExitCode::Input as i32;
        }
        return crate::serve::run(Path::new("."), args);
    }
    let json = args.iter().any(|a| a == JSON_FLAG);
    let mut rest: Vec<String> = args.iter().filter(|a| *a != JSON_FLAG).cloned().collect();
    // Every verb runs from the top level (§3.5), so a typed relative path argument —
    // `--touching`, the only one — is rebased onto the cwd's prefix to keep its cwd-relative
    // meaning; an absolute one is left to [`repo_relative`].
    if let Some(i) = rest.iter().position(|a| a == "--touching")
        && let Some(path) = rest.get_mut(i + 1)
        && Path::new(path).is_relative()
    {
        *path = rebase_on_cwd(&git_stdout(Path::new("."), "--show-prefix"), path);
    }
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    // Resolve the home/work split once (§3.5), after the serve/--version short-circuits;
    // `work` is the work-tree top level, whatever the cwd; in normal mode `home == work`.
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
                print!("{}", render_human(&output));
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

/// One verb's §5.1 grammar beyond the global `--json` (stripped before dispatch): `(name,
/// entry point taking argv with `args[0]` the verb, positional count, switches, value-taking
/// flags, usage)`.
type Verb = (
    &'static str,
    fn(&Workspace, &[&str]) -> Result<CommandOutput>,
    usize,
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
);

/// Every verb (§5.1), each held to its own [`Verb`] grammar.
const VERBS: &[Verb] = &[
    ("blame", blame, 1, &[], &[], "stele blame <node-id>/<slug>"),
    ("build", build, 0, &[], &[], "stele build"),
    (
        "check",
        check,
        0,
        &[FREEZE_FLAG, REPORT_FLAG, RUN_COMMANDS_FLAG],
        &["--only"],
        CHECK_USAGE,
    ),
    (
        "emit",
        emit,
        0,
        &[EMIT_CHECK_FLAG, EMIT_CLAUDE_RULES_FLAG],
        &[],
        "stele emit [--check] [--claude-rules]",
    ),
    (
        "hazards",
        hazards,
        0,
        &[],
        &["--node"],
        "stele hazards [--node <id>]",
    ),
    (
        "init",
        init,
        0,
        &[UNDERCOVER_FLAG],
        &[],
        "stele init [--undercover]",
    ),
    (
        "invariants",
        invariants,
        0,
        &[],
        &["--touching"],
        "stele invariants [--touching <path>]",
    ),
    ("migrate", migrate, 0, &[], &[], "stele migrate"),
    ("node", node, 1, &[], &[], "stele node <id>"),
    (
        "nodes",
        nodes,
        0,
        &[],
        &["--kind"],
        "stele nodes [--kind <kind>]",
    ),
    ("root", cmd_root, 0, &[], &[], "stele root"),
    (
        "unfold",
        unfold,
        1,
        &[],
        &["--depth"],
        "stele unfold <id> [--depth N]",
    ),
];

/// Run the verb `args[0]` names, after holding its argv to the [`VERBS`] grammar: any token
/// past its positionals that is no flag of its own is a §5.3 bad flag (exit 2), surfaced
/// BEFORE the verb runs — a mistyped `check --run-command` never silently skips liveness, a
/// mistyped `init --help` never scaffolds a live tree, a repeated valued flag is refused (the
/// verb reads one value, so `--only a --only b` would drop `b`). A missing positional or flag
/// value is left to the verb's own usage error.
fn dispatch(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    // `main` decodes argv lossily, so U+FFFD marks a non-UTF-8 byte: no verb, id or path.
    if let Some(token) = args.iter().find(|a| a.contains('\u{fffd}')) {
        return Err(SteleError::input_msg(format!(
            "argument {token:?} is not valid UTF-8"
        )));
    }
    match args.first().copied() {
        Some(verb) => {
            let (_, run, arity, switches, valued, usage) = VERBS
                .iter()
                .find(|v| v.0 == verb)
                .ok_or_else(|| SteleError::input_msg(format!("unknown command: {verb:?}")))?;
            let (mut tokens, mut positionals, mut seen) = (args[1..].iter(), 0, vec![]);
            while let Some(token) = tokens.next() {
                if valued.contains(token) && !seen.contains(&token) {
                    seen.push(token);
                    tokens.next();
                } else if positionals < *arity && !token.starts_with("--") {
                    positionals += 1;
                } else if !switches.contains(token) {
                    return Err(SteleError::input_msg(format!(
                        "unexpected argument {token:?}; usage: {usage} [--json]"
                    )));
                }
            }
            run(ws, args)
        }
        None => Err(SteleError::input_msg(
            "usage: stele root | node <id> | unfold <id> | invariants | hazards | nodes | \
             check | emit | blame | build | init | migrate | serve | --version   (add --json \
             for the machine envelope; serve speaks MCP on stdio and takes no --json)",
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

/// A success's human-path stdout: its summary line (findings render instead when present),
/// then any `check --report` block (§4.2 reason-plus-visibility; under `--json` it rides in
/// `data`). Shared by [`run`] and [`serve_render`], so the two never diverge.
fn render_human(output: &CommandOutput) -> String {
    let mut text = match (&output.summary, output.findings.is_empty()) {
        (Some(summary), true) => format!("{summary}\n"),
        (None, true) => String::new(),
        (_, false) => assert::render_human(&output.findings),
    };
    text.push_str(output.report.as_deref().unwrap_or(""));
    text
}

/// Dispatch one verb and render it exactly as [`run`]'s human path would print to stdout —
/// the single code path the CLI and `serve` share (§5.2: same engine, tools mirror the
/// verbs, no duplicated rendering): [`render_human`] on success; an input/internal error
/// becomes `text = <error message>` with the error's exit class. serve maps exit ≥ 2 to an
/// `isError` tool result (§5.2).
pub fn serve_render(ws: &Workspace, argv: &[&str]) -> VerbRender {
    match dispatch(ws, argv) {
        Ok(output) => VerbRender {
            exit: if output.findings.is_empty() { 0 } else { 1 },
            text: render_human(&output),
        },
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
    git_ls_files(root, &[])?;
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

/// `stele build` (§5.1): sources → in-memory graph → atomically written lock.
/// Any input error aborts before a single byte is written (§5.3 atomicity).
/// `build` never reads `.stele/config.toml` — config tunes `check`/`emit`, not the
/// graph (§3.4).
fn build(ws: &Workspace, _: &[&str]) -> Result<CommandOutput> {
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
fn check(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
fn blame(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
/// The CLAUDE.md shim (§3.3): one line pointing Claude Code at the sibling AGENTS.md.
/// Written at the graph home AND beside every nested node's AGENTS.md — Claude Code has no
/// native AGENTS.md discovery at any level (verified empirically 2026-09-16, 2.1.273), but a
/// nested `CLAUDE.md` is lazily loaded on the first read under its directory, and its
/// `@AGENTS.md` import pulls the sibling node file with it.
const CLAUDE_SHIM: &str = "CLAUDE.md";
const CLAUDE_SHIM_CONTENT: &str = "@AGENTS.md\n";
/// The undercover shim (§3.5): the single materialized file, at the invoking work-tree
/// root, a relative `@`-import of the overlay root node. `CLAUDE.local.md` is auto-loaded
/// alongside `CLAUDE.md` and docs-recommended gitignored (verified 2026-07-21) — the private
/// analogue of the §3.3 `@AGENTS.md` line.
const CLAUDE_LOCAL_SHIM: &str = "CLAUDE.local.md";

/// `stele emit` (§5.1): read the COMMITTED lock (never rebuild, §3.2/§5.1) and render
/// every node's generated region in place — only between its markers — plus the
/// transpose indexes (§6.1) and the CLAUDE.md shim (§3.3). `--check` diffs instead of
/// writing (divergence → exit 1, §3.1); `--claude-rules` additionally renders
/// path-scoped `.claude/rules/*.md` (§3.3, opt-in). Like `node`/`blame` it trusts the
/// committed lock and does not byte-compare — staleness is `check`'s job (§5.3).
fn emit(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    let _config = config::load(&ws.home)?;
    let on_disk = read_committed_lock(&ws.home)?;
    require_lock_version(&on_disk)?;
    let lock = lock::parse_lock(&on_disk)?;

    if ws.mode == Mode::Undercover {
        // Self-heal (§3.5): re-ensure the `info/exclude` block before rendering, so the shim and
        // overlay this emit (re)writes never surface in `git status`. No-op when unchanged.
        ensure_info_exclude(&ws.common)?;
    }

    // `--claude-rules` is a normal-mode-only projection (§3.5): undercover materializes exactly
    // ONE file, the `CLAUDE.local.md` shim, so the multi-file `.claude/rules/*.md` fan-out has
    // no home here — reject it as an input error (exit 2) rather than silently scaffolding a
    // second surface. Checked before the `--check` dispatch so the flag is rejected regardless.
    if ws.mode == Mode::Undercover && args.contains(&EMIT_CLAUDE_RULES_FLAG) {
        return Err(SteleError::input_msg(
            "--claude-rules is unavailable in undercover mode; the single materialized file is \
             CLAUDE.local.md",
        ));
    }

    // All emit targets live at the graph home (§3.5): node regions, the transpose indexes,
    // the shim, and any `.claude/rules/*.md` all render under `home`.
    let claude_rules = args.contains(&EMIT_CLAUDE_RULES_FLAG);
    if args.contains(&EMIT_CHECK_FLAG) {
        return emit_check(ws, &lock, claude_rules);
    }
    emit_write(ws, &lock, claude_rules)
}

/// Render every region in place, write both transpose indexes, ensure the CLAUDE.md
/// shim, and (when `claude_rules`) render `.claude/rules/*.md`. `emit` rewrites strictly
/// between markers and under `.stele/`/`.claude/` — never the marker lines or authored
/// prose (§5.1).
fn emit_write(ws: &Workspace, lock: &Lock, claude_rules: bool) -> Result<CommandOutput> {
    let root = &ws.home;
    // An id collision or a hand-authored target in the rules projection is an input error
    // before the first write.
    let rules = claude_rules
        .then(|| render_claude_rules(lock))
        .transpose()?;
    for name in rules.iter().flat_map(BTreeMap::keys) {
        refuse_foreign_rule(root, name)?;
    }
    // Every region renders and validates before the first write: no half-written tree.
    let mut writes = Vec::new();
    for node in lock.nodes.values() {
        let path = node_agents_path(ws, node);
        let contents = read_node_agents(root, &path)?;
        let region = require_region(&path, &contents)?;
        let rendered = emit::render_region(lock, node);
        let head = &contents[..region.content_start];
        let updated = format!(
            "{head}{}{rendered}{}",
            region_lead(head, &rendered),
            &contents[region.content_end..]
        );
        if updated != contents {
            let target = in_tree_target(root, &path).ok_or_else(|| {
                SteleError::input(
                    &path,
                    1,
                    "node AGENTS.md links outside the work tree; refusing to write through it",
                )
            })?;
            writes.push((root.join(target), updated));
        }
    }
    for (target, updated) in &writes {
        write_atomic(target, updated, None)?;
    }

    for (name, content) in indexes(lock) {
        write_under_root(root, INDEX_DIR, name, &content)?;
    }
    ensure_shim(ws, lock)?;
    if let Some(rules) = rules {
        write_claude_rules(root, &rules)?;
    }

    let regions = lock.nodes.len();
    Ok(CommandOutput::new(
        json!({ "regions": regions, "indexes": 2 }),
        Some(format!(
            "stele emit: rendered {regions} region(s) and 2 index file(s)"
        )),
    ))
}

/// The two transpose indexes (§6.1) as `(file name under INDEX_DIR, rendered content)`.
fn indexes(lock: &Lock) -> [(&'static str, String); 2] {
    [
        (INVARIANTS_INDEX, emit::render_invariants_index(lock)),
        (HAZARDS_INDEX, emit::render_hazards_index(lock)),
    ]
}

/// The LF that terminates the begin marker's line before `emit` fills a region for the
/// first time (§3.1/§7). The one-line empty form `<!-- @stele --><!-- @end -->` that
/// `init` scaffolds owns no bytes and no newline, so filling it with non-empty content
/// must close the marker line — otherwise the first `emit` would render a form no later
/// `emit` reproduces, and `emit --check` would diverge forever. Empty content (a
/// childless component's region) leaves the one-line form byte-identical.
fn region_lead(head: &str, rendered: &str) -> &'static str {
    if rendered.is_empty() || head.ends_with('\n') {
        ""
    } else {
        "\n"
    }
}

/// Render to memory and byte-diff against on-disk regions and index files (§3.1) — and,
/// when `claude_rules`, the `.claude/rules/*.md` projection, a stale marker-stamped file
/// included; any divergence is an assertion failure (exit 1) naming each divergent file.
/// Missing regions and malformed markers remain input errors (exit 2), same as `emit`.
fn emit_check(ws: &Workspace, lock: &Lock, claude_rules: bool) -> Result<CommandOutput> {
    let root = &ws.home;
    let mut divergent: Vec<String> = Vec::new();
    for node in lock.nodes.values() {
        let path = node_agents_path(ws, node);
        let contents = read_node_agents(root, &path)?;
        let region = require_region(&path, &contents)?;
        if contents[region.content_start..region.content_end] != *emit::render_region(lock, node) {
            divergent.push(path.display().to_string());
        }
    }
    for (name, content) in indexes(lock) {
        check_file(root, INDEX_DIR, name, &content, &mut divergent);
    }
    if claude_rules {
        let rules = render_claude_rules(lock)?;
        for (name, (_, content)) in &rules {
            check_file(root, CLAUDE_RULES_DIR, name, content, &mut divergent);
        }
        divergent.extend(
            stale_claude_rules(root, &rules)
                .iter()
                .map(|name| format!("{CLAUDE_RULES_DIR}/{name}")),
        );
    }

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

/// A node's AGENTS.md path — the lock's recorded `source` (never derived from the id, which
/// an `id:` override decouples, §2.1), mode-aware (§3.5). Normal mode returns the
/// home-relative source path (`emit` joins it onto `ws.home == "."`). Undercover mode
/// returns the absolute overlay path `<home>/.stele/tree/<source>`; `emit`'s later
/// `ws.home`-join is a no-op on an absolute path.
fn node_agents_path(ws: &Workspace, node: &LockNode) -> PathBuf {
    match ws.mode {
        Mode::Normal => PathBuf::from(&node.source),
        Mode::Undercover => ws.home.join(TREE_DIR).join(&node.source),
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

/// Read `path`: `None` when it is absent, any other IO failure a §5.3 internal error (exit 3)
/// naming `shown`. Each caller decides what absence means.
fn read_if_present(path: &Path, shown: &dyn std::fmt::Display) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        read => read
            .map(Some)
            .map_err(|e| SteleError::internal(format!("read {shown}: {e}"))),
    }
}

/// Read a node's AGENTS.md: a file gone since the build is a stale lock (exit 2), any
/// other IO failure exit 3.
fn read_node_agents(root: &Path, path: &Path) -> Result<String> {
    read_if_present(&root.join(path), &path.display())?.ok_or_else(|| {
        SteleError::input_msg(format!(
            "lock names {} but it is gone; {RUN_BUILD_HINT}",
            path.display()
        ))
    })
}

/// Locate a node's generated region, mapping its absence to the §3.1 exit-2 error that
/// points the user at `stele init` (`emit` never scaffolds the region itself).
fn require_region(path: &Path, contents: &str) -> Result<parse::Region> {
    parse::find_region(path, contents)?.ok_or_else(|| {
        SteleError::input(
            path,
            1,
            "node AGENTS.md has no generated region (`<!-- @stele -->` … `<!-- @end -->`); \
             run stele init to scaffold it (§3.1)",
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

/// Ensure the harness shims exist when the graph declares a system node (§3.3/§3.5),
/// mode-aware. Normal: `CLAUDE.md` = `@AGENTS.md` at the graph home (today's behavior,
/// byte-identical) AND one beside every nested node's AGENTS.md — Claude Code loads no
/// AGENTS.md natively at any level, so without the nested shim a node file is invisible
/// (verified empirically 2026-09-16, 2.1.273). Undercover: `CLAUDE.local.md` at the invoking
/// WORK-tree root, a single relative `@`-import of the overlay root node (§3.5) — exactly one
/// materialized file, so no nested shims (nothing is ever written under `.stele/tree/`).
/// All are only-if-absent — an existing shim is the operator's (or team's) and is never
/// overwritten, even when it differs; a symlink there, even dangling, counts as existing,
/// so a repo-planted link never steers the write outside the tree. This is ensure-on-write
/// only; `emit --check` does NOT diff any shim (a shim is not a generated region — see
/// [`emit_check`]).
fn ensure_shim(ws: &Workspace, lock: &Lock) -> Result<()> {
    if !lock.nodes.contains_key(crate::model::SYSTEM_ID) {
        return Ok(());
    }
    match ws.mode {
        Mode::Normal => {
            let path = ws.home.join(CLAUDE_SHIM);
            if path.symlink_metadata().is_err() {
                std::fs::write(&path, CLAUDE_SHIM_CONTENT)
                    .map_err(|e| SteleError::internal(format!("write {CLAUDE_SHIM}: {e}")))?;
            }
            ensure_nested_shims(ws, lock)
        }
        Mode::Undercover => {
            // The shim belongs at the invoking work-tree root (`ws.work`, §3.5), where the
            // root-anchored `/CLAUDE.local.md` exclude entry matches.
            let path = ws.work.join(CLAUDE_LOCAL_SHIM);
            if path.symlink_metadata().is_ok() {
                return Ok(());
            }
            let import = shim_import_path(&ws.work, &ws.home);
            let content = format!("@{}\n", import.to_string_lossy().replace('\\', "/"));
            std::fs::write(&path, content)
                .map_err(|e| SteleError::internal(format!("write {CLAUDE_LOCAL_SHIM}: {e}")))
        }
    }
}

/// Ensure a `CLAUDE.md` = `@AGENTS.md` shim beside every NESTED node's AGENTS.md (§3.3),
/// normal mode only. The root node's shim is the one [`ensure_shim`] just wrote, so the
/// only-if-absent guard skips it. Only-if-absent, exactly like the root shim: a directory
/// that already carries a `CLAUDE.md` keeps the team's file byte-for-byte, and
/// `emit --check` never diffs these.
fn ensure_nested_shims(ws: &Workspace, lock: &Lock) -> Result<()> {
    for node in lock.nodes.values() {
        let path = ws.home.join(&node.source).with_file_name(CLAUDE_SHIM);
        if path.symlink_metadata().is_ok() {
            continue;
        }
        std::fs::write(&path, CLAUDE_SHIM_CONTENT)
            .map_err(|e| SteleError::internal(format!("write {}: {e}", path.display())))?;
    }
    Ok(())
}

/// The relative path from the canonicalized work-tree root to the overlay root node file
/// `<home>/.stele/tree/AGENTS.md` (§3.5), for the undercover shim's `@`-import. Both roots
/// are canonicalized (symlinks resolved on each, per §3.5) before comparison; the path is
/// then the common canonical prefix stripped, one `..` per remaining work component, the
/// remaining home components, and the overlay tail. Same checkout → `.stele/tree/AGENTS.md`;
/// a work tree one level below the home → `../.stele/tree/AGENTS.md`; a sibling worktree →
/// `../<home-dir>/.stele/tree/AGENTS.md`.
fn shim_import_path(work: &Path, home: &Path) -> PathBuf {
    let canon_work = work.canonicalize().unwrap_or_else(|_| work.to_path_buf());
    let canon_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let work_comps: Vec<_> = canon_work.components().collect();
    let home_comps: Vec<_> = canon_home.components().collect();
    let common = work_comps
        .iter()
        .zip(&home_comps)
        .take_while(|(a, b)| a == b)
        .count();
    let mut rel = PathBuf::new();
    for _ in common..work_comps.len() {
        rel.push("..");
    }
    for comp in &home_comps[common..] {
        rel.push(comp.as_os_str());
    }
    rel.push(TREE_DIR);
    rel.push("AGENTS.md");
    rel
}

/// Remove every marker-stamped `.claude/rules/` file no node renders any more, then render
/// one `.claude/rules/<slug>.md` per node that declares claims (§3.3, opt-in). Each write is gated on the
/// stele marker: a target that is absent or already opens with [`emit::CLAUDE_RULE_MARKER`]
/// is (re)generated; a foreign file (a hand-authored rule with no marker) is NEVER clobbered
/// nor pruned — clobbering it is an exit-2 input error naming the file.
fn write_claude_rules(root: &Path, rules: &BTreeMap<String, (&str, String)>) -> Result<()> {
    // Prune first: on a case-folding filesystem a write keeps an existing entry's case, so
    // after a case-only rename (`Billing` → `billing`) a later prune of `Billing.md` would
    // delete the rule just written.
    for name in stale_claude_rules(root, rules) {
        std::fs::remove_file(root.join(CLAUDE_RULES_DIR).join(&name))
            .map_err(|e| SteleError::internal(format!("remove {CLAUDE_RULES_DIR}/{name}: {e}")))?;
    }
    for (name, (_, content)) in rules {
        write_under_root(root, CLAUDE_RULES_DIR, name, content)?;
    }
    Ok(())
}

/// The `.claude/rules/` projection (§3.3): filename → (node id, content) for every node that
/// declares claims. [`emit::rule_slug`] flattens `/`, so two ids can share a filename (`a/b`
/// and `a-b`; `root` and the system node), and a case-folding filesystem (APFS, NTFS) makes
/// `A-b` and `a-b` one file too — an exit-2 input error naming both, never a silent
/// overwrite.
fn render_claude_rules(lock: &Lock) -> Result<BTreeMap<String, (&str, String)>> {
    let mut rules: BTreeMap<String, (&str, String)> = BTreeMap::new();
    for (id, node) in lock.nodes.iter().filter(|(_, n)| !n.claims.is_empty()) {
        let name = format!("{}.md", emit::rule_slug(id));
        let rule = (id.as_str(), emit::render_claude_rule(id, node));
        let folded = name.to_lowercase();
        if let Some((_, (other, _))) = rules.iter().find(|(n, _)| n.to_lowercase() == folded) {
            return Err(SteleError::input_msg(format!(
                "node ids `{other}` and `{id}` both map to {CLAUDE_RULES_DIR}/{name}; rename one"
            )));
        }
        rules.insert(name, rule);
    }
    Ok(rules)
}

/// The marker-stamped `.claude/rules/` files (sorted) the projection no longer renders —
/// a node removed or left claim-free. Foreign files are never stale.
fn stale_claude_rules(root: &Path, rules: &BTreeMap<String, (&str, String)>) -> BTreeSet<String> {
    let dir = root.join(CLAUDE_RULES_DIR);
    std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            !rules.contains_key(name)
                && std::fs::read_to_string(dir.join(name))
                    .is_ok_and(|s| s.starts_with(emit::CLAUDE_RULE_MARKER))
        })
        .collect()
}

/// Guard one generated `.claude/rules/<name>` (§3.3) before emit's first write. When the
/// target exists and does NOT open with [`emit::CLAUDE_RULE_MARKER`] it is a foreign file —
/// refuse with an exit-2 input error naming it (never overwrite a team's rule); an absent or
/// marker-carrying target may be (re)generated.
fn refuse_foreign_rule(root: &Path, name: &str) -> Result<()> {
    let path = root.join(CLAUDE_RULES_DIR).join(name);
    // Bytes, not text: a non-UTF-8 rule is foreign, never mistaken for an absent one.
    if let Ok(existing) = std::fs::read(&path)
        && !existing.starts_with(emit::CLAUDE_RULE_MARKER.as_bytes())
    {
        return Err(SteleError::input(
            format!("{CLAUDE_RULES_DIR}/{name}"),
            1,
            "refusing to overwrite a hand-authored .claude/rules file (no stele marker line); \
             move it aside or delete it, then re-run stele emit --claude-rules",
        ));
    }
    Ok(())
}

/// Byte-diff one on-disk generated file against its freshly-rendered content; a missing or
/// divergent file is recorded for the `emit --check` failure list.
fn check_file(root: &Path, dir: &str, name: &str, expected: &str, divergent: &mut Vec<String>) {
    let path = root.join(dir).join(name);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(expected) {
        divergent.push(format!("{dir}/{name}"));
    }
}

// ─── the read/query surface (§5.1, §5.3) ─────────────────────────────────────
//
// Every verb below reads the COMMITTED lock and never rebuilds (§5.3): a missing
// lock is exit 2 "run stele build", they trust the lock's freshness (staleness is
// `check`'s job). None writes anything.

/// Reject a committed lock this engine cannot read (§3.2/§5.3). A version-1 lock is
/// the pre-0.3.0 `stele:` grammar, so its fix line names `migrate` before `build`
/// (there is no dual-read); a newer version needs a newer engine (a rebuild would
/// downgrade it); any other older version keeps the plain rebuild hint.
fn require_lock_version(on_disk: &str) -> Result<()> {
    let version = lock::read_version(on_disk)?;
    if version == lock::LOCK_VERSION {
        return Ok(());
    }
    let fix = match version {
        lock::LEGACY_LOCK_VERSION => lock::LEGACY_LOCK_FIX,
        v if v > lock::LOCK_VERSION => "upgrade stele to an engine that reads it",
        _ => RUN_BUILD_HINT,
    };
    Err(SteleError::input_msg(format!(
        "committed lock is version {version}; this engine writes version {}. {fix}",
        lock::LOCK_VERSION
    )))
}

/// Load + version-check + strict-parse the committed lock (§5.3), the shared front
/// door for the read/query verbs. A missing lock → exit 2 "run stele build"; an
/// unknown `version` → exit 2 (never best-effort parsed, §3.2).
fn load_committed(root: &Path) -> Result<Lock> {
    let on_disk = read_committed_lock(root)?;
    require_lock_version(&on_disk)?;
    lock::parse_lock(&on_disk)
}

/// `stele root` (§6): the initialContext as text — the six items in order. Items 1–2
/// (identity, commands) render from the system node here; items 3–6 (hazards, router,
/// indexes, engine) reuse the `emit` region renderer so `root` and the materialized
/// root AGENTS.md never diverge.
fn cmd_root(ws: &Workspace, _: &[&str]) -> Result<CommandOutput> {
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
fn invariants(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
    // A typo is a §5.3 bad flag, never an empty listing.
    if let Some(k) = kind.filter(|k| !["component", "container", "system"].contains(k)) {
        return Err(SteleError::input_msg(format!(
            "unknown --kind {k:?}; expected one of component|container|system"
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

// ─── init: the adoption scaffold (§7) ────────────────────────────────────────

/// The empty generated region `init` writes (§3.1/§7): the bare one-line form, with no
/// name and no body between the markers, for `emit` to fill later.
const EMPTY_REGION: &str = "<!-- @stele --><!-- @end -->\n";

/// The `init --undercover` flag (§3.5): scaffold a private overlay graph instead of the
/// in-tree one. The one extra token `init` accepts; inert on every other verb.
const UNDERCOVER_FLAG: &str = "--undercover";

/// The marker-fenced managed block `init --undercover` maintains in `<common-dir>/info/exclude`
/// (§3.5): its begin/end lines bracket the two unconditional exclude entries so the overlay
/// and the shim never surface in `git status`. Rewritten in place, never touching a line
/// outside the fence.
const EXCLUDE_BEGIN: &str = "# stele:begin undercover";
const EXCLUDE_END: &str = "# stele:end undercover";
const EXCLUDE_ENTRIES: [&str; 2] = ["/.stele/", "/CLAUDE.local.md"];

/// The one-line, timestamp-free content of the `.stele/undercover` marker (§3.5) — a
/// deterministic provenance stamp, so re-running `init --undercover` rewrites it byte-identically.
const UNDERCOVER_MARKER_CONTENT: &str = "stele undercover\n";

/// `stele init` (§7): scan the tracked tree, propose node boundaries, and scaffold a
/// skeleton `stele` block + empty generated region for each proposed node that has no
/// stele block yet. Proposals: the repo root (system) plus each top-level directory
/// that holds tracked files, is not the ADR directory, and is not already covered by an
/// existing node (component/import-cluster proposals are out of scope for v1). A file
/// that already carries a stele block — tracked or not — is left byte-identical (the §7
/// idempotency promise); a prose-only file gets the skeleton prepended and its prose preserved.
/// A proposed target that a `.gitignore` matches is skipped with a stderr notice (never
/// scaffolded into an untrackable path). All writes happen first; then the whole set is
/// staged in ONE `git add` batch so the next `stele build` (tracked-files scan, §2.4)
/// sees them, together with any untouched block-carrying target still untracked (an earlier
/// scaffold whose stage failed) — a staging failure is a warning plus a `git add` instruction
/// (and a summary that says unstaged), never a post-write abort; nothing to stage, no
/// claim. `init` never deletes content, never writes inside a generated region, and never
/// writes the lock.
fn init(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
    // `--undercover` (§3.5) forks init to the overlay scaffold; without it, normal init is
    // byte-identical to today.
    if args.contains(&UNDERCOVER_FLAG) {
        return init_undercover(ws);
    }
    // Normal-mode init scaffolds and stages the WORK tree in place (§7); the git spawns,
    // the tracked reads, and the source writes all target the invoking checkout. (Phase 3
    // forks `init --undercover` to scaffold the `<home>/.stele/tree/` overlay instead, with
    // no `git add`.)
    let work = &ws.work;
    let tracked = tracked_files(work)?;
    let adr_dir = detect_adr_dir(&tracked);
    let existing = existing_node_dirs(work, &tracked)?;

    let mut written: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    let mut scaffold = |path: &str, kind| scaffold_node(work, path, kind, &mut written);
    if !existing.contains("") && !skip_if_ignored(work, "AGENTS.md")? {
        kept.extend(scaffold("AGENTS.md", NodeKind::System)?);
    }
    for dir in proposable_top_dirs(&tracked, &existing, &adr_dir) {
        let path = format!("{dir}/AGENTS.md");
        if skip_if_ignored(work, &path)? {
            continue;
        }
        kept.extend(scaffold(&path, NodeKind::Container)?);
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
    let stage: Vec<String> = kept
        .iter()
        .filter(|t| !tracked.contains(t))
        .map(|t| t.to_string_lossy().replace('\\', "/"))
        .chain(written.iter().cloned())
        .collect();
    let staged = match (!stage.is_empty()).then(|| git_add(work, &stage)) {
        Some(Some(warning)) => {
            eprintln!("{warning}");
            ", unstaged (git add failed)"
        }
        Some(None) => ", staged for the next stele build",
        None => "",
    };
    Ok(CommandOutput::new(
        json!({ "wrote": written }),
        Some(format!(
            "stele init: scaffolded {} node(s){staged}",
            written.len()
        )),
    ))
}

/// `stele init --undercover` (§3.5): scaffold a private overlay graph rooted at the parent
/// of the git common dir, never touching the tracked tree. The steps run in a fixed order —
/// (1) resolve the home directly from the common dir (the marker does not exist yet, so
/// `ws.mode` is Normal at entry); (2) refuse a submodule checkout (home inside `.git`);
/// (3) reject mutual exclusion with a tracked shared graph BEFORE any write; (4) install the
/// `info/exclude` managed block FIRST, before any artifact exists, so a failed exclude write
/// aborts init having created nothing (no leak window); (5) write the deterministic marker;
/// (6) scaffold the overlay root system node plus one container per proposable top dir, leaving
/// any existing overlay file untouched. No `git add`, no `git check-ignore` filter — the overlay
/// is never tracked, so `git status` stays clean. Re-running is idempotent (exit 0).
fn init_undercover(ws: &Workspace) -> Result<CommandOutput> {
    let work = &ws.work;
    // (1) Home = parent of the canonicalized git common dir (as [`resolve_workspace`] resolves
    // it); a non-repo cwd is the same exit-2 input error the tracked scan would give.
    let (common, home) = &graph_home(work).ok_or_else(|| {
        SteleError::input_msg(
            "not a git repository (stele undercover roots its private graph at the git common dir)",
        )
    })?;

    // (2) Submodule refusal (§3.5): in a submodule the git common dir is
    // `<superproject>/.git/modules/<name>`, so the home would fall INSIDE the superproject's
    // `.git` — the overlay and marker would land in version-control internals. A `.git` component
    // anywhere in the resolved home covers every submodule shape → exit 2, no write. (v2 concern.)
    if home.components().any(|c| c.as_os_str() == ".git") {
        return Err(SteleError::input_msg(
            "undercover mode does not support submodule checkouts (the git common dir lives \
             inside the superproject)",
        ));
    }

    let tracked = tracked_files(work)?;
    // (3) Mutual exclusion (§3.5): a tracked stele-block AGENTS.md or a tracked `.stele/` path
    // means the repo already carries a shared committed graph — refuse before writing a byte.
    reject_shared_graph_conflict(work, &tracked)?;

    // (4) Exclude block FIRST — before any artifact under `<home>/.stele/` exists (§3.5). Install
    // it before the marker so that if `info/exclude` cannot be written (a read-only `.git/info`),
    // init aborts having created nothing: no marker, no overlay, nothing to leak into `git status`.
    // Gitignore entries for not-yet-existent paths are harmless. This is the ordering guarantee.
    ensure_info_exclude(common)?;

    // (5) Marker — its presence is what selects undercover mode on every later invocation.
    // `<home>/.stele/undercover` (= UNDERCOVER_MARKER), creating `.stele/`.
    write_under_root(home, LOCK_DIR, "undercover", UNDERCOVER_MARKER_CONTENT)?;

    // (6) Overlay scaffold: reuse the normal-init proposal set verbatim against the tracked
    // roster. Post-guard `existing` is empty (no tracked node), so every top dir is proposed.
    let adr_dir = detect_adr_dir(&tracked);
    let existing = existing_node_dirs(work, &tracked)?;
    let overlay_root = home.join(TREE_DIR);
    let mut written: Vec<String> = Vec::new();
    scaffold_overlay_node(&overlay_root, "AGENTS.md", NodeKind::System, &mut written)?;
    for dir in proposable_top_dirs(&tracked, &existing, &adr_dir) {
        let path = format!("{dir}/AGENTS.md");
        scaffold_overlay_node(&overlay_root, &path, NodeKind::Container, &mut written)?;
    }

    Ok(CommandOutput::new(
        json!({ "wrote": written }),
        Some(format!(
            "stele init --undercover: scaffolded {} node(s)",
            written.len()
        )),
    ))
}

/// The mutual-exclusion guard for `init --undercover` (§3.5): a repo already carrying a
/// shared, committed graph — any tracked `AGENTS.md` with a stele block, or any tracked path
/// under `.stele/` — cannot also run undercover (mixing is a v2 concern). Exit 2, no write.
fn reject_shared_graph_conflict(work: &Path, tracked: &[PathBuf]) -> Result<()> {
    let has_tracked_node = !existing_node_dirs(work, tracked)?.is_empty();
    let has_tracked_stele = tracked.iter().any(|rel| {
        let posix = rel.to_string_lossy().replace('\\', "/");
        posix == LOCK_DIR || posix.starts_with(&format!("{LOCK_DIR}/"))
    });
    if has_tracked_node || has_tracked_stele {
        return Err(SteleError::input_msg(
            "repo already carries a shared stele graph; undercover mode cannot coexist \
             (mixing is v2)",
        ));
    }
    Ok(())
}

/// Scaffold one overlay node (§3.5) under `overlay_root`, leaving any existing overlay file
/// untouched (the §7 idempotency/non-destructive discipline: a re-run finds every file present
/// and rewrites nothing). An absent target takes [`scaffold_node`]'s new-file path.
fn scaffold_overlay_node(
    overlay_root: &Path,
    rel_path: &str,
    kind: NodeKind,
    written: &mut Vec<String>,
) -> Result<()> {
    if overlay_root.join(rel_path).exists() {
        return Ok(());
    }
    scaffold_node(overlay_root, rel_path, kind, written).map(drop)
}

/// Install the marker-fenced managed block into `<common-dir>/info/exclude` (§3.5), creating
/// `info/` and the file if absent. Idempotent replace-in-place: an existing fence has ONLY its
/// bracketed lines rewritten; otherwise the block is appended (with a separating newline when
/// the file is non-empty and unterminated). Lines outside the fence are never touched, save a
/// stray fence line ([`rewrite_exclude_block`]).
fn ensure_info_exclude(common: &Path) -> Result<()> {
    let dir = common.join("info");
    std::fs::create_dir_all(&dir)
        .map_err(|e| SteleError::internal(format!("create {}: {e}", dir.display())))?;
    let path = dir.join("exclude");
    let existing = read_if_present(&path, &path.display())?.unwrap_or_default();
    let updated = rewrite_exclude_block(&existing);
    if updated != existing {
        // Atomic (§3.5): an interrupted write never leaves a half-rewritten exclude (which
        // could unhide the overlay). The temp lives inside `.git/info/`, never the work tree,
        // so it cannot itself surface in `git status`.
        write_atomic(&path, &updated, None)?;
    }
    Ok(())
}

/// Replace `path` via a unique sibling temp file + rename (§3.5/§5.3 atomicity): an
/// interrupted or partial write never replaces the live file — never a partial lock or a
/// half-rewritten exclude on disk. The temp name (`.<name>.tmp.<pid>.<seq>`) carries the pid
/// and a process-local counter so concurrent writers (two builds racing on the lock) never
/// collide, and it is cleaned up best-effort if the rename fails. Shared by the lock, the
/// `info/exclude` install, and `migrate`'s/`emit`'s source rewrites. The lock passes its
/// `lock_home`: its messages name paths relative to it (`.stele/graph.lock`), and it lands as
/// a fresh file — a build never inherits the old lock's mode. Every other target's
/// permissions are carried onto the replacement (best-effort), so rewriting a source in
/// place never strips an executable bit.
fn write_atomic(path: &Path, bytes: &str, lock_home: Option<&Path>) -> Result<()> {
    static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!(".{name}.tmp.{}.{seq}", std::process::id()));
    let shown = |p: &Path| {
        let rel = lock_home.and_then(|home| p.strip_prefix(home).ok());
        rel.unwrap_or(p).display().to_string()
    };
    std::fs::write(&temp, bytes)
        .map_err(|e| SteleError::internal(format!("write {}: {e}", shown(&temp))))?;
    if lock_home.is_none()
        && let Ok(existing) = std::fs::metadata(path)
    {
        let _ = std::fs::set_permissions(&temp, existing.permissions());
    }
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        SteleError::internal(format!("rename into {}: {e}", shown(path)))
    })
}

/// The `info/exclude` content with the managed block (§3.5) present exactly once. The fence
/// is the first begin line paired with the first end line after it: that inclusive span is
/// replaced in place; a stray fence line outside it is dropped, and with no pair the block is
/// appended after a newline separator — a half-deleted fence heals in one run, and every
/// non-fence line outside the pair is byte-preserved. The block itself is the two begin/end
/// fence lines bracketing [`EXCLUDE_ENTRIES`].
fn rewrite_exclude_block(existing: &str) -> String {
    let block = format!(
        "{EXCLUDE_BEGIN}\n{}\n{EXCLUDE_END}\n",
        EXCLUDE_ENTRIES.join("\n")
    );
    let lines: Vec<&str> = existing.split_inclusive('\n').collect();
    let find =
        |fence: &str, from: usize| (from..lines.len()).find(|&i| lines[i].trim_end() == fence);
    let pair = find(EXCLUDE_BEGIN, 0).and_then(|b| Some((b, find(EXCLUDE_END, b)?)));
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        match pair {
            Some((b, _)) if i == b => out.push_str(&block),
            Some((b, e)) if (b..=e).contains(&i) => {}
            _ if [EXCLUDE_BEGIN, EXCLUDE_END].contains(&line.trim_end()) => {}
            _ => out.push_str(line),
        }
    }
    if pair.is_none() {
        out.push_str(append_separator(&out));
        out.push_str(&block);
    }
    out
}

/// The newline that terminates a non-empty, unterminated `contents` before more is appended.
fn append_separator(contents: &str) -> &'static str {
    if contents.is_empty() || contents.ends_with('\n') {
        ""
    } else {
        "\n"
    }
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
/// stele block but no region (§3.1), at an in-tree symlink's target. A non-AGENTS.md file,
/// a blockless file (nothing to route), a symlink out of the tree (never written through),
/// or a file that already has a region is left byte-identical. Records the path for staging so the next `stele build` sees the change.
fn ensure_region_for_block(root: &Path, rel: &Path, written: &mut Vec<String>) -> Result<()> {
    if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
        return Ok(());
    }
    let Some(rel) = &in_tree_target(root, rel) else {
        return Ok(());
    };
    let contents = read_node_source(&root.join(rel), rel)?;
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
    format!("{contents}{}\n{EMPTY_REGION}", append_separator(contents))
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
        let contents = read_node_source(&root.join(rel), rel)?;
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
/// block + empty region. A file already carrying a block is left untouched. Prose-only
/// file → prepend the skeleton (after any leading `#` heading), preserve every existing
/// byte, and append an empty region if the file has none. An in-tree symlink is written at its target; one out of the tree is left as is.
/// Records the written path for staging; returns the untouched block-carrying target.
fn scaffold_node(
    root: &Path,
    rel_path: &str,
    kind: NodeKind,
    written: &mut Vec<String>,
) -> Result<Option<PathBuf>> {
    let Some(target) = in_tree_target(root, Path::new(rel_path)) else {
        return Ok(None);
    };
    let full = root.join(&target);
    let contents = match read_if_present(&full, &rel_path)? {
        // A block already there (an untracked earlier scaffold included) is §7's untouched file.
        Some(existing) if !matches!(parse::parse_agents_file(&target, &existing), Ok(None)) => {
            return Ok(Some(target));
        }
        Some(existing) => prepend_skeleton(rel_path, &existing, kind)?,
        None => new_skeleton_file(rel_path, root, kind),
    };
    // `full` is `root` joined with a file path, so it always has a parent.
    std::fs::create_dir_all(full.parent().unwrap_or(root))
        .map_err(|e| SteleError::internal(format!("create dir for {rel_path}: {e}")))?;
    std::fs::write(&full, contents)
        .map_err(|e| SteleError::internal(format!("write {rel_path}: {e}")))?;
    written.push(target.to_string_lossy().replace('\\', "/"));
    Ok(None)
}

/// Where a write to `rel` (under `root`) lands: `rel` itself, or — for a symlink — its
/// resolved target relative to `root`. `None` when the link dangles or leaves the tree,
/// which is never written through.
fn in_tree_target(root: &Path, rel: &Path) -> Option<PathBuf> {
    let path = root.join(rel);
    if !path.is_symlink() {
        return Some(rel.to_path_buf());
    }
    let target = path.canonicalize().ok()?;
    Some(
        target
            .strip_prefix(root.canonicalize().ok()?)
            .ok()?
            .to_path_buf(),
    )
}

/// A brand-new node AGENTS.md (§7): `# <name>` heading, the skeleton block, then an
/// empty generated region.
fn new_skeleton_file(rel_path: &str, root: &Path, kind: NodeKind) -> String {
    let name = node_display_name(rel_path, root);
    format!("# {name}\n\n{}\n{EMPTY_REGION}", skeleton_block(kind))
}

/// Prepend the skeleton block to a prose-only file (§7): keep a leading `# heading`
/// first, then the block, then every existing byte of prose, then an empty region if
/// the file carries none. Preserves all prior content.
fn prepend_skeleton(rel_path: &str, prose: &str, kind: NodeKind) -> Result<String> {
    let (heading, body) = split_leading_heading(prose);
    let mut out = String::new();
    if let Some(heading) = heading {
        out.push_str(heading);
        out.push_str("\n\n");
    }
    out.push_str(&skeleton_block(kind));
    out.push('\n');
    out.push_str(body);
    // Append an empty region only when the (now-combined) file has none.
    Ok(match parse::find_region(Path::new(rel_path), &out)? {
        Some(_) => out,
        None => append_empty_region(&out),
    })
}

/// Split a leading `# heading` line off the front of a file: `(Some(heading), rest)`
/// when the first non-empty line is an ATX heading, else `(None, whole)`. The rest
/// keeps its bytes verbatim (leading blank lines after the heading are dropped from
/// `rest` only to the extent the reassembler re-adds a single separator).
fn split_leading_heading(prose: &str) -> (Option<&str>, &str) {
    let trimmed = prose.trim_start_matches(['\n', '\r']);
    // A single line is a heading with no body, or bare prose.
    let (first, rest) = trimmed.split_once('\n').unwrap_or((trimmed, ""));
    if first.starts_with('#') {
        (
            Some(first.trim_end()),
            rest.trim_start_matches(['\n', '\r']),
        )
    } else {
        (None, prose)
    }
}

/// The skeleton `stele` block (§7): the required `kind` active, every other typed field
/// present but commented out so nothing is auto-filled (purpose/invariants stay the
/// human's to write, §7). The `decided_by` hint is the §2.6 decision token.
fn skeleton_block(kind: NodeKind) -> String {
    format!(
        "```stele\nkind: {}\n# purpose:            # \u{2264}{PURPOSE_MAX_CHARS}-char scent \u{2014} fill it in (never auto-generated, \u{00A7}7)\n# commands: {{}}\n# invariants: []\n# hazards: []\n# edges:\n#   depends: []\n#   decided_by: [{DECISION_PREFIX}0001]\n# budget:\n```\n",
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
/// failure mode is already gone. Returns that warning; `None` when the paths staged.
fn git_add(root: &Path, paths: &[String]) -> Option<String> {
    let why = match Command::new("git")
        .arg("add")
        .arg("--")
        .args(paths)
        .current_dir(root)
        .output()
    {
        Ok(output) if output.status.success() => return None,
        Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_string(),
        Err(e) => format!("could not run git: {e}"),
    };
    Some(format!(
        "stele init: `git add` failed: {why}\n  stage the scaffold yourself so stele build sees it: git add -- {}",
        paths.join(" ")
    ))
}

// ─── `stele migrate` (§5.1) ──────────────────────────────────────────────────

/// One file `migrate` rewrites: where its bytes live, how it is displayed in the report,
/// and which notation slots it can carry ([`legacy::SourceKind`]).
struct MigrateTarget {
    display: String,
    kind: legacy::SourceKind,
    path: PathBuf,
}

/// `stele migrate` (§5.1/§5.3): rewrite the pre-0.3.0 `stele:` notation to the glyph
/// notation (§2.5) IN PLACE, over `build`'s scan scope — VCS-tracked files,
/// `.steleignore`-filtered, `.stele/` excluded, plus the overlay under
/// `<home>/.stele/tree/` in undercover mode (§3.5). It reads and writes no lock: `stele
/// build` follows it, and until it does the v1 lock keeps failing `check`/`emit` (§3.2).
///
/// Process contract (§5.3): exit `0` whether or not anything was rewritten (a no-op run
/// is success — the rewrite is idempotent), `2` on an input error with NOTHING written
/// (every source is read and rewritten in memory before the first byte lands), `3` on an
/// IO failure; never `1` — `migrate` asserts nothing. A dirty work tree is a WARNING, not
/// a refusal: the rewrite is meant to be reviewed as a diff.
fn migrate(ws: &Workspace, _: &[&str]) -> Result<CommandOutput> {
    if ws.mode == Mode::Undercover {
        // Self-heal (§3.5) like every other undercover verb: re-ensure the `info/exclude`
        // block so the private graph stays out of `git status`. The managed block's own
        // `# stele:begin undercover` fence is NOT notation (§2.5/§3.5) and migrate never
        // touches it — it lives in the git common dir, outside every scan scope.
        ensure_info_exclude(&ws.common)?;
    }
    warn_if_dirty(&ws.work);

    // Pass 1 — read and rewrite every source in memory. A read or decode failure aborts
    // here, before a single file has been replaced (§5.3: nothing written on an error).
    let mut rewritten: Vec<(PathBuf, String, String)> = Vec::new();
    for target in migrate_targets(ws)? {
        let Some(contents) = read_migrate_source(&target)? else {
            continue;
        };
        if let Some(migrated) = legacy::rewrite(target.kind, &contents) {
            rewritten.push((target.path, target.display, migrated));
        }
    }

    // Pass 2 — land them, each through the temp-file + rename discipline (§5.3), so an
    // interrupted run never leaves a half-written source.
    for (path, _, contents) in &rewritten {
        write_atomic(path, contents, None)?;
    }

    let files: Vec<String> = rewritten
        .into_iter()
        .map(|(_, display, _)| display)
        .collect();
    let summary = if files.is_empty() {
        "stele migrate: nothing to rewrite — already the glyph notation (0 file(s))".to_string()
    } else {
        let mut summary = format!("stele migrate: rewrote {} file(s)", files.len());
        for display in &files {
            summary.push_str("\n  ");
            summary.push_str(display);
        }
        summary
    };
    Ok(CommandOutput::new(
        json!({ "files": files.len(), "rewrote": files }),
        Some(summary),
    ))
}

/// Every file `migrate` reads, in scan order (§5.1 scope). Normal mode: the tracked,
/// `.steleignore`-filtered roster minus `.stele/`, with each tracked `AGENTS.md` read as a
/// node file. Undercover (§3.5): the same tracked roster for code/markdown comments — the
/// work tree's own `AGENTS.md` files are the collaborators', never node sources, so they
/// are read as plain markdown — plus every overlay `AGENTS.md` under `<home>/.stele/tree/`
/// as a node file.
fn migrate_targets(ws: &Workspace) -> Result<Vec<MigrateTarget>> {
    let stele_prefix = format!("{LOCK_DIR}/");
    let mut targets: Vec<MigrateTarget> = tracked_files(&ws.work)?
        .into_iter()
        .filter_map(|rel| {
            let display = rel.to_string_lossy().replace('\\', "/");
            let path = ws.work.join(&rel);
            // A tracked symlink is left untouched: the rename in `write_atomic` would replace
            // the link itself with a copy of its (possibly out-of-repo) target.
            if display == LOCK_DIR || display.starts_with(&stele_prefix) || path.is_symlink() {
                return None;
            }
            let kind = if ws.mode == Mode::Normal
                && rel.file_name().is_some_and(|name| name == "AGENTS.md")
            {
                legacy::SourceKind::Node
            } else if matches!(
                rel.extension().map(|e| e.to_string_lossy().to_lowercase()),
                Some(ref ext) if ext == "md" || ext == "markdown"
            ) {
                legacy::SourceKind::Markdown
            } else {
                legacy::SourceKind::Code
            };
            Some(MigrateTarget {
                display,
                kind,
                path,
            })
        })
        .collect();

    if ws.mode == Mode::Undercover {
        targets.extend(
            overlay_sources(&ws.home.join(TREE_DIR), Path::new(""))?
                .into_iter()
                .map(|source| MigrateTarget {
                    display: format!(
                        "{TREE_DIR}/{}",
                        source.rel.to_string_lossy().replace('\\', "/")
                    ),
                    kind: legacy::SourceKind::Node,
                    path: source.source,
                }),
        );
    }
    Ok(targets)
}

/// Read one migrate target. `Ok(None)` skips it: a tracked path with nothing to read
/// ([`crate::model::read_tracked`]), or a code file that is not valid UTF-8 — the §2.4 scan
/// decodes those lossily, but a rewrite would corrupt their bytes. A node source that is not valid UTF-8 is a §5.3 input error instead — the same hard stop
/// `build` gives it — and, running in pass 1, it aborts before anything is written.
fn read_migrate_source(target: &MigrateTarget) -> Result<Option<String>> {
    let Some(bytes) = crate::model::read_tracked(&target.path, &target.display)? else {
        return Ok(None);
    };
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Some(text)),
        Err(_) if target.kind == legacy::SourceKind::Node => Err(SteleError::input_msg(format!(
            "{}: node source is not valid UTF-8 (§3.1)",
            target.display
        ))),
        Err(_) => Ok(None),
    }
}

/// Warn (stderr, non-fatal) when the work tree carries uncommitted changes (§5.1): the
/// rewrite is meant to be reviewed as a diff, which a dirty tree makes harder. Purely
/// diagnostic — a `git` that cannot be spawned simply produces no warning, and neither
/// case changes the exit code.
fn warn_if_dirty(root: &Path) {
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.is_empty());
    if dirty {
        eprintln!(
            "warning: the work tree has uncommitted changes — stele migrate rewrites sources in \
             place and is meant to be reviewed as a diff (§5.1)"
        );
    }
}

// ─── the source pipeline (§5.1) ──────────────────────────────────────────────

/// One node source (§2.4/§3.5): `rel` is the repo-root-relative path fed to
/// [`parse::parse_agents_file`] (so node id and territory come out identical in both
/// modes), `source` is the absolute path the bytes are read from. In normal mode the two
/// name the same tracked file; in undercover mode `rel` is the mirror path (the
/// `.stele/tree/` prefix stripped) and `source` is the overlay file at the graph home.
struct NodeSource {
    rel: PathBuf,
    source: PathBuf,
}

/// The node-source roster for one build (§2.4/§3.5), mode-aware. Normal: every tracked
/// `AGENTS.md`, `rel` the tracked path and `source` under `ws.work` — today's behavior.
/// Undercover: a sorted recursive filesystem walk of `<home>/.stele/tree/**/AGENTS.md`
/// (never `git ls-files`, never `.steleignore`-filtered — these are authored sources, not
/// scanned code, §3.5), `rel` the mirror path with the `.stele/tree/` prefix stripped and
/// `source` the overlay absolute path. Every other build input (anchors, extraction, ADRs,
/// freshness) stays on `ws.work` + tracked, unchanged.
fn node_sources(ws: &Workspace, tracked: &[PathBuf]) -> Result<Vec<NodeSource>> {
    match ws.mode {
        Mode::Normal => Ok(tracked
            .iter()
            .filter(|rel| rel.file_name().is_some_and(|name| name == "AGENTS.md"))
            .map(|rel| NodeSource {
                rel: rel.clone(),
                source: ws.work.join(rel),
            })
            .collect()),
        Mode::Undercover => overlay_sources(&ws.home.join(TREE_DIR), Path::new("")),
    }
}

/// Every `AGENTS.md` under `<tree_root>/<rel>` (the overlay `<home>/.stele/tree/`, §3.5) as a
/// [`NodeSource`] whose `rel` mirrors the tree path (`<tree_root>/apps/api/AGENTS.md` →
/// `apps/api/AGENTS.md`), sorted by `rel`. A missing overlay root is empty, not an error (an
/// undercover repo with no scaffold yet builds an empty graph).
fn overlay_sources(tree_root: &Path, rel: &Path) -> Result<Vec<NodeSource>> {
    // Collected through its components so the root itself (`rel` empty) has no trailing `/`.
    let dir: PathBuf = tree_root.join(rel).components().collect();
    let entries = match std::fs::read_dir(&dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries
            .map_err(|e| SteleError::internal(format!("read overlay {}: {e}", dir.display())))?,
    };
    let mut sources = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| SteleError::internal(format!("read overlay entry: {e}")))?;
        let (rel, source) = (rel.join(entry.file_name()), entry.path());
        let file_type = entry
            .file_type()
            .map_err(|e| SteleError::internal(format!("stat {}: {e}", source.display())))?;
        if file_type.is_dir() {
            sources.extend(overlay_sources(tree_root, &rel)?);
        } else if rel.file_name().is_some_and(|name| name == "AGENTS.md") {
            sources.push(NodeSource { rel, source });
        }
    }
    sources.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(sources)
}

/// Sources → in-memory graph (§5.1): parse every node source (§3.5, [`node_sources`]) into
/// nodes, scan the tracked tree for comment anchors (§2.4), resolve each claim's anchor to
/// a `file:line` (§2.4), index the repo's ADRs (§2.6), and derive import edges
/// (§2.3/§4.2). Shared by `build` and `check`, so every derived slot is recomputed
/// identically on both paths. Node discovery is mode-aware (normal: tracked `AGENTS.md`;
/// undercover: the overlay walk); every other scan runs over `ws.work` + tracked
/// unchanged. `verified` is stamped only by `build` afterward (§4.5).
fn build_graph(ws: &Workspace, tracked: &[PathBuf]) -> Result<Graph> {
    let root = &ws.work;
    let mut graph = Graph::default();
    for source in node_sources(ws, tracked)? {
        let contents = read_node_source(&source.source, &source.rel)?;
        if let Some(node) = parse::parse_agents_file(&source.rel, &contents)? {
            graph.insert(node)?;
        }
    }

    // Import extraction (§2.3/§4.2): attribute each cross-boundary reference to its
    // owning node via the territory index, then fill each node's `extracted.imports`
    // and retain the per-edge reference occurrences for the structural class (§4.2).
    // Territory reads only the node roster, so extraction runs beside the anchor scan —
    // the two tree-sitter passes dominate a build — and its error still surfaces last.
    let territory = graph.territory();
    let (anchors, extraction) = std::thread::scope(|scope| {
        let extraction = assert::spawn(scope, || {
            extract::extract_imports(root, tracked, &territory)
        });
        (anchors::scan(root, tracked), extraction())
    });
    graph.anchors = anchors?;
    resolve_claim_anchors(root, &mut graph.nodes, &graph.anchors)?;
    graph.adrs = build_adr_index(root, tracked)?;
    let extraction = extraction?;
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

/// [`build_graph`], then [`warn_untracked_node_files`] over the untracked-but-not-gitignored
/// files. Their `git ls-files --others` walk shares nothing with the build, so it runs beside
/// the tree-sitter passes; a build error still surfaces first and suppresses every warning.
fn build_graph_and_warn(ws: &Workspace, tracked: &[PathBuf]) -> Result<Graph> {
    let (graph, others) = std::thread::scope(|scope| {
        let others = assert::spawn(scope, || {
            git_ls_files(&ws.work, &["--others", "--exclude-standard"])
        });
        (build_graph(ws, tracked), others())
    });
    let graph = graph?;
    warn_untracked_node_files(ws, others?)?;
    Ok(graph)
}

/// Read a node source (AGENTS.md, §3.1) from its absolute `source` path, naming the
/// repo-root-relative `rel` in any error (identical text in both modes, §3.5). Unlike the
/// scan's binary-skip (§2.4), a declared node source that is not valid UTF-8 is a hard
/// §5.3 input error naming the file — a node whose own bytes cannot be read is a
/// repo-out-of-spec condition, never silently dropped; so is one gone from the work tree
/// (§5.3 "a lock node whose file is gone"). A genuine IO failure stays a §5.3 internal error.
fn read_node_source(source: &Path, rel: &Path) -> Result<String> {
    let bytes = crate::model::read_tracked(source, &rel.display().to_string())?
        .ok_or_else(|| SteleError::input_msg(format!("{}: node source is gone", rel.display())))?;
    String::from_utf8(bytes).map_err(|_| {
        SteleError::input_msg(format!(
            "{}: node source is not valid UTF-8 (§3.1)",
            rel.display()
        ))
    })
}

/// Resolve every claim's anchor to a `file:line` (§2.4), recomputed every build. A
/// `※ <slug>` landmark anchor resolves to the winning landmark occurrence (or stays `null`
/// when the slug has zero occurrences — build stays 0, §4.1 fails it later). A
/// `<path>#<symbol>` anchor resolves via tree-sitter: exactly one definition →
/// its line; zero or many → `null` plus the §4.1 unresolved-vs-ambiguous marker. Its path
/// binds only when git tracks it byte for byte — `.steleignore`d or not, since a named path
/// is not scanned — so an untracked file or a case-folded spelling never resolves.
fn resolve_claim_anchors(
    root: &Path,
    nodes: &mut [Node],
    anchors: &crate::model::AnchorData,
) -> Result<()> {
    let named = git_ls_files(root, &[])?;
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
                let tracked = named.iter().any(|file| file.as_os_str() == path);
                let resolution = tracked.then(|| anchors::resolve_symbol(root, path, symbol));
                match resolution
                    .transpose()?
                    .unwrap_or(SymbolResolution::Unresolved)
                {
                    SymbolResolution::Resolved(line) => {
                        claim.resolved = Some(format!("{path}:{line}"));
                        claim.resolution = Resolution::Resolved;
                    }
                    SymbolResolution::Ambiguous => claim.resolution = Resolution::Ambiguous,
                    SymbolResolution::Unresolved => claim.resolution = Resolution::Unresolved,
                }
            } else {
                // In neither namespace (no `※`, no `#`): derive_slug accepts it as a bare
                // symbol, but it binds nothing — §4.1 reports it unresolved.
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
    let claims: Vec<_> = graph
        .nodes
        .iter_mut()
        .flat_map(|node| node.invariants.iter_mut().chain(&mut node.hazards))
        .filter(|claim| claim.resolved.is_some())
        .collect();
    let digests = assert::par_map(&claims, |claim| {
        anchors::digest_for_claim(root, &claim.anchor, claim.resolved.as_deref().unwrap())
    });
    for (claim, digest) in claims.into_iter().zip(digests) {
        claim.verified = Some(Verified {
            sha: sha.clone(),
            digest: digest?,
        });
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
    Ok(git_ls_files(root, &[])?
        .into_iter()
        .filter(|rel| !ignore.is_ignored(&rel.to_string_lossy().replace('\\', "/")))
        .collect())
}

/// Warn (stderr only) about work-tree AGENTS.md files that declare a stele node the build's
/// roster will never read — a real adoption trap, mode-aware (§2.4/§3.5). For each untracked,
/// non-gitignored, non-steleignored work-tree AGENTS.md whose content carries a stele block,
/// print ONE warning line. This is purely diagnostic: stdout, the process exit code, and the
/// `--json` envelope are all untouched. Excluded (each for a reason): a gitignored/excluded
/// path (`--exclude-standard` drops it — including the undercover overlay under `.stele/`,
/// hidden by the `info/exclude` block, §3.5), a steleignored path (already invisible to every
/// scan, §2.4), and a blockless AGENTS.md (`Ok(None)` = plain-markdown degradation, not a
/// node). The message differs by mode: Normal names the untracked node the tracked scan misses
/// (run `git add`); Undercover names a node authored outside the overlay (build reads only
/// `.stele/tree/`), which is the confusion signal, not an adoption gap.
fn warn_untracked_node_files(ws: &Workspace, others: Vec<PathBuf>) -> Result<()> {
    let root = &ws.work;
    let ignore = Steleignore::load(root)?;
    for rel in others {
        if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
            continue;
        }
        let posix = rel.to_string_lossy().replace('\\', "/");
        if ignore.is_ignored(&posix) {
            continue;
        }
        // Non-UTF-8 bytes cannot carry a parseable stele block, so an unreadable file reads
        // as empty — blockless, a silent skip (never a node). `Ok(None)` = blockless;
        // `Ok(Some)`/`Err` both mean a block is present (a malformed block still declares a
        // node the scan omits).
        let contents = std::fs::read_to_string(root.join(&rel)).unwrap_or_default();
        if matches!(parse::parse_agents_file(&rel, &contents), Ok(None)) {
            continue;
        }
        match ws.mode {
            Mode::Normal => eprintln!(
                "warning: {posix} declares a stele node but is not tracked — run git add \
                 (build scans tracked files only, §2.4)"
            ),
            Mode::Undercover => eprintln!(
                "warning: {posix} declares a stele node but undercover mode reads node sources \
                 from .stele/tree/ — move it into the overlay"
            ),
        }
    }
    Ok(())
}

/// `git ls-files <args> -z` (§2.4): with no args the raw VCS-tracked file list.
/// Repo-root-relative and sorted, NUL-delimited to survive unusual filenames.
fn git_ls_files(root: &Path, args: &[&str]) -> Result<Vec<PathBuf>> {
    // Messages name the command by its first flag: `git ls-files`, `git ls-files --others`.
    let cmd = ["git ls-files"].into_iter().chain(args.first().copied());
    let cmd = cmd.collect::<Vec<_>>().join(" ");
    let output = Command::new("git")
        .arg("ls-files")
        .args(args)
        .arg("-z")
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `{cmd}`: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Outside a work tree git prints "fatal: not a git repository …" — a user
        // mistake (§5.3 input, exit 2), not an internal fault. Surface a one-liner
        // instead of the raw git text so `stele <verb>` outside a repo fails cleanly. Only the
        // tracked scan (no args) is that probe; a later listing failing is internal.
        if args.is_empty() && stderr.contains("not a git repository") {
            return Err(SteleError::input_msg(
                "not a git repository (stele scans VCS-tracked files; run inside a git repo)",
            ));
        }
        return Err(SteleError::internal(format!(
            "`{cmd}` failed: {}",
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
        // Gone (deleted, dangling) is skipped like any scanned file; bytes decode lossily,
        // as only the status token matters.
        let Some(bytes) = crate::model::read_tracked(&root.join(path), path)? else {
            continue;
        };
        entries.push(AdrEntry {
            id: format!("{dir}/{nnnn}"),
            number,
            status: adr_status(&String::from_utf8_lossy(&bytes)),
            path: path.clone(),
        });
    }
    Ok(entries)
}

/// The zero-padded number of a tracked ADR file `<dir>/NNNN-*.md` (e.g. `"0007"`),
/// or `None` when the path is not an ADR file directly under `dir`.
fn adr_number<'a>(dir: &str, path: &'a str) -> Option<&'a str> {
    let name = (path.strip_prefix(dir)?.strip_prefix('/'))
        .filter(|name| !name.contains('/') && name.ends_with(".md"))?;
    let digits = name.split('-').next().filter(|d| !d.is_empty())?;
    digits.bytes().all(|b| b.is_ascii_digit()).then_some(digits)
}

/// The ADR status (§2.6): the lowercased status token, drawn from whichever of the three
/// conventions in the wild the file uses, or `"unknown"` when none is present. §4.1
/// checks `≠ superseded`.
///   • MADR YAML frontmatter — a `status:` key inside a leading `---`-fenced block;
///   • adr-tools/Nygard inline — a `Status: X` on any line (the original form);
///   • adr-tools/Nygard heading — a `## Status` ATX heading, the value on the first
///     non-empty line after it.
/// Each value reduces to its first word ([`status_word`]): quotes, emphasis and links drop.
/// Frontmatter outranks the inline form, which outranks the heading form.
fn adr_status(contents: &str) -> String {
    const UNKNOWN: &str = "unknown";
    frontmatter_status(contents)
        .or_else(|| inline_status(contents))
        .or_else(|| heading_status(contents))
        .unwrap_or_else(|| UNKNOWN.to_string())
}

/// The MADR YAML-frontmatter status: the `status:` key inside a leading `---` fence,
/// reduced to its status word (so a quoted scalar reads as its first word). `None` when
/// the file opens with no frontmatter or the block carries no `status:` key.
fn frontmatter_status(contents: &str) -> Option<String> {
    const FENCE: &str = "---";
    let mut lines = contents.lines();
    (lines.next()?.trim_end() == FENCE).then_some(())?;
    lines
        .map(str::trim)
        .take_while(|line| *line != FENCE)
        .find_map(|line| line.strip_prefix("status:"))
        .and_then(status_word)
}

/// The inline `Status: X` form (adr-tools/Nygard, the original): the status word after
/// the first `Status:` on any line, markdown emphasis on either side dropped. `None` when
/// no line carries a `Status:` with a word after it.
fn inline_status(contents: &str) -> Option<String> {
    contents
        .lines()
        .find_map(|line| line.split("Status:").nth(1))
        .and_then(status_word)
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

/// The status word of a status value: quoting, emphasis and markdown-link syntax reduced
/// to its text by taking the first maximal run of ASCII letters and inner hyphens, lowercased (so
/// `[Accepted](0009.md)` → `accepted`, `Superseded by [ADR-9](…)` → `superseded`).
/// `None` for a line with no letters.
fn status_word(line: &str) -> Option<String> {
    let word: String = line
        .chars()
        .skip_while(|c| !c.is_ascii_alphabetic())
        .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
        .collect();
    (!word.is_empty()).then(|| word.to_ascii_lowercase())
}

// ─── the committed lock (§5.3) ────────────────────────────────────────────────

/// Read the committed lock, mapping a missing file to the §5.3 input error (exit
/// 2 + `run stele build`) that `check`/`emit`/`node` share. A genuine IO failure
/// is an internal error (exit 3).
fn read_committed_lock(root: &Path) -> Result<String> {
    let shown = format!("{LOCK_DIR}/{LOCK_FILE}");
    read_if_present(&root.join(&shown), &shown)?.ok_or_else(|| {
        SteleError::input_msg(format!("no committed lock at {shown}; {RUN_BUILD_HINT}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A normal-mode workspace rooted at `root` — `home` and `work` the identical path,
    /// the only mode a direct verb call exercises (§3.5). Mirrors what [`resolve_workspace`]
    /// returns for a normal checkout, without the `resolve_workspace` git spawn.
    fn normal_workspace(root: &Path) -> Workspace {
        Workspace {
            common: root.to_path_buf(),
            home: root.to_path_buf(),
            mode: Mode::Normal,
            work: root.to_path_buf(),
        }
    }

    /// Run `git` in `root`, asserting success — the tracked-file/staging surface `init`
    /// needs (§2.4). Kept minimal: `init` reads `git ls-files` and calls `git add`.
    /// Repo-locating vars are dropped — under a git hook, an inherited `GIT_DIR`
    /// would point the fixture's git at the host repo.
    fn git(root: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        for var in [
            "GIT_COMMON_DIR",
            "GIT_DIR",
            "GIT_INDEX_FILE",
            "GIT_WORK_TREE",
        ] {
            cmd.env_remove(var);
        }
        let out = cmd.args(args).current_dir(root).output().expect("run git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    #[test]
    fn git_add_warns_with_the_os_error_when_git_cannot_spawn() {
        // A vanished work tree fails the spawn itself (`current_dir`), not git.
        let gone = tempfile::tempdir().unwrap().path().to_path_buf();
        let warning = git_add(&gone, &["AGENTS.md".into()]).unwrap();
        assert!(
            warning.starts_with("stele init: `git add` failed: could not run git: "),
            "{warning}"
        );
        assert!(warning.ends_with("git add -- AGENTS.md"), "{warning}");
    }

    #[test]
    fn adr_status_reads_all_three_conventions() {
        // MADR YAML frontmatter (F5); a frontmatter block without `status:` defers.
        assert_eq!(
            adr_status("---\nstatus: superseded\ndate: 2026-01-01\n---\n# t\n"),
            "superseded"
        );
        assert_eq!(
            adr_status("---\ndate: 2026-01-01\n---\nStatus: Proposed\n"),
            "proposed"
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
        // A quoted YAML scalar (the MADR 4.0 template) and markdown emphasis reduce to the
        // status word, as the heading form does.
        for contents in [
            "---\nstatus: \"superseded by ADR-0002\"\n---\n",
            "**Status:** Superseded\n",
            "Status: **Superseded**\n",
        ] {
            assert_eq!(adr_status(contents), "superseded", "{contents:?}");
        }
        // A hyphenated status stays whole: `in-progress` is not `in`, nor is a
        // `superseded-candidate` superseded.
        assert_eq!(adr_status("---\nstatus: in-progress\n---\n"), "in-progress");
        assert_eq!(
            adr_status("Status: Superseded-candidate\n"),
            "superseded-candidate"
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
