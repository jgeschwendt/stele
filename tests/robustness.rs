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

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
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

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 2, "{}", build.combined());
    assert!(
        build.combined().contains("lib/AGENTS.md"),
        "the error must name the offending node source:\n{}",
        build.combined()
    );
}

// ─── F2: marker-lookalikes inside a fence are quotations (§3.1) ───────────────

// A `stele:begin`/`stele:end` pair inside a fenced code block is literal example text,
// not a generated region. Before the fix, `find_region` matched it and `emit` overwrote
// the authored bytes between the fenced markers (silent data loss).
#[test]
fn region_markers_inside_a_fence_are_not_a_real_region() {
    let fixture = Fixture::bare();
    let authored = "# root\n\n\
         ```stele\nkind: system\n```\n\n\
         Example of the marker syntax:\n\n\
         ```markdown\n\
         <!-- stele:begin router -->\n\
         HAND-AUTHORED EXAMPLE — emit must never clobber this\n\
         <!-- stele:end -->\n\
         ```\n";
    fixture.write("AGENTS.md", authored);
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

    // Scaffold the canonical two-line regions, then materialize everything so the graph
    // is fully up to date (the baseline the one-line rewrite must preserve).
    assert_eq!(fixture.run(&["init"]).code, 0);
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(fixture.run(&["emit", "--check"]).code, 0);

    // Collapse the container's two-line region to the one-line empty form.
    fixture.replace(
        "lib/AGENTS.md",
        "<!-- stele:begin router -->\n<!-- stele:end -->",
        "<!-- stele:begin router --><!-- stele:end -->",
    );
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
