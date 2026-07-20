//! SPEC §5.3 process contract: the exit-code taxonomy. `0` clean, `1` assertion failure
//! ("repo out of spec"), `2` input error (malformed block, duplicate id, unknown lock
//! version, missing lock). Input-error cases land in Phase B (plan step 7); the clean and
//! assertion-failure cases need the assertion suite (Phase D). Ignored until then.

mod common;

use common::Fixture;

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
