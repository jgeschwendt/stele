//! `stele emit` (§3.1/§6.1): render the committed lock into generated regions, the
//! transpose indexes, harness shims and `.claude/rules/` (§3.3), or diff them under `--check`.

use super::{
    CommandOutput, Mode, RUN_BUILD_HINT, TREE_DIR, Workspace,
    init::{ensure_info_exclude, in_tree_target, write_atomic},
    read_committed_lock, read_if_present, require_lock_version,
};
use crate::config;
use crate::emit;
use crate::lock::{self, Lock, LockNode};
use crate::model::{ExitCode, Result, SteleError};
use crate::parse;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The `emit --check` flag (§3.1): render to memory and diff against on-disk regions
/// and index files, failing CI on divergence instead of rewriting.
pub(super) const EMIT_CHECK_FLAG: &str = "--check";
/// The `emit --claude-rules` flag (§3.3): additionally render path-scoped
/// `.claude/rules/*.md` from node claims (opt-in).
pub(super) const EMIT_CLAUDE_RULES_FLAG: &str = "--claude-rules";
/// The transpose-index directory (§6.1), relative to the repo root.
pub(super) const INDEX_DIR: &str = ".stele/index";
pub(super) const INVARIANTS_INDEX: &str = "invariants.md";
pub(super) const HAZARDS_INDEX: &str = "hazards.md";
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
pub(super) fn emit(ws: &Workspace, args: &[&str]) -> Result<CommandOutput> {
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
pub(super) fn write_under_root(root: &Path, dir: &str, name: &str, content: &str) -> Result<()> {
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
