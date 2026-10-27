//! Shared filesystem primitives for the verbs that write: the atomic replace, the
//! under-root writer, the in-tree symlink resolver, and the undercover `info/exclude` block.

use super::read_if_present;
use crate::model::{Result, SteleError};
use std::path::{Path, PathBuf};

/// The marker-fenced managed block `init --undercover` maintains in `<common-dir>/info/exclude`
/// (§3.5): its begin/end lines bracket the two unconditional exclude entries so the overlay
/// and the shim never surface in `git status`. Rewritten in place, never touching a line
/// outside the fence.
const EXCLUDE_BEGIN: &str = "# stele:begin undercover";
const EXCLUDE_END: &str = "# stele:end undercover";
const EXCLUDE_ENTRIES: [&str; 2] = ["/.stele/", "/CLAUDE.local.md"];

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
pub(super) fn append_separator(contents: &str) -> &'static str {
    if contents.is_empty() || contents.ends_with('\n') {
        ""
    } else {
        "\n"
    }
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

/// Write a file under a repo-root-relative directory, creating the directory tree.
pub(super) fn write_under_root(root: &Path, dir: &str, name: &str, content: &str) -> Result<()> {
    let dir_path = root.join(dir);
    std::fs::create_dir_all(&dir_path)
        .map_err(|e| SteleError::internal(format!("create {dir}: {e}")))?;
    std::fs::write(dir_path.join(name), content)
        .map_err(|e| SteleError::internal(format!("write {dir}/{name}: {e}")))
}
