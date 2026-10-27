//! Command-line surface and subcommand dispatch (SPEC §5.1, §5.3).
//!
//! This module is the shell: it resolves the [`Workspace`] (§3.5), holds each verb to its
//! §5.1 grammar, and prints the result as human text or the one `--json` envelope (§5.3);
//! [`serve_render`] gives `serve` the same path. It also loads the committed lock the read
//! verbs share and guards undercover mode against a tracked shared graph. Submodules:
//!
//! - `gate` — `build`, the sole writer of `.stele/graph.lock`; `check`, which rebuilds in
//!   memory, byte-compares against the committed lock, then runs the six §4 assertion
//!   classes; and `blame`.
//! - `graph` — the in-memory pipeline: node sources → parsed graph with resolved anchors,
//!   the ADR index, import edges, and the `verified {sha, digest}` watermarks.
//! - `emit` — renders generated regions, transpose indexes, harness shims and
//!   `.claude/rules/` from the committed lock, or byte-diffs them under `--check`.
//! - `query` — the lock-reading verbs: `root`, `node`, `unfold`, `invariants`, `hazards`,
//!   `nodes`.
//! - `init` — the adoption scaffold (§7), normal and `--undercover` (§3.5).
//! - `fs` — the shared write primitives: `write_atomic` (the lock's writer),
//!   `write_under_root`, `in_tree_target`, `append_separator`, and the undercover
//!   `info/exclude` block.
//! - `migrate` — the in-place rewrite of pre-0.3.0 notation (§2.5).

mod emit;
mod fs;
mod gate;
mod graph;
mod init;
mod migrate;
mod query;

use crate::assert::{self, Finding};
use crate::lock::{self, Lock};
use crate::model::{ExitCode, Result, SteleError};
use emit::{EMIT_CHECK_FLAG, EMIT_CLAUDE_RULES_FLAG, emit};
use gate::{blame, build, check};
use graph::git_ls_files;
use init::{UNDERCOVER_FLAG, existing_node_dirs, init};
use migrate::migrate;
use query::{cmd_root, hazards, invariants, node, nodes, unfold};
use serde::Serialize;
use serde_json::{Value, json};
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

/// The stable machine contract (§5.3 `--json` envelope). `findings` carries assertion
/// results only; input/internal errors surface via
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
// dir, outside the tracked tree; `init --undercover` writes the marker that selects it.

/// The two graph modes (§3.5): a committed graph at the repo root, or a private on-disk
/// graph rooted at the parent of the git common dir. Carried on every [`Workspace`].
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
/// artifact (contrast `.stele/config.toml`, §3.4), written by `init --undercover`.
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
/// `query::repo_relative` to refuse).
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

// ─── the committed lock (§5.3) ────────────────────────────────────────────────

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

/// Read the committed lock, mapping a missing file to the §5.3 input error (exit
/// 2 + `run stele build`) that `check`/`emit`/`node` share. A genuine IO failure
/// is an internal error (exit 3).
fn read_committed_lock(root: &Path) -> Result<String> {
    let shown = format!("{LOCK_DIR}/{LOCK_FILE}");
    read_if_present(&root.join(&shown), &shown)?.ok_or_else(|| {
        SteleError::input_msg(format!("no committed lock at {shown}; {RUN_BUILD_HINT}"))
    })
}
