//! Referenced-not-scanned paths (SPEC §4.1d): an `enforced_by` guard is a path the user
//! names, so `.steleignore` — which hides paths from source scanning (§2.4) — never makes
//! a git-tracked guard read as untracked.

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
