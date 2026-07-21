//! Adoption diagnostics (SPEC §2.4 scan scope): `build`/`check` warn on stderr when an
//! untracked AGENTS.md declares a stele node. The scan is scoped to VCS-tracked files, so
//! an agent that hand-creates an AGENTS.md with a stele block but never `git add`s it
//! leaves the graph silently a node short — the warning is the only signal. The warning is
//! purely diagnostic: stdout, the exit code, and the `--json` envelope stay byte-identical
//! (machine consumers are unaffected). These tests pin the four boundary cases.

mod common;

use common::Fixture;

/// The stderr substring every untracked-node warning carries.
const WARNING_MARK: &str = "declares a stele node but is not tracked";

/// A committed root system node plus an UNTRACKED `lib/AGENTS.md` with a stele block:
/// `build` warns on stderr, still exits 0, and the lock omits the untracked node.
#[test]
fn untracked_node_file_warns_but_build_stays_clean() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.commit("root node only");

    // Hand-created but never staged: the exact adoption trap.
    fixture.write("lib/AGENTS.md", "# lib\n\n```stele\nkind: container\n```\n");

    let build = fixture.run(&["build"]);
    assert_eq!(
        build.code,
        0,
        "build must still exit 0:\n{}",
        build.combined()
    );
    assert!(
        build.stderr.contains(WARNING_MARK) && build.stderr.contains("lib/AGENTS.md"),
        "stderr must name the untracked node file:\n{}",
        build.stderr
    );
    // stdout stays a clean success line — the warning is stderr-only.
    assert!(
        !build.stdout.contains(WARNING_MARK),
        "warning must not leak onto stdout:\n{}",
        build.stdout
    );

    // The node is absent from the graph the lock records.
    let nodes = fixture.run(&["nodes"]);
    assert_eq!(nodes.code, 0, "{}", nodes.combined());
    assert!(
        !nodes.stdout.contains("lib"),
        "the untracked node must be absent from the lock:\n{}",
        nodes.stdout
    );

    // `check` warns on the same trap and still reflects an in-spec lock (exit 0): the
    // untracked node leaves the committed lock matching the freshly-built graph.
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
    assert!(
        check.stderr.contains(WARNING_MARK) && check.stderr.contains("lib/AGENTS.md"),
        "check must warn about the same untracked node:\n{}",
        check.stderr
    );
}

/// After `git add`, the file is tracked: the warning is gone and the node is present.
#[test]
fn tracking_the_node_file_clears_the_warning_and_adds_the_node() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.commit("root node only");
    fixture.write("lib/AGENTS.md", "# lib\n\n```stele\nkind: container\n```\n");

    // Stage the node file — `git ls-files` now sees it (tracked), so the tracked-file scan
    // picks it up and the untracked-diagnostic no longer fires.
    fixture.stage_all();

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    assert!(
        !build.combined().contains(WARNING_MARK),
        "a tracked node file must not warn:\n{}",
        build.combined()
    );

    let nodes = fixture.run(&["nodes"]);
    assert!(
        nodes.stdout.contains("lib"),
        "the now-tracked node must be present in the lock:\n{}",
        nodes.stdout
    );
}

/// An untracked AGENTS.md with NO stele block is plain-markdown degradation, not a node —
/// it earns no warning.
#[test]
fn untracked_blockless_agents_file_does_not_warn() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.commit("root node only");

    // Prose only, no stele fence.
    fixture.write("docs/AGENTS.md", "# docs\n\nJust notes, no node here.\n");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    assert!(
        !build.combined().contains(WARNING_MARK),
        "a blockless AGENTS.md must not warn:\n{}",
        build.combined()
    );
}

/// A steleignored untracked node file is already invisible to every scan (§2.4), so it
/// earns no warning either.
#[test]
fn steleignored_untracked_node_file_does_not_warn() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write(".steleignore", "vendor/\n");
    fixture.commit("root node plus a steleignore for vendor/");

    // Untracked node file under a steleignored subtree.
    fixture.write(
        "vendor/AGENTS.md",
        "# vendor\n\n```stele\nkind: container\n```\n",
    );

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    assert!(
        !build.combined().contains(WARNING_MARK),
        "a steleignored untracked node file must not warn:\n{}",
        build.combined()
    );
}
