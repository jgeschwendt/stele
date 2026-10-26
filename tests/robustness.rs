//! Scan/parse robustness regressions (SPEC §2.4 scan scope, §3.1 region markers, §5.3
//! process contract). Each test drives the real `build`/`emit` pipeline over a git repo,
//! guarding a specific failure the rung-3 review found: a tracked binary aborting the
//! scan (F1), a fenced marker-lookalike clobbering authored bytes (F2), a concurrent
//! build racing on a shared temp lock (F13), and the one-line empty-region form being
//! rejected (F14).

mod common;

use common::Fixture;

/// Bytes that are not valid UTF-8 (a stand-in for a tracked PNG/font/icon): a lone
/// `0xff 0xfe` and the never-valid `0xc0 0xc1` continuation bytes.
const BINARY_BLOB: [u8; 12] = [
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0xff, 0xfe, 0x00, 0x01, 0xc0, 0xc1,
];

// ─── F1: a tracked binary must not abort the scan (§2.4) ──────────────────────

// Before the fix, `anchors::scan` read every tracked file with `read_to_string`, so a
// tracked binary blob aborted `build`/`check` with exit 3. It must be skipped silently.
#[test]
fn tracked_binary_file_does_not_abort_the_scan() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    std::fs::write(fixture.path("logo.png"), BINARY_BLOB).expect("write binary blob");
    fixture.commit("root node plus a tracked binary");

    // `migrate` skips it the same way (§5.1 scan scope).
    for verb in ["build", "check", "migrate"] {
        let run = fixture.run(&[verb]);
        assert_eq!(run.code, 0, "{verb}: {}", run.combined());
    }
}

// A node source (AGENTS.md) is NOT a binary blob to skip: invalid UTF-8 there is a hard
// §5.3 input error (exit 2) naming the file, never a silent drop of a declared node.
#[test]
fn invalid_utf8_node_source_is_a_hard_input_error() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    std::fs::create_dir_all(fixture.path("lib")).expect("create lib dir");
    std::fs::write(
        fixture.path("lib/AGENTS.md"),
        b"# lib \xff\xfe\n\n```stele\nkind: container\n```\n",
    )
    .expect("write invalid-utf8 node source");
    fixture.commit("a node source with invalid UTF-8");

    // `migrate` gives it the same hard stop (§5.3), before writing anything.
    for verb in ["build", "migrate"] {
        let run = fixture.run(&[verb]);
        assert_eq!(run.code, 2, "{verb}: {}", run.combined());
        assert!(
            run.stderr
                .contains("lib/AGENTS.md: node source is not valid UTF-8"),
            "the error must name the offending node source:\n{}",
            run.combined()
        );
    }
}

// ─── F2: marker-lookalikes inside a fence are quotations (§3.1) ───────────────

// A `@stele`/`@end` pair inside a fenced code block is literal example text,
// not a generated region. Before the fix, `find_region` matched it and `emit` overwrote
// the authored bytes between the fenced markers (silent data loss). A fence nested in a
// list item sits 4+ spaces in, past the fence scanner's indent limit — its marker lines
// are indented the same, so they are never markers either.
#[test]
fn region_markers_inside_a_fence_are_not_a_real_region() {
    for example in [
        "```markdown\n<!-- @stele -->\nHAND-AUTHORED EXAMPLE\n<!-- @end -->\n```\n",
        "1. Add these lines:\n\n    ```markdown\n    <!-- @stele -->\n    <!-- @end -->\n    ```\n",
    ] {
        let fixture = Fixture::bare();
        let authored = format!(
            "# root\n\n```stele\nkind: system\n```\n\nExample of the marker syntax:\n\n{example}"
        );
        fixture.write("AGENTS.md", &authored);
        fixture.commit("root whose only markers live inside a fence");

        assert_eq!(fixture.run(&["build"]).code, 0);

        // No real region: emit points at `stele init` (exit 2) rather than rendering into
        // the fenced example.
        let emit = fixture.run(&["emit"]);
        assert_eq!(emit.code, 2, "{}", emit.combined());
        assert!(
            emit.combined().to_lowercase().contains("init"),
            "{}",
            emit.combined()
        );
        // The authored bytes are untouched — the data loss the fix prevents.
        assert_eq!(fixture.read("AGENTS.md"), authored);
    }
}

// ─── F13: concurrent builds must not collide on the temp lock (§5.3) ──────────

// The number of builds raced in one repo. A fixed temp name made the loser's rename
// ENOENT (exit 3); enough parallelism reliably provoked the old collision.
const CONCURRENT_BUILDS: usize = 24;

#[test]
fn concurrent_builds_do_not_collide_on_the_temp_lock() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.commit("root node");

    let codes: Vec<i32> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..CONCURRENT_BUILDS)
            .map(|_| scope.spawn(|| fixture.run(&["build"]).code))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    assert!(
        codes.iter().all(|&c| c == 0),
        "a concurrent build failed (temp-lock collision): {codes:?}"
    );
}

// ─── F14: the one-line empty-region form is accepted (§3.1/§7) ────────────────

// The begin marker's annotation ends at the FIRST `-->`, so an end marker glued onto the
// same line is a valid empty region — not a begin-without-end error (exit 2).
#[test]
fn one_line_empty_region_form_is_accepted() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write("lib/AGENTS.md", "# lib\n\n```stele\nkind: container\n```\n");
    fixture.commit("root plus an empty container");

    // Scaffold the regions — `init` writes the bare one-line form (§7) — then
    // materialize everything so the graph is fully up to date.
    assert_eq!(fixture.run(&["init"]).code, 0);
    assert!(
        fixture
            .read("lib/AGENTS.md")
            .contains("<!-- @stele --><!-- @end -->"),
        "init did not scaffold the bare one-line region: {}",
        fixture.read("lib/AGENTS.md")
    );
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(fixture.run(&["emit", "--check"]).code, 0);
    fixture.commit("one-line empty region form");

    // It parses as an empty region, so the container is still up to date (no exit 2).
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());

    // emit reproduces the one-line form byte-identically (empty container region).
    let before = fixture.read("lib/AGENTS.md");
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(
        fixture.read("lib/AGENTS.md"),
        before,
        "emit rewrote the one-line region"
    );
}

// ─── a leading UTF-8 BOM is not content (§2.4, §3.1) ───────────────────────────

// A Windows editor's BOM (U+FEFF) before line 1 hid a first-line ```stele fence (the
// node dropped out, so `init` prepended a second skeleton), failed a package.json's JSON
// parse (its package drew no edge), and glued itself to `.steleignore`'s first pattern.
#[test]
fn leading_bom_is_skipped_in_node_package_and_ignore_files() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "\u{feff}```stele\nkind: system\n```\n");
    fixture.write(".steleignore", "\u{feff}ghost/\n");
    fixture.write("ghost/AGENTS.md", "```stele\nkind: container\n```\n");
    for name in ["shared", "web"] {
        fixture.write(
            &format!("{name}/AGENTS.md"),
            "```stele\nkind: component\n```\n",
        );
    }
    fixture.write(
        "shared/package.json",
        "\u{feff}{ \"name\": \"@acme/shared\", \"main\": \"index.ts\" }\n",
    );
    fixture.write("shared/index.ts", "export const m = 1;\n");
    fixture.write(
        "web/index.ts",
        "import { m } from \"@acme/shared\";\nexport { m };\n",
    );
    fixture.commit("BOM-prefixed node, package and ignore files");

    assert_eq!(fixture.run(&["init"]).code, 0);
    assert_eq!(fixture.read("AGENTS.md").matches("kind:").count(), 1);
    assert_eq!(fixture.run(&["build"]).code, 0);
    let lock = stele::lock::parse_lock(&fixture.read(".stele/graph.lock")).unwrap();
    assert_eq!(
        lock.nodes.keys().collect::<Vec<_>>(),
        ["/", "shared", "web"]
    );
    assert_eq!(lock.nodes["web"].extracted.imports, ["shared"]);
}

// ─── an absent tracked file skips instead of aborting (§2.4, §5.3) ────────────

// A tracked source file deleted from the working tree WITHOUT staging the removal
// (`rm app.rs`, no `git rm`) still appears in `git ls-files`, so the scan/extractor
// tries to read a path that is gone. Before the fix `Ctx::read` (and the anchor scan)
// mapped that ENOENT to a §5.3 internal error (exit 3), aborting `build`/`check`. The
// file must be skipped — the extractor sees nothing, a claim anchored there falls to
// Unresolved (§4.1) — never a fatal exit 3.
#[test]
fn tracked_source_deleted_without_staging_does_not_abort() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write("app.rs", "pub fn f() {}\n");
    fixture.commit("root node plus a tracked rust source");

    // Delete the working-tree file but leave the removal unstaged — git still tracks it.
    fixture.delete_file("app.rs");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    assert_ne!(
        check.code,
        3,
        "an absent tracked file must never abort:\n{}",
        check.combined()
    );
    assert!(
        matches!(check.code, 0 | 1),
        "check exited {} (want 0 or an honest finding):\n{}",
        check.code,
        check.combined()
    );
}

// A pathologically nested source must not overflow the stack: every tree-sitter walk
// (comment scan, import extraction, symbol and landmark binding, the §4.5 digest) is
// iterative, so 20 000 nested parens cost heap, never a stack-overflow abort.
#[test]
fn deeply_nested_source_does_not_overflow_the_stack() {
    let fixture = Fixture::bare();
    fixture.write(
        "AGENTS.md",
        "# root\n\n```stele\nkind: system\ninvariants:\n  - claim: f nests\n    anchor: deep.rs#f\n  \
         - claim: g nests\n    anchor: ※ deep-g\n```\n",
    );
    let nest = |body: &str| format!("{}{body}{}", "(".repeat(20_000), ")".repeat(20_000));
    fixture.write(
        "deep.rs",
        &format!(
            "use std::fmt;\nfn f() -> i32 {{ {} }}\n// ※ deep-g\nfn g() -> i32 {{ {} }}\n",
            nest("1"),
            nest("2")
        ),
    );
    fixture.commit("a root claim pair over a deeply nested rust source");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// A committed dangling symlink whose name carries a source extension (`dead.rs` →
// nonexistent target) is tracked, so the scan follows it and reads ENOENT. It must
// skip, never abort `build` (companion to the deleted-file case above).
#[cfg(unix)]
#[test]
fn committed_dangling_symlink_with_source_ext_does_not_abort() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    std::os::unix::fs::symlink("nonexistent-target", fixture.path("dead.rs"))
        .expect("create dangling symlink");
    fixture.commit("root node plus a committed dangling .rs symlink");

    let build = fixture.run(&["build"]);
    assert_eq!(
        build.code,
        0,
        "a committed dangling symlink must not abort build:\n{}",
        build.combined()
    );
}

// `init` never writes through a tracked symlink: an AGENTS.md linking out of the tree
// (a block with no region, or prose only) is left as is, its target byte-identical.
#[cfg(unix)]
#[test]
fn init_never_writes_through_a_symlinked_agents_file() {
    let fixture = Fixture::bare();
    let outside = tempfile::tempdir().unwrap();
    let block = "# x\n\n```stele\nkind: system\n```\n";
    std::fs::write(outside.path().join("root.md"), block).unwrap();
    std::fs::write(outside.path().join("lib.md"), "# lib\n").unwrap();
    std::fs::create_dir(fixture.path("lib")).unwrap();
    for (link, target) in [("AGENTS.md", "root.md"), ("lib/AGENTS.md", "lib.md")] {
        std::os::unix::fs::symlink(outside.path().join(target), fixture.path(link)).unwrap();
    }
    fixture.commit("AGENTS.md files linking out of the tree");

    let init = fixture.run(&["init"]);
    assert_eq!(init.code, 0, "{}", init.combined());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("root.md")).unwrap(),
        block
    );
    assert_eq!(
        std::fs::read_to_string(outside.path().join("lib.md")).unwrap(),
        "# lib\n"
    );
}

// …but an AGENTS.md linking INSIDE the tree is scaffolded at its target, the link kept: a
// block gets its region (AGENTS.md → CLAUDE.md), prose its skeleton (lib/AGENTS.md → NOTES.md).
#[cfg(unix)]
#[test]
fn init_writes_through_an_in_tree_symlinked_agents_file() {
    let fixture = Fixture::bare();
    fixture.write("CLAUDE.md", "```stele\nkind: system\n```\n");
    fixture.write("lib/NOTES.md", "# lib\n");
    for (link, target) in [("AGENTS.md", "CLAUDE.md"), ("lib/AGENTS.md", "NOTES.md")] {
        std::os::unix::fs::symlink(target, fixture.path(link)).unwrap();
    }
    fixture.commit("AGENTS.md files linking inside the tree");

    assert_eq!(fixture.run(&["init"]).code, 0);
    assert!(fixture.path("AGENTS.md").is_symlink() && fixture.path("lib/AGENTS.md").is_symlink());
    assert!(
        fixture
            .read("CLAUDE.md")
            .ends_with("<!-- @stele --><!-- @end -->\n")
    );
    assert!(fixture.read("lib/NOTES.md").contains("kind: container"));
}

// ─── a stele verb outside a git repo is a clean input error (§5.3) ────────────

// Outside a work tree `git ls-files` prints "fatal: not a git repository" and exits
// non-zero. Before the fix that raw text surfaced as a §5.3 internal error (exit 3); a
// user running stele in the wrong directory is an input mistake (exit 2) with a
// one-line explanation, not a leaked `fatal:` line.
#[test]
fn stele_verb_outside_a_git_repo_is_a_clean_input_error() {
    let dir = tempfile::tempdir().expect("create non-git temp dir");
    for args in [&["build"][..], &["init", "--undercover"]] {
        let out = std::process::Command::new(common::BIN)
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("spawn stele binary");
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?} must exit 2 (input error), not 3"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("not a git repository"),
            "message must explain the condition:\n{stderr}"
        );
        assert!(
            !stderr.contains("fatal:"),
            "raw git text must not leak:\n{stderr}"
        );
    }
}

// ─── an IO failure is exit 3, naming what failed (§5.3) ───────────────────────

// Each row plants one unreadable or unwritable path — a directory where a file belongs, a
// corrupt git index — and runs the verb that trips on it. An exclude file git cannot read
// passes `git ls-files` and fails only the untracked-file listing.
#[test]
fn io_failures_are_exit_3() {
    for (plant, verb, says) in [
        (
            ".stele/graph.lock/x",
            "build",
            "rename into .stele/graph.lock: ",
        ),
        (".stele/graph.lock/x", "check", "read .stele/graph.lock: "),
        ("apps/AGENTS.md/x", "init", "read apps/AGENTS.md: "),
        ("AGENTS.md/x", "emit", "read AGENTS.md: "),
        (".git/index", "build", "`git ls-files` failed: "),
        (
            ".git/info/exclude/x",
            "build",
            "`git ls-files --others` failed: fatal: cannot use .git/info/exclude as an exclude file",
        ),
    ] {
        let fixture = Fixture::bare();
        fixture.write("AGENTS.md", "```stele\nkind: system\n```\n");
        fixture.write("apps/app.ex", "defmodule App do\nend\n");
        fixture.commit("a root node and one top-level dir");
        if verb == "emit" {
            assert_eq!(fixture.run(&["build"]).code, 0);
        }
        // The file the plant's directory replaces, if any (`AGENTS.md`, `info/exclude`).
        let _ = std::fs::remove_file(fixture.path(plant).parent().unwrap());
        fixture.write(plant, "not what this path should hold\n");
        let run = fixture.run(&[verb]);
        assert_eq!(run.code, 3, "{plant} {verb}: {}", run.combined());
        assert!(run.stderr.contains(says), "{plant} {verb}: {}", run.stderr);
    }
}

// A `<path>#<symbol>` anchor's path is repo-root-relative (§2.4): an absolute path or a
// `..` segment never reaches the filesystem, so `build` records no `resolved` (no file
// read, no digest — no oracle for files outside the repo) and `check` names why. Each
// probe escapes and re-enters the repo, so an unchecked join WOULD bind it.
#[test]
fn anchor_paths_outside_the_repo_never_resolve() {
    let fixture = Fixture::bare();
    let dir = fixture.root.file_name().unwrap().to_str().unwrap();
    let absolute = fixture.path("m.py");
    for anchor in [
        format!("../{dir}/m.py#secret_fn"),
        format!("{}#secret_fn", absolute.display()),
        format!("../{dir}/notes.md#setup"),
    ] {
        fixture.write(
            "AGENTS.md",
            &format!("```stele\nkind: system\ninvariants:\n  - claim: escapes\n    anchor: {anchor}\n```\n"),
        );
        fixture.write("m.py", "def secret_fn():\n    pass\n");
        fixture.write("notes.md", "## Setup\n");
        fixture.commit("an anchor whose path leaves the repo");
        assert_eq!(fixture.run(&["build"]).code, 0, "{anchor}");
        let lock = fixture.read(".stele/graph.lock");
        assert!(lock.contains("\"resolved\": null"), "{anchor}\n{lock}");
        let check = fixture.run(&["check"]);
        let out = check.combined();
        assert_eq!(check.code, 1, "{anchor}\n{out}");
        assert!(
            out.contains("is not a repo-root-relative path"),
            "{anchor}\n{out}"
        );
    }
}

// An ADR is scanned like any tracked file (§2.6): a Latin-1 byte decodes lossily (only its
// status token matters), and a dangling symlink or an unstaged delete is skipped — never an
// exit-3 abort of `build`/`check` (a delete after the build stales the lock: exit 2).
#[cfg(unix)]
#[test]
fn unreadable_adrs_never_abort() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write("adr/0001-a.md", "# 1\n\nStatus: accepted\n");
    std::fs::write(
        fixture.path("adr/0002-b.md"),
        b"# 2\n\nStatus: rejected\nAuthor: Ren\xe9\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("../gone.md", fixture.path("adr/0003-moved.md")).unwrap();
    fixture.commit("a Latin-1 ADR and a dangling one");
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let lock = fixture.read(".stele/graph.lock");
    assert!(
        lock.contains("\"rejected\"") && !lock.contains("0003"),
        "{lock}"
    );
    fixture.commit("lock");
    std::fs::remove_file(fixture.path("adr/0001-a.md")).unwrap();
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 2, "{}", check.combined());
    assert!(
        check.stderr.contains("run stele build"),
        "{}",
        check.combined()
    );
}

// A node source gone from the work tree (an unstaged `rm`) is §5.3's "a lock node whose
// file is gone": an input error (exit 2) naming it, never an internal exit 3.
#[test]
fn a_gone_node_source_is_an_input_error() {
    let fixture = Fixture::bare();
    fixture.write("AGENTS.md", "# root\n\n```stele\nkind: system\n```\n");
    fixture.write("lib/AGENTS.md", "# lib\n\n```stele\nkind: container\n```\n");
    fixture.commit("two nodes");
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("lock");
    std::fs::remove_file(fixture.path("lib/AGENTS.md")).unwrap();
    for verb in ["build", "check", "init"] {
        let run = fixture.run(&[verb]);
        assert_eq!(run.code, 2, "{verb}: {}", run.combined());
        assert!(
            run.stderr.contains("lib/AGENTS.md: node source is gone"),
            "{verb}: {}",
            run.combined()
        );
    }
}
