//! `.steleignore` scan exclusion (SPEC §2.4): a committed root `.steleignore` makes a
//! subtree invisible to EVERY source scan at once — node discovery (§3.1), the anchor
//! scan (§2.4), import extraction (§4.2), and the §4.3 exhaustiveness walk. These tests
//! drive the whole `build`→`check` pipeline over a real git repo whose `vendor/` subtree
//! carries its own node, a landmark, and a boundary-crossing import, then prove all of it
//! vanishes when ignored and returns when un-ignored.

mod common;

use common::Fixture;

/// A repo whose `vendor/` subtree declares a node, a landmark, and an import that crosses
/// into the `app` node — every scan has something to find there. `.steleignore` decides
/// whether it is visible. Returns the committed fixture.
fn repo() -> Fixture {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write("app/AGENTS.md", "# app\n\n```stele\nkind: container\n```\n");
    fixture.write("app/core.py", "def x():\n    return 1\n");
    // vendor imports app.core (a boundary-crossing import) and declares a landmark; with
    // no `depends` edge, that import is a structural violation IF the scan can see it.
    fixture.write(
        "vendor/AGENTS.md",
        "# vendor\n\n```stele\nkind: container\n```\n",
    );
    fixture.write(
        "vendor/main.py",
        "# stele:landmark vendor-mark\nfrom app.core import x\n",
    );
    fixture.commit("materialize a repo with a vendor subtree");
    fixture
}

// An ignored subtree contributes no node, no landmark, and no import — so `check` is clean
// (nothing unmapped, nothing crossing a boundary).
#[test]
fn ignored_subtree_is_invisible_to_every_scan() {
    let fixture = repo();
    fixture.write(
        ".steleignore",
        "# the vendored tree is not part of the graph\nvendor/\n",
    );
    fixture.commit("ignore the vendor subtree");

    assert_eq!(fixture.run(&["build"]).code, 0);

    // Node discovery: no `vendor` node.
    let nodes = fixture.run(&["nodes"]);
    assert_eq!(nodes.code, 0, "{}", nodes.combined());
    assert!(nodes.combined().contains("app"), "{}", nodes.combined());
    assert!(
        !nodes.combined().contains("vendor"),
        "ignored subtree still declared a node:\n{}",
        nodes.combined()
    );

    // Anchor scan: the vendor landmark never entered the lock.
    let lock = fixture.read(".stele/graph.lock");
    assert!(
        !lock.contains("vendor-mark"),
        "ignored subtree's landmark leaked into the lock"
    );

    // Import extraction + exhaustiveness: no boundary-crossing import, no unmapped dir.
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// Removing the ignore restores the node, the landmark, and the import — the import now
// crosses `vendor → app` with no `depends`, so structural fails (proving imports returned).
#[test]
fn un_ignoring_restores_the_whole_subtree() {
    let fixture = repo();
    fixture.write(".steleignore", "vendor/\n");
    fixture.commit("ignore the vendor subtree");
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["check"]).code, 0);

    // Un-ignore (empty the file, keep it on disk so the tracked file still reads) + rebuild.
    fixture.write(".steleignore", "# nothing ignored now\n");
    assert_eq!(fixture.run(&["build"]).code, 0);

    let nodes = fixture.run(&["nodes"]);
    assert!(
        nodes.combined().contains("vendor"),
        "un-ignoring did not restore the node:\n{}",
        nodes.combined()
    );
    let lock = fixture.read(".stele/graph.lock");
    assert!(
        lock.contains("vendor-mark"),
        "un-ignoring did not restore the landmark"
    );

    // The vendor→app import is now visible and uncovered: structural fires (exit 1).
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 1, "{}", check.combined());
    assert!(
        check.combined().contains("vendor")
            && check.combined().to_lowercase().contains("structural"),
        "expected a structural violation from the restored import:\n{}",
        check.combined()
    );
}
