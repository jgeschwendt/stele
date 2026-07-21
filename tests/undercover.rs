//! SPEC §3.5 undercover mode — Phase 3: `init --undercover` (marker + overlay scaffold +
//! `info/exclude` block) and the mode-aware node roster that lets `build` discover overlay
//! nodes. The invariant under test throughout: nothing the engine writes ever surfaces in
//! `git status` — the overlay lives at the graph home under `.stele/`, hidden by the shared
//! common-dir exclude block, and `init --undercover` never `git add`s. The broader matrix
//! (grove worktrees, emit/serve, per-worktree freshness) lands in Phase 5.

mod common;

use common::Fixture;
use std::process::Command;

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
