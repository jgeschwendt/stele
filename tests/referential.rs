//! Paths the user names resolve as named: an `enforced_by` guard (SPEC §4.1d) is not
//! scanned, so `.steleignore` (§2.4) never makes a git-tracked guard read as untracked;
//! a `/`-bearing `exhaustiveness.exclude` glob (§3.4) matches the directory path.

mod common;

use common::Fixture;

/// A tracked guard under a steleignored `vendor/` passes §4.1d.
#[test]
fn steleignored_tracked_guard_satisfies_enforced_by() {
    let fixture = Fixture::bare();
    fixture.write(
        "AGENTS.md",
        "# root\n\n```stele\nkind: system\ninvariants:\n  - claim: capped\n    anchor: ※ cap\n    enforced_by: vendor/guard.sh\n```\n",
    );
    fixture.write("a.sh", "# ※ cap\nf() { :; }\n");
    fixture.write("vendor/guard.sh", "true\n");
    fixture.write(".steleignore", "vendor/\n");
    fixture.commit("guarded");
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("lock");
    let check = fixture.run(&["check", "--only", "referential"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

/// `exclude = ["apps/legacy"]` silences exactly the path the §4.3 fix line names.
#[test]
fn path_shaped_exclude_drops_that_directory() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write(
        "apps/web/AGENTS.md",
        "# web\n\n```stele\nkind: container\n```\n",
    );
    fixture.write("apps/legacy/a.txt", "");
    fixture.write(
        ".stele/config.toml",
        "[exhaustiveness]\nexclude = [\"apps/legacy\"]\n",
    );
    fixture.commit("legacy");
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check", "--only", "exhaustiveness"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}
