//! SPEC §5.3 process contract: the exit-code taxonomy. `0` clean, `1` assertion failure
//! ("repo out of spec"), `2` input error (malformed block, duplicate id, unknown lock
//! version, missing lock). Input-error cases land in Phase B (plan step 7); the clean and
//! assertion-failure cases need the assertion suite (Phase D). Ignored until then.

mod common;

use common::Fixture;

// P1 (F9): `stele build` on an UNBORN HEAD (git init → author files → build, no commit
// yet — the §7 greenfield order) must succeed (exit 0), stamp every claim `verified:null`
// (no commit to anchor a watermark to), and keep the `--json` stdout a single clean
// object with the notice on stderr. After the first commit, rebuild → check is green
// (null-watermark byte-compare consistency).
#[test]
fn exit_0_on_build_with_unborn_head() {
    let fixture = Fixture::bare();
    // A claim whose landmark anchor RESOLVES — so it would normally be stamped; on an
    // unborn HEAD it must stay verified:null (no commit to anchor to).
    fixture.write(
        "rules.py",
        "# stele:landmark greenfield-rule\ndef f():\n    return 1\n",
    );
    fixture.write(
        "AGENTS.md",
        "# proj\n\n```stele\nkind: system\npurpose: greenfield probe\ninvariants:\n\
         \x20 - claim: the rule holds\n    anchor: lm:greenfield-rule\n```\n\n\
         <!-- stele:begin router -->\n<!-- stele:end -->\n",
    );
    // Stage (so `git ls-files` sees the node) but do NOT commit — HEAD is unborn.
    fixture.stage_all();

    let build = fixture.run(&["build"]);
    assert_eq!(
        build.code,
        0,
        "unborn-HEAD build must exit 0: {}",
        build.combined()
    );
    // The notice is on stderr, so the human/JSON stdout is uncorrupted.
    assert!(
        build.stderr.contains("unborn HEAD"),
        "expected a clean stderr notice: {}",
        build.combined()
    );
    let lock = fixture.read(".stele/graph.lock");
    assert!(
        lock.contains("\"verified\": null"),
        "every claim must stamp verified:null pre-first-commit:\n{lock}"
    );

    // `--json` stdout is exactly one object even with the notice printed.
    let json = fixture.run(&["build", "--json"]);
    assert_eq!(json.code, 0, "{}", json.combined());
    let value: serde_json::Value =
        serde_json::from_str(json.stdout.trim()).expect("build --json stdout is one JSON object");
    assert_eq!(value["ok"], true, "{}", json.stdout);

    // First commit, then rebuild → check is green (null carried over, byte-compare holds).
    fixture.commit("first commit");
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// A mutating verb (§5.3 `stele init`) rejects any argument beyond the global `--json`:
// `stele init --help` is an input error (exit 2) with a usage line, and — the real trap —
// leaves the tree untouched, never silently running init and scaffolding into a live repo.
#[test]
fn exit_2_on_init_with_a_stray_arg_and_writes_nothing() {
    let fixture = Fixture::bare();
    fixture.write("apps/web/app.ex", "defmodule Acme.App do\nend\n");
    fixture.commit("bare tree, no stele nodes yet");

    let init = fixture.run(&["init", "--help"]);
    assert_eq!(init.code, 2, "{}", init.combined());
    assert!(
        init.combined().contains("usage: stele init"),
        "expected a usage line: {}",
        init.combined()
    );
    // No scaffold ran: neither the root system node nor the apps/ container was written.
    assert!(
        !fixture.path("AGENTS.md").exists(),
        "init --help scaffolded the root node"
    );
    assert!(
        !fixture.path("apps/AGENTS.md").exists(),
        "init --help scaffolded a container node"
    );
}

// `stele build` is likewise strict: a stray positional argument is an input error (exit 2)
// that aborts BEFORE any lock write, so the committed lock is never overwritten by junk.
#[test]
fn exit_2_on_build_with_a_stray_arg_and_leaves_the_lock_untouched() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let before = fixture.read(".stele/graph.lock");

    let build = fixture.run(&["build", "stray"]);
    assert_eq!(build.code, 2, "{}", build.combined());
    assert!(
        build.combined().contains("usage: stele build"),
        "expected a usage line: {}",
        build.combined()
    );
    assert_eq!(
        fixture.read(".stele/graph.lock"),
        before,
        "build with a stray arg mutated the lock"
    );
}

// Two nodes normalizing to the same id → exit 2 (§2.1). A second AGENTS.md explicitly
// declares the apps/web id already held by apps/web/AGENTS.md.
#[test]
fn exit_2_on_duplicate_node_id() {
    let fixture = Fixture::acme();
    fixture.replace(
        "packages/shared/AGENTS.md",
        "kind: container\n",
        "kind: container\nid: apps/web\n",
    );
    fixture.commit("collide packages/shared onto the apps/web id");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 2, "{}", build.combined());
}

// Two `stele` blocks in one file → exit 2 (§3.1 item 1).
#[test]
fn exit_2_on_two_stele_blocks_in_one_file() {
    let fixture = Fixture::acme();
    fixture.append(
        "apps/worker/AGENTS.md",
        "\n```stele\nkind: container\n```\n",
    );
    fixture.commit("add a second stele block to apps/worker");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 2, "{}", build.combined());
}

// Unknown lock `version` → exit 2, never best-effort parse (§3.2).
#[test]
fn exit_2_on_unknown_lock_version() {
    let fixture = Fixture::acme();
    fixture.write(
        ".stele/graph.lock",
        "{\n  \"adrs\": {},\n  \"landmarks\": {},\n  \"nodes\": {},\n  \"version\": 99\n}\n",
    );
    fixture.commit("write a lock with an unknown version");

    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 2, "{}", check.combined());
}

// A read command with no committed lock → exit 2 + "run stele build" (§5.3). `build` is
// the only writer of the lock; check/emit/query refuse without one.
#[test]
fn exit_2_without_a_lock_says_run_stele_build() {
    let fixture = Fixture::acme();
    for args in [vec!["check"], vec!["emit"], vec!["node", "apps/web"]] {
        let result = fixture.run(&args);
        let out = result.combined();
        assert_eq!(result.code, 2, "{args:?}: {out}");
        assert!(out.contains("run stele build"), "{args:?}: {out}");
    }
}

// Clean acme: build → check → exit 0 (§5.3 "0 success (check clean)").
#[test]
fn exit_0_on_clean_acme_build_then_check() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// A single assertion failure is exit 1, never exit 2 — the "repo out of spec" vs "input
// error" boundary (§5.3). Uses the 8.2 vestigial violation as a representative failure.
#[test]
fn assertion_failure_is_exit_1_not_2() {
    let fixture = Fixture::acme();
    fixture.delete_line_containing(
        "apps/web/lib/billing/charge.ex",
        "alias AcmeStore.Subscription",
    );
    fixture.commit("introduce a single vestigial-edge violation");

    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
}
