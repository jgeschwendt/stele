//! `stele init` (§7): the adoption scaffold — node blocks, empty generated regions, and the
//! private `--undercover` overlay with its `info/exclude` block (§3.5).

use super::{
    CommandOutput, LOCK_DIR, TREE_DIR, Workspace,
    emit::write_under_root,
    graph::{ADR_DIRS, adr_number, read_node_source, tracked_files},
    graph_home, read_if_present,
};
use crate::assert;
use crate::model::{DECISION_PREFIX, NodeKind, PURPOSE_MAX_CHARS, Result, SteleError};
use crate::parse;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The empty generated region `init` writes (§3.1/§7): the bare one-line form, with no
/// name and no body between the markers, for `emit` to fill later.
const EMPTY_REGION: &str = "<!-- @stele --><!-- @end -->\n";

/// The `init --undercover` flag (§3.5): scaffold a private overlay graph instead of the
/// in-tree one. The one extra token `init` accepts; inert on every other verb.
pub(super) const UNDERCOVER_FLAG: &str = "--undercover";

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
pub(super) fn init(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
pub(super) fn reject_shared_graph_conflict(work: &Path, tracked: &[PathBuf]) -> Result<()> {
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
pub(super) fn ensure_info_exclude(common: &Path) -> Result<()> {
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
pub(super) fn write_atomic(path: &Path, bytes: &str, lock_home: Option<&Path>) -> Result<()> {
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
pub(super) fn in_tree_target(root: &Path, rel: &Path) -> Option<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Mode;

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
