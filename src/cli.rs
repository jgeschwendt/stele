//! Command-line surface and subcommand dispatch (SPEC §5.1, §5.3).
//!
//! `build` is the sole writer of `.stele/graph.lock`: it walks the repo, parses
//! every VCS-tracked AGENTS.md into an in-memory graph, and atomically writes the
//! canonical lock (§5.1 pipeline). `check` requires a committed lock, rebuilds
//! in-memory, and byte-compares the canonical serialization against the on-disk
//! file (§5.3); the six assertion classes land in Phase D, after that compare.
//! `emit`/`node` are Phase B stubs that only enforce the lock-presence gate.
//! Anchor resolution and import extraction arrive in Phase C, so `build` stamps
//! `resolved`/`verified` null and leaves `extracted.imports`/`landmarks` empty.

use crate::config;
use crate::lock::{self, Lock};
use crate::model::{Graph, Result, SteleError};
use crate::parse;
use serde::Serialize;
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The committed lock, relative to the repo root.
const LOCK_DIR: &str = ".stele";
const LOCK_FILE: &str = "graph.lock";
/// Temp name a fresh lock is written to before the atomic rename (§5.3 build
/// atomicity: never a partial lock on disk).
const LOCK_TEMP: &str = ".graph.lock.tmp";

/// The message every read command emits when the committed lock is missing or
/// stale (§5.3); tests assert on the `run stele build` substring.
const RUN_BUILD_HINT: &str = "run stele build";

/// The machine-output flag (§5.3): exactly one JSON envelope on stdout.
const JSON_FLAG: &str = "--json";

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

/// A command's result: the `--json` `data` payload plus an optional human-readable
/// line (printed only when `--json` is absent).
struct CommandOutput {
    data: Value,
    summary: Option<String>,
}

impl CommandOutput {
    /// A stub command's empty success (no data, no human line).
    fn empty() -> Self {
        Self {
            data: json!({}),
            summary: None,
        }
    }
}

/// Run the CLI, returning the §5.3 process exit code (`0` success, else the error's
/// exit class). With `--json`, exactly one envelope prints to stdout regardless of
/// outcome; otherwise a success prints its human line to stdout and an error prints
/// `file:line: message` to stderr.
pub fn run(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == JSON_FLAG);
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != JSON_FLAG)
        .collect();
    let root = Path::new(".");
    let command = rest.first().copied().unwrap_or("");

    match dispatch(root, &rest) {
        Ok(output) => {
            if json {
                print_envelope(command, true, 0, output.data);
            } else if let Some(summary) = output.summary {
                println!("{summary}");
            }
            0
        }
        Err(error) => {
            let exit = error.exit as i32;
            if json {
                print_envelope(command, false, exit, error_data(&error));
            } else {
                eprintln!("{error}");
            }
            exit
        }
    }
}

fn dispatch(root: &Path, args: &[&str]) -> Result<CommandOutput> {
    match args.first().copied() {
        Some("build") => build(root),
        Some("check") => check(root),
        Some("emit") => emit(root),
        Some("node") => node(root),
        Some(other) => Err(SteleError::input_msg(format!(
            "unknown or not-yet-implemented command: {other:?} (the full surface lands in Phase E)"
        ))),
        None => Err(SteleError::input_msg(
            "usage: stele build | check | emit | node <id>   (the full surface lands in Phase E)",
        )),
    }
}

/// Print the one §5.3 JSON envelope. `findings` is always empty until Phase D.
fn print_envelope(command: &str, ok: bool, exit: i32, data: Value) {
    let envelope = Envelope {
        stele: env!("CARGO_PKG_VERSION"),
        command,
        ok,
        exit,
        data,
        findings: Vec::new(),
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

/// `stele build` (§5.1): sources → in-memory graph → atomically written lock.
/// Any input error aborts before a single byte is written (§5.3 atomicity).
/// `build` never reads `.stele/config.toml` — config tunes `check`/`emit`, not the
/// graph (§3.4).
fn build(root: &Path) -> Result<CommandOutput> {
    let graph = build_graph(root)?;
    let lock = Lock::from_graph(&graph);
    let bytes = lock::to_canonical_string(&lock);
    write_lock_atomic(root, &bytes)?;
    let nodes = graph.nodes.len();
    Ok(CommandOutput {
        data: json!({ "lock": format!("{LOCK_DIR}/{LOCK_FILE}"), "nodes": nodes }),
        summary: Some(format!(
            "stele build: wrote {LOCK_DIR}/{LOCK_FILE} ({nodes} node(s))"
        )),
    })
}

/// `stele check` (§5.1, §5.3): load config (§3.4), require a committed lock, rebuild
/// the graph in-memory, and byte-compare the canonical serialization against the
/// on-disk file. `verified` watermarks are carried over from the committed lock,
/// never re-stamped (§4.5). The six assertion classes run here in Phase D, after the
/// compare succeeds; for now a matching lock exits 0.
fn check(root: &Path) -> Result<CommandOutput> {
    let _config = config::load(root)?;

    let on_disk = read_committed_lock(root)?;

    let version = lock::read_version(&on_disk)?;
    if version != lock::LOCK_VERSION {
        return Err(SteleError::input_msg(format!(
            "committed lock is version {version}; this engine writes version {}. {RUN_BUILD_HINT}",
            lock::LOCK_VERSION
        )));
    }
    let committed = lock::parse_lock(&on_disk)?;

    let graph = build_graph(root)?;
    let mut rebuilt = Lock::from_graph(&graph);
    rebuilt.carry_over_verified(&committed);

    if lock::to_canonical_string(&rebuilt) != on_disk {
        return Err(SteleError::input_msg(format!(
            "committed lock does not match the freshly-built graph; {RUN_BUILD_HINT}"
        )));
    }

    // Phase D: the six assertion classes (§4) run over `graph`/`_config` here.
    Ok(CommandOutput {
        data: json!({ "nodes": graph.nodes.len() }),
        summary: None,
    })
}

/// `stele emit` (§5.1): a Phase B stub. It loads config (§3.4) and enforces the §5.3
/// lock-presence gate; rendering lands in Phase E.
fn emit(root: &Path) -> Result<CommandOutput> {
    let _config = config::load(root)?;
    read_committed_lock(root)?;
    Ok(CommandOutput::empty())
}

/// `stele node <id>` (§5.1): a Phase B stub. It enforces only the §5.3 lock-presence
/// gate; the query lands in Phase E. Config tunes `check`/`emit`, not queries (§3.4).
fn node(root: &Path) -> Result<CommandOutput> {
    read_committed_lock(root)?;
    Ok(CommandOutput::empty())
}

// ─── the source pipeline (§5.1) ──────────────────────────────────────────────

/// Sources → in-memory graph (§5.1): parse every VCS-tracked AGENTS.md and
/// aggregate declared nodes, rejecting duplicate ids. Shared by `build` and
/// `check`. Anchor resolution and import extraction arrive in Phase C.
fn build_graph(root: &Path) -> Result<Graph> {
    let mut graph = Graph::default();
    for rel in tracked_agents_files(root)? {
        let absolute = root.join(&rel);
        let contents = std::fs::read_to_string(&absolute)
            .map_err(|e| SteleError::internal(format!("read {}: {e}", rel.display())))?;
        if let Some(node) = parse::parse_agents_file(&rel, &contents)? {
            graph.insert(node)?;
        }
    }
    Ok(graph)
}

/// VCS-tracked AGENTS.md files (§2.4 scan scope: tracked only, so `.gitignore` is
/// honored by construction), repo-root-relative and sorted for deterministic
/// iteration. `git ls-files -z` is NUL-delimited to survive unusual filenames.
fn tracked_agents_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git ls-files`: {e}")))?;
    if !output.status.success() {
        return Err(SteleError::internal(format!(
            "`git ls-files` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<PathBuf> = listing
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.file_name().is_some_and(|name| name == "AGENTS.md"))
        .collect();
    files.sort();
    Ok(files)
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
/// any input error has aborted before this point.
fn write_lock_atomic(root: &Path, bytes: &str) -> Result<()> {
    let dir = root.join(LOCK_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| SteleError::internal(format!("create {LOCK_DIR}: {e}")))?;
    let temp = dir.join(LOCK_TEMP);
    std::fs::write(&temp, bytes)
        .map_err(|e| SteleError::internal(format!("write {LOCK_DIR}/{LOCK_TEMP}: {e}")))?;
    std::fs::rename(&temp, dir.join(LOCK_FILE))
        .map_err(|e| SteleError::internal(format!("rename into {LOCK_DIR}/{LOCK_FILE}: {e}")))?;
    Ok(())
}
