//! Structural freeze baseline (SPEC §4.2 `check --freeze`). `--freeze` writes every
//! CURRENT structural violation to `.stele/freeze.json`; every later `check` suppresses
//! exactly the baselined violations while a NEW violation still fails. Each test copies
//! the acme fixture, introduces the 8.1 forward violation (store imports billing,
//! undeclared), and exercises baselining over a real git repo (the `tests/common`
//! harness). The freeze file is a canonical, sorted, byte-stable artifact.

mod common;

use common::Fixture;

/// Line 9 of `store/subscription.ex` (an alias to a billing module) is the EXAMPLE 8.1
/// forward violation: store imports billing with no declared depends edge.
fn introduce_store_imports_billing(fixture: &Fixture) {
    fixture.insert_line_at(
        "apps/web/lib/store/subscription.ex",
        9,
        "  alias AcmeWeb.Billing.Charge",
    );
}

// A baselined violation is suppressed: `--freeze` records it, and the next `check`
// passes even though the violating import is still present.
#[test]
fn freeze_baseline_suppresses_the_frozen_violation() {
    let fixture = Fixture::acme();
    introduce_store_imports_billing(&fixture);
    fixture.commit("store imports billing (undeclared edge)");

    assert_eq!(fixture.run(&["build"]).code, 0);
    // Unfrozen, the violation fails.
    let unfrozen = fixture.run(&["check"]);
    assert_eq!(unfrozen.code, 1, "{}", unfrozen.combined());

    // Freeze it, then the same tree checks clean.
    let freeze = fixture.run(&["check", "--freeze"]);
    assert_eq!(freeze.code, 0, "{}", freeze.combined());
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// A violation that appears AFTER the baseline still fails; the baselined one stays
// suppressed — the ratchet only ever tightens.
#[test]
fn a_new_violation_still_fails_over_a_frozen_baseline() {
    let fixture = Fixture::acme();
    introduce_store_imports_billing(&fixture);
    fixture.commit("store imports billing (undeclared edge)");
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["check", "--freeze"]).code, 0);

    // A second, independent violation: worker (no declared depends) imports shared.
    fixture.insert_line_at("apps/worker/lib/dunning.ex", 11, "  alias AcmeShared.Money");
    fixture.commit("worker imports shared (undeclared edge)");
    assert_eq!(fixture.run(&["build"]).code, 0);

    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    // The new violation surfaces; the baselined one is not reported.
    assert!(out.contains("apps/worker imports packages/shared"), "{out}");
    assert!(!out.contains("apps/web/lib/store imports"), "{out}");
}

// The baseline is a byte-stable artifact, and suppression is deterministic: freezing
// twice writes the same file, and two consecutive checks agree.
#[test]
fn freeze_baseline_is_byte_stable_and_check_is_repeatable() {
    let fixture = Fixture::acme();
    introduce_store_imports_billing(&fixture);
    fixture.commit("store imports billing (undeclared edge)");
    assert_eq!(fixture.run(&["build"]).code, 0);

    assert_eq!(fixture.run(&["check", "--freeze"]).code, 0);
    let first = fixture.read(".stele/freeze.json");
    assert_eq!(fixture.run(&["check", "--freeze"]).code, 0);
    let second = fixture.read(".stele/freeze.json");
    assert_eq!(first, second, "freeze baseline diverged across runs");

    let a = fixture.run(&["check"]);
    let b = fixture.run(&["check"]);
    assert_eq!(a.code, 0, "{}", a.combined());
    assert_eq!(b.code, 0, "{}", b.combined());
    assert_eq!(
        a.combined(),
        b.combined(),
        "check output diverged across runs"
    );
}
