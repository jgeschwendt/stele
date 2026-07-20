//! Command-line surface and subcommand dispatch (SPEC §5.1, §5.3).
//!
//! `build` is the sole writer of `.stele/graph.lock`: it walks the repo, parses
//! every VCS-tracked AGENTS.md into an in-memory graph, and atomically writes the
//! canonical lock (§5.1 pipeline). `check` requires a committed lock, rebuilds
//! in-memory, and byte-compares the canonical serialization against the on-disk
//! file (§5.3); the six assertion classes land in Phase D, after that compare.
//! `emit`/`node` are Phase B stubs that only enforce the lock-presence gate.
//! `build_graph` resolves comment anchors and the ADR index (Phase C1) and derives
//! import edges (Phase C2), so `build` stamps `resolved` and `verified {sha}` and
//! fills `landmarks{}`/`adrs{}`/`extracted.imports`; the freshness `digest` (C3) is
//! the last compiled slot still `null`.

use crate::anchors::{self, SymbolResolution};
use crate::config;
use crate::extract;
use crate::lock::{self, Lock};
use crate::model::{AdrEntry, Graph, Node, Resolution, Result, SteleError, Verified};
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
    let mut graph = build_graph(root)?;
    stamp_verified(root, &mut graph)?;
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
        let contents = std::fs::read_to_string(root.join(rel))
            .map_err(|e| SteleError::internal(format!("read {}: {e}", rel.display())))?;
        if let Some(node) = parse::parse_agents_file(rel, &contents)? {
            graph.insert(node)?;
        }
    }

    graph.anchors = anchors::scan(root, &tracked)?;
    resolve_claim_anchors(root, &mut graph.nodes, &graph.anchors)?;
    graph.adrs = build_adr_index(root, &tracked)?;

    // Import extraction (§2.3/§4.2): attribute each cross-boundary reference to its
    // owning node via the territory index, then fill each node's `extracted.imports`.
    let territory = graph.territory();
    let imports = extract::extract_imports(root, &tracked, &territory)?;
    for node in &mut graph.nodes {
        node.extracted_imports = imports.get(&node.id).cloned().unwrap_or_default();
    }
    Ok(graph)
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
/// carries `verified` over instead (§4.5).
fn stamp_verified(root: &Path, graph: &mut Graph) -> Result<()> {
    let sha = head_sha(root)?;
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
    Ok(())
}

/// The full `HEAD` commit sha (§4.5 watermark). A repo with no commit yet is a §5.3
/// internal error — `build` needs a watermark to stamp.
fn head_sha(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git rev-parse HEAD`: {e}")))?;
    if !output.status.success() {
        return Err(SteleError::internal(format!(
            "`git rev-parse HEAD` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// VCS-tracked files (§2.4 scan scope: tracked only, so `.gitignore` is honored by
/// construction), repo-root-relative and sorted for deterministic iteration.
/// `git ls-files -z` is NUL-delimited to survive unusual filenames.
fn tracked_files(root: &Path) -> Result<Vec<PathBuf>> {
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

/// The ADR status (§2.6): the lowercased first token after `Status:` in the file,
/// or `"unknown"` when there is no `Status:` line. §4.1 checks `≠ superseded`.
fn adr_status(contents: &str) -> String {
    const UNKNOWN: &str = "unknown";
    contents
        .lines()
        .find_map(|line| line.split("Status:").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or(UNKNOWN)
        .to_ascii_lowercase()
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
