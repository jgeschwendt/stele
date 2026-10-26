//! Paths the user names resolve as named: an `enforced_by` guard (SPEC §4.1d) is not
//! scanned, so `.steleignore` (§2.4) never makes a git-tracked guard read as untracked, nor
//! an inherited `GIT_*_PATHSPECS=1`;
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
    for var in [
        "GIT_LITERAL_PATHSPECS",
        "GIT_GLOB_PATHSPECS",
        "GIT_ICASE_PATHSPECS",
    ] {
        let status = isolate_git(std::process::Command::new(BIN).current_dir(fixture.path("")))
            .args(["check", "--only", "referential"])
            .env(var, "1")
            .status()
            .unwrap();
        assert!(status.success(), "{var}=1");
    }
    // Mid-merge, git lists the conflicted guard once per stage; it is still tracked.
    fixture.git(&["checkout", "-q", "-b", "side"]);
    fixture.write("vendor/guard.sh", "side\n");
    fixture.commit("side");
    fixture.git(&["checkout", "-q", "-"]);
    fixture.write("vendor/guard.sh", "main\n");
    fixture.commit("main");
    let merge = isolate_git(std::process::Command::new("git").current_dir(fixture.path("")))
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "merge",
            "-q",
            "side",
        ])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let conflicted = fixture.run(&["check", "--only", "referential"]);
    assert_eq!(conflicted.code, 0, "{}", conflicted.combined());
}

/// A blank `enforced_by` names no guard: build rejects it (exit 2) rather than letting the
/// claim read as tracked and dodge digest staling.
#[test]
fn blank_enforced_by_is_rejected_at_build() {
    let fixture = Fixture::bare();
    fixture.write(
        "AGENTS.md",
        "# root\n\n```stele\nkind: system\ninvariants:\n  - claim: capped\n    anchor: ※ cap\n    enforced_by: ' '\n```\n",
    );
    fixture.write("a.sh", "# ※ cap\n");
    fixture.commit("blank guard");
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 2, "{}", build.combined());
    assert!(
        build.combined().contains("enforced_by"),
        "{}",
        build.combined()
    );
}

/// Two ADR files sharing a number make `§ 0003` ambiguous (the lock keeps one): check
/// names both, whichever status each carries.
#[test]
fn duplicate_adr_number_names_both_files() {
    for (a, b) in [("Accepted", "Superseded"), ("Superseded", "Accepted")] {
        let fixture = Fixture::bare();
        fixture.write(
            "AGENTS.md",
            "# root\n\n```stele\nkind: system\nedges:\n  decided_by: [\"§ 0003\"]\n```\n",
        );
        fixture.write("adr/0003-a.md", &format!("# a\n\nStatus: {a}\n"));
        fixture.write("adr/0003-b.md", &format!("# b\n\nStatus: {b}\n"));
        fixture.commit("twin adrs");
        assert_eq!(fixture.run(&["build"]).code, 0);
        let out = fixture.run(&["check", "--only", "referential"]).combined();
        assert!(
            out.contains("adr/0003 is declared by 2 files: adr/0003-a.md, adr/0003-b.md"),
            "{out}"
        );
    }
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

/// A `<path>#<symbol>` anchor binds only a path git tracks byte for byte (§2.4): a
/// gitignored file or a case-folded spelling is unresolved even where the disk opens it.
#[test]
fn symbol_anchor_into_an_untracked_or_case_folded_path_is_unresolved() {
    for anchor in ["gen/api.py#handler", "src/M.py#handler"] {
        let fixture = Fixture::bare();
        fixture.write(
            "AGENTS.md",
            &format!("# root\n\n```stele\nkind: system\ninvariants:\n  - claim: handled\n    anchor: {anchor}\n```\n"),
        );
        fixture.write(".gitignore", "gen/\n");
        fixture.write("gen/api.py", "def handler():\n    return 1\n");
        fixture.write("src/m.py", "def handler():\n    return 1\n");
        fixture.commit("import");
        assert_eq!(fixture.run(&["build"]).code, 0);
        fixture.commit("lock");
        let check = fixture.run(&["check", "--only", "referential"]);
        let out = check.combined();
        assert_eq!(check.code, 1, "{anchor}: {out}");
        assert!(
            out.contains(&format!("anchor {anchor} unresolved")),
            "{out}"
        );
    }
}
