//! `stele migrate` (§5.1): rewrite pre-0.3.0 `stele:` notation to the glyph notation
//! (§2.5) in place, over `build`'s scan scope.

use super::{
    CommandOutput, LOCK_DIR, Mode, TREE_DIR, Workspace,
    fs::{ensure_info_exclude, write_atomic},
    graph::{overlay_sources, tracked_files},
};
use crate::legacy;
use crate::model::{Result, SteleError};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

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
pub(super) fn migrate(ws: &Workspace, _: &[&str]) -> Result<CommandOutput> {
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
