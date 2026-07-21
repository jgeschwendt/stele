//! SPEC §3.5 undercover mode — Phase 3: `init --undercover` (marker + overlay scaffold +
//! `info/exclude` block) and the mode-aware node roster that lets `build` discover overlay
//! nodes. The invariant under test throughout: nothing the engine writes ever surfaces in
//! `git status` — the overlay lives at the graph home under `.stele/`, hidden by the shared
//! common-dir exclude block, and `init --undercover` never `git add`s. The broader matrix
//! (grove worktrees, emit/serve, per-worktree freshness) lands in Phase 5.

mod common;

use common::{Fixture, RunResult};
use std::path::Path;
use std::process::Command;

/// Run the built binary in an arbitrary directory (a linked worktree the [`Fixture`] does
/// not own), capturing status + both streams. The read/build verbs exercised here spawn only
/// `git`, so the inherited PATH suffices — no liveness stubs needed.
fn run_in(dir: &Path, args: &[&str]) -> RunResult {
    let out = Command::new(common::BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn stele");
    RunResult {
        code: out.status.code().expect("stele terminated by signal"),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    }
}

/// An undercover repo with a built private lock: `committed_two_dir_repo` + `init --undercover`,
/// an authored overlay root system node (with a generated region for `emit` to fill), then a
/// clean `build`. The scaffolded `apps`/`packages` container nodes are left as-is.
fn undercover_built() -> Fixture {
    let fixture = committed_two_dir_repo();
    assert_eq!(fixture.run(&["init", "--undercover"]).code, 0);
    fixture.write(
        ".stele/tree/AGENTS.md",
        "# root\n\n```stele\nkind: system\npurpose: undercover root\n```\n\n\
         <!-- stele:begin router -->\n<!-- stele:end -->\n",
    );
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "build:\n{}", build.combined());
    fixture
}

/// Run `git` in the fixture root and return trimmed stdout (asserting success).
fn git_stdout(fixture: &Fixture, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(&fixture.root)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare repo with two committed app directories (no AGENTS.md, no `.stele/`), the clean
/// canvas for an undercover adoption.
fn committed_two_dir_repo() -> Fixture {
    let fixture = Fixture::bare();
    fixture.write(
        "apps/web/lib/app.ex",
        "defmodule Acme.App do\n  def hello, do: :world\nend\n",
    );
    fixture.write(
        "packages/shared/src/index.ts",
        "export const version = 1;\n",
    );
    fixture.commit("materialize a bare two-directory tree");
    fixture
}

// `init --undercover` writes the marker, scaffolds the overlay, installs the exclude block,
// and leaves the work tree byte-clean — nothing tracked, nothing staged.
#[test]
fn init_undercover_writes_marker_overlay_and_exclude_block() {
    let fixture = committed_two_dir_repo();

    let init = fixture.run(&["init", "--undercover"]);
    assert_eq!(init.code, 0, "init --undercover:\n{}", init.combined());
    assert!(
        init.combined().contains("undercover"),
        "summary omits undercover:\n{}",
        init.combined()
    );
    // Never the normal-init staging hint (§3.5: no git add).
    assert!(
        !init.combined().contains("git add"),
        "undercover init printed a staging hint:\n{}",
        init.combined()
    );

    // Marker + overlay root + per-top-dir overlay nodes all live under the graph home.
    assert!(fixture.path(".stele/undercover").exists(), "no marker");
    assert_eq!(fixture.read(".stele/undercover"), "stele undercover\n");
    assert!(
        fixture.path(".stele/tree/AGENTS.md").exists(),
        "no overlay root node"
    );
    assert!(
        fixture.path(".stele/tree/apps/AGENTS.md").exists(),
        "no overlay apps node"
    );
    assert!(
        fixture.path(".stele/tree/packages/AGENTS.md").exists(),
        "no overlay packages node"
    );
    // Nothing scaffolded into the tracked tree.
    assert!(
        !fixture.path("AGENTS.md").exists(),
        "init leaked a root AGENTS.md into the work tree"
    );

    // The managed exclude block carries both entries between its fence.
    let exclude = fixture.read(".git/info/exclude");
    assert!(exclude.contains("# stele:begin undercover"), "{exclude}");
    assert!(exclude.contains("# stele:end undercover"), "{exclude}");
    assert!(exclude.contains("/.stele/"), "{exclude}");
    assert!(exclude.contains("/CLAUDE.local.md"), "{exclude}");

    // The leak test: the exclude block hides the overlay, so status is empty and nothing staged.
    assert_eq!(
        git_stdout(&fixture, &["status", "--porcelain"]),
        "",
        "undercover init left the work tree dirty"
    );
    assert_eq!(
        git_stdout(&fixture, &["diff", "--cached", "--name-only"]),
        "",
        "undercover init staged files"
    );
}

// Re-running `init --undercover` is idempotent: exit 0, overlay bytes untouched.
#[test]
fn init_undercover_is_idempotent() {
    let fixture = committed_two_dir_repo();
    assert_eq!(fixture.run(&["init", "--undercover"]).code, 0);

    let root_before = fixture.read(".stele/tree/AGENTS.md");
    let apps_before = fixture.read(".stele/tree/apps/AGENTS.md");
    let exclude_before = fixture.read(".git/info/exclude");

    let second = fixture.run(&["init", "--undercover"]);
    assert_eq!(second.code, 0, "second init:\n{}", second.combined());

    assert_eq!(fixture.read(".stele/tree/AGENTS.md"), root_before);
    assert_eq!(fixture.read(".stele/tree/apps/AGENTS.md"), apps_before);
    assert_eq!(
        fixture.read(".git/info/exclude"),
        exclude_before,
        "exclude block rewritten non-idempotently"
    );
    assert_eq!(git_stdout(&fixture, &["status", "--porcelain"]), "");
}

// A repo already carrying a shared, committed stele graph cannot also go undercover (§3.5).
#[test]
fn init_undercover_on_shared_graph_exits_2() {
    let fixture = Fixture::acme();
    let init = fixture.run(&["init", "--undercover"]);
    assert_eq!(init.code, 2, "expected exit 2:\n{}", init.combined());
    assert!(
        init.combined().contains("shared stele graph"),
        "message:\n{}",
        init.combined()
    );
    // Nothing was written: no marker, no overlay.
    assert!(!fixture.path(".stele/undercover").exists());
    assert!(!fixture.path(".stele/tree").exists());
}

// After undercover init, `build` discovers overlay-declared nodes and warns about none of
// them (the overlay is expected-untracked, §3.5).
#[test]
fn build_works_in_undercover_and_discovers_overlay_nodes() {
    let fixture = committed_two_dir_repo();
    assert_eq!(fixture.run(&["init", "--undercover"]).code, 0);

    // Author explicit overlay blocks (§3.5): the root system node and one container whose id
    // mirrors its tree path (`.stele/tree/apps/AGENTS.md` → node `apps`).
    fixture.write(
        ".stele/tree/AGENTS.md",
        "# root\n\n```stele\nkind: system\npurpose: undercover root\n```\n",
    );
    fixture.write(
        ".stele/tree/apps/AGENTS.md",
        "# apps\n\n```stele\nkind: container\npurpose: the apps overlay node\n```\n",
    );

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "build:\n{}", build.combined());
    // No untracked-node warning in undercover mode.
    assert!(
        !build.combined().contains("not tracked"),
        "build warned about the overlay:\n{}",
        build.combined()
    );

    // The private lock lists both overlay-declared node ids.
    let lock = fixture.read(".stele/graph.lock");
    assert!(
        lock.contains("\"apps\""),
        "lock omits the apps node:\n{lock}"
    );
    assert!(
        lock.contains("undercover root"),
        "lock omits the system purpose:\n{lock}"
    );

    // Still clean.
    assert_eq!(git_stdout(&fixture, &["status", "--porcelain"]), "");
}

// Without the marker, normal mode ignores an orphan `.stele/tree/AGENTS.md` — the overlay
// convention is inert unless undercover is selected.
#[test]
fn normal_mode_ignores_orphan_overlay_file() {
    let fixture = committed_two_dir_repo();
    // Scaffold a real tracked node so build has something to compile, and commit it.
    fixture.write(
        "AGENTS.md",
        "# proj\n\n```stele\nkind: system\npurpose: tracked root\n```\n",
    );
    fixture.commit("add a tracked root node");
    // An UNTRACKED overlay orphan carrying a distinctive purpose — written after the commit so
    // it stays outside VCS; with no marker present the repo is in normal mode.
    fixture.write(
        ".stele/tree/AGENTS.md",
        "# orphan\n\n```stele\nkind: system\npurpose: ORPHAN-OVERLAY-SENTINEL\n```\n",
    );

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "build:\n{}", build.combined());
    let lock = fixture.read(".stele/graph.lock");
    assert!(
        !lock.contains("ORPHAN-OVERLAY-SENTINEL"),
        "normal-mode build discovered the overlay orphan:\n{lock}"
    );
    assert!(
        lock.contains("tracked root"),
        "normal-mode build missed the tracked node:\n{lock}"
    );
}

// ─── Phase 4: emit / check guards / serve (§3.5) ─────────────────────────────

// emit renders regions into the overlay files and indexes at the home, and materializes the
// one `CLAUDE.local.md` shim at the work-tree root — all while `git status` stays empty.
#[test]
fn emit_undercover_writes_regions_indexes_and_local_shim() {
    let fixture = undercover_built();

    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 0, "emit:\n{}", emit.combined());

    // Regions rendered inside the overlay root node (the index pointer lines, §6 item 5).
    let overlay_root = fixture.read(".stele/tree/AGENTS.md");
    assert!(
        overlay_root.contains(".stele/index/invariants.md"),
        "overlay root region not rendered:\n{overlay_root}"
    );

    // Both transpose indexes land at the home under `.stele/index/`.
    assert!(
        fixture.path(".stele/index/invariants.md").exists(),
        "no invariants index"
    );
    assert!(
        fixture.path(".stele/index/hazards.md").exists(),
        "no hazards index"
    );

    // The single materialized file: `CLAUDE.local.md` at the work-tree root, a same-checkout
    // relative import of the overlay root node.
    assert_eq!(
        fixture.read("CLAUDE.local.md"),
        "@.stele/tree/AGENTS.md\n",
        "undercover shim content"
    );

    // The leak invariant: the exclude block covers the overlay, the indexes, and the shim.
    assert_eq!(
        git_stdout(&fixture, &["status", "--porcelain"]),
        "",
        "emit left the work tree dirty"
    );
}

// An operator's pre-existing `CLAUDE.local.md` is never overwritten (only-if-absent, §3.5).
#[test]
fn emit_undercover_respects_existing_claude_local() {
    let fixture = undercover_built();
    let authored = "# my private notes\n\n@somewhere/else.md\n";
    fixture.write("CLAUDE.local.md", authored);

    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 0, "emit:\n{}", emit.combined());
    assert_eq!(
        fixture.read("CLAUDE.local.md"),
        authored,
        "emit clobbered a hand-authored CLAUDE.local.md"
    );
}

// `emit --claude-rules` has no home in undercover mode: the single materialized file is the
// shim, so the multi-file projection is an input error (exit 2, §3.5).
#[test]
fn emit_claude_rules_rejected_in_undercover_exit_2() {
    let fixture = undercover_built();
    let emit = fixture.run(&["emit", "--claude-rules"]);
    assert_eq!(emit.code, 2, "expected exit 2:\n{}", emit.combined());
    assert!(
        emit.combined().contains("unavailable in undercover mode"),
        "message:\n{}",
        emit.combined()
    );
    // Nothing was rendered under `.claude/rules/`.
    assert!(
        !fixture.path(".claude/rules").exists(),
        "claude-rules leaked a directory despite the rejection"
    );
}

// A tracked, shared committed graph appearing while undercover is mutual-exclusion exit 2 at
// BOTH build and check — and build leaves the pre-existing private lock byte-for-byte intact.
#[test]
fn check_and_build_exit_2_on_tracked_shared_graph_while_undercover() {
    let fixture = undercover_built();
    let lock_before = fixture.read(".stele/graph.lock");

    // A collaborator commits a shared, tracked stele-block AGENTS.md at the tree root.
    fixture.write(
        "AGENTS.md",
        "# shared\n\n```stele\nkind: system\npurpose: a tracked shared graph\n```\n",
    );
    fixture.commit("adopt a tracked shared stele graph");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 2, "build must exit 2:\n{}", build.combined());
    assert!(
        build.combined().contains("shared stele graph"),
        "build message:\n{}",
        build.combined()
    );
    // No partial lock: the private lock is unchanged (§5.3 build atomicity).
    assert_eq!(
        fixture.read(".stele/graph.lock"),
        lock_before,
        "build rewrote the private lock despite the conflict"
    );

    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 2, "check must exit 2:\n{}", check.combined());
    assert!(
        check.combined().contains("shared stele graph"),
        "check message:\n{}",
        check.combined()
    );
}

// The inverse advisory: an UNTRACKED work-tree AGENTS.md that carries a stele block while
// undercover is a confusion signal (build reads the overlay, not the tree) — stderr-only, exit
// untouched.
#[test]
fn advisory_fires_for_untracked_tree_stele_block_in_undercover() {
    let fixture = undercover_built();
    // Authored in the work tree, never committed — outside the overlay build actually reads.
    fixture.write(
        "packages/shared/AGENTS.md",
        "# stray\n\n```stele\nkind: container\npurpose: authored in the wrong place\n```\n",
    );

    let build = fixture.run(&["build"]);
    assert_eq!(
        build.code,
        0,
        "build must still succeed:\n{}",
        build.combined()
    );
    assert!(
        build
            .combined()
            .contains("undercover mode reads node sources from .stele/tree/"),
        "overlay advisory missing:\n{}",
        build.combined()
    );
}

// The multi-worktree sharing property: from a linked worktree whose common dir is the main
// checkout's `.git`, every query resolves the home lock (the shared private graph).
#[test]
fn worktree_query_resolves_home_lock() {
    let fixture = undercover_built();

    // A linked worktree off the main checkout (Fixtures are real git repos, §common).
    let wt_home = tempfile::tempdir().expect("worktree temp dir");
    let wt_path = wt_home.path().join("feature");
    git_stdout(
        &fixture,
        &["worktree", "add", wt_path.to_str().expect("utf-8 path")],
    );

    // The queries run from the WORKTREE, but resolve the graph home (the main checkout).
    let root = run_in(&wt_path, &["root"]);
    assert_eq!(root.code, 0, "root from worktree:\n{}", root.combined());
    assert!(
        root.stdout.contains("undercover root"),
        "worktree root did not render the home graph:\n{}",
        root.stdout
    );
    let nodes = run_in(&wt_path, &["nodes"]);
    assert_eq!(nodes.code, 0, "nodes from worktree:\n{}", nodes.combined());
    assert!(
        nodes.stdout.contains("apps"),
        "worktree nodes did not render the home graph:\n{}",
        nodes.stdout
    );
}

// `emit` from a linked worktree lands `CLAUDE.local.md` in the WORKTREE root, and its relative
// `@`-import resolves back to the overlay root node at the shared home.
#[test]
fn shim_relative_import_from_linked_worktree() {
    let fixture = undercover_built();

    let wt_home = tempfile::tempdir().expect("worktree temp dir");
    let wt_path = wt_home.path().join("feature");
    git_stdout(
        &fixture,
        &["worktree", "add", wt_path.to_str().expect("utf-8 path")],
    );

    let emit = run_in(&wt_path, &["emit"]);
    assert_eq!(emit.code, 0, "emit from worktree:\n{}", emit.combined());

    // The shim lands in the worktree root, not the main checkout.
    let shim = wt_path.join("CLAUDE.local.md");
    assert!(shim.exists(), "no shim in the worktree root");
    assert!(
        !fixture.path("feature").exists() && !fixture.path("CLAUDE.local.md").exists(),
        "emit from the worktree leaked a shim into the main checkout"
    );

    // Assert by RESOLVING (not by exact string): the import path, joined onto the worktree
    // root, canonicalizes to the overlay root node at the home.
    let content = std::fs::read_to_string(&shim).expect("read shim");
    let rel = content.trim().trim_start_matches('@');
    let resolved = wt_path.join(rel).canonicalize().expect("resolve import");
    let overlay = fixture
        .path(".stele/tree/AGENTS.md")
        .canonicalize()
        .expect("canonicalize overlay");
    assert_eq!(
        resolved, overlay,
        "shim import does not resolve to the overlay root"
    );
}
