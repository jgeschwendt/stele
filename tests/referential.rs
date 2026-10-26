//! Paths the user names resolve as named: an `enforced_by` guard (SPEC §4.1d) is not
//! scanned, so `.steleignore` (§2.4) never makes a git-tracked guard read as untracked, nor
//! an inherited `GIT_LITERAL_PATHSPECS=1`;
//! an `exhaustiveness.exclude` glob (§3.4) matches as its gitignore spelling reads.

mod common;

use common::{BIN, Fixture, isolate_git};

/// A tracked guard under a steleignored `vendor/` passes §4.1d, whatever git's pathspec env.
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
    let literal = isolate_git(std::process::Command::new(BIN).current_dir(fixture.path("")))
        .args(["check", "--only", "referential"])
        .env("GIT_LITERAL_PATHSPECS", "1")
        .status()
        .unwrap();
    assert!(literal.success());
}

/// An `exhaustiveness.exclude` entry reads as a gitignore line (§4.3): `./`, a leading or
/// trailing `/` and a trailing `/**` change no match, a slash anchors at the root and `**/`
/// matches at any depth. Each spelling silences exactly its directory, never another.
#[test]
fn gitignore_spelled_excludes_drop_their_directory() {
    for (entry, silenced) in [
        ("apps/legacy", true),
        ("apps/legacy/", true),
        ("./apps/legacy", true),
        ("/apps/legacy/**", true),
        ("**/apps/legacy", true),
        ("**/legacy", true),
        ("legacy/", true),
        ("legacy", true),
        ("/legacy", false),
        ("legacy/**", false),
        ("**/web/legacy", false),
    ] {
        let fixture = Fixture::bare();
        fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
        fixture.write(
            "apps/web/AGENTS.md",
            "# web\n\n```stele\nkind: container\n```\n",
        );
        fixture.write("apps/legacy/a.txt", "");
        fixture.write(
            ".stele/config.toml",
            &format!("[exhaustiveness]\nexclude = [\"{entry}\"]\n"),
        );
        fixture.commit("legacy");
        assert_eq!(fixture.run(&["build"]).code, 0);
        let check = fixture.run(&["check", "--only", "exhaustiveness"]);
        assert_eq!(check.code == 0, silenced, "{entry}: {}", check.combined());
    }
}
