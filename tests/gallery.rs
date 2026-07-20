//! The EXAMPLE §8 CI failure gallery, encoded as the behavioral oracle: one test per
//! assertion class. Each copies the acme fixture, applies the §8 mutation, and (where
//! the mutated tree is still buildable) runs `stele build` then `stele check`, asserting
//! the exit code (SPEC §5.3) and the stable, SEMANTIC message fragments the later phases
//! must produce. Ignored until their phase lands (plan Phase D); the ignore reason names
//! the phase that un-ignores them.

mod common;

use common::Fixture;

/// apps/api gets exactly this many uncovered source files in 8.6a (EXAMPLE: "14 files").
const API_FILE_COUNT: usize = 14;

/// A `legacy/money.ts` copy whose line 7 duplicates the `money-type` landmark (8.3b).
const LEGACY_MONEY_TS: &str = "\
// Legacy money helpers retained during the cents migration.
// Predates packages/shared/src/money.ts and should have been deleted.
export type LegacyMoney = {
  readonly cents: number;
  readonly currency: string;
};
// stele:landmark money-type
export function legacyMoney(cents: number): LegacyMoney {
  return { cents, currency: \"usd\" };
}
";

// 8.1 Structural / violation (forward): apps/web/lib/store imports billing, undeclared.
#[test]
fn gallery_8_1_structural_forward_violation() {
    let fixture = Fixture::acme();
    fixture.insert_line_at(
        "apps/web/lib/store/subscription.ex",
        9,
        "  alias AcmeWeb.Billing.Charge",
    );
    fixture.commit("store imports billing (undeclared edge)");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "mutated tree must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("structural"), "{out}");
    assert!(
        out.contains("apps/web/lib/store imports apps/web/lib/billing"),
        "{out}"
    );
    assert!(out.contains("edge not declared"), "{out}");
}

// 8.2 Structural / vestigial (reverse): billing declares depends on store, but no longer
// imports it.
#[test]
fn gallery_8_2_structural_reverse_vestigial() {
    let fixture = Fixture::acme();
    fixture.delete_line_containing(
        "apps/web/lib/billing/charge.ex",
        "alias AcmeStore.Subscription",
    );
    fixture.commit("billing stops importing store");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "mutated tree must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("structural"), "{out}");
    assert!(out.contains("no import found"), "{out}");
}

// 8.3 Referential: (a) a renamed landmark comment leaves lm:refund-cap unresolved;
// (b) a copy-paste refactor duplicates the money-type landmark (cardinality 2).
#[test]
fn gallery_8_3_referential() {
    let fixture = Fixture::acme();
    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "stele:landmark refund-cap",
        "stele:landmark refund-cap-v2",
    );
    fixture.write("packages/shared/src/legacy/money.ts", LEGACY_MONEY_TS);
    fixture.commit("rename refund-cap landmark; duplicate money-type in a legacy copy");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "mutated tree must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("unresolved"), "{out}");
    assert!(out.contains("cardinality 2"), "{out}");
}

// 8.4 Freshness: a semantic edit INSIDE changeset/2 (the digested region) with the claim
// untouched. The baseline build stamps verified {sha, digest}; the edit then lands
// WITHOUT a rebuild, so `check` recomputes the region digest and sees it drift (§4.5).
#[test]
fn gallery_8_4_freshness_digest_drift() {
    let fixture = Fixture::acme();
    // Baseline: stamp the clean digest of changeset/2 for the refund-cap claim.
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("baseline graph.lock");

    // The region changes; the claim comment does not. No rebuild re-affirms it.
    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "|> cast(attrs, @castable_fields)",
        "|> cast(attrs, [:amount_cents, :charge_id, :reason])",
    );
    fixture.commit("loosen cap for partial captures");

    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("digest"), "{out}");
}

// 8.5 Budget: one mutation — a style guide pasted after the root's generated region —
// trips BOTH budget classes the EXAMPLE §8.5 oracle emits (SPEC §4.4):
//   budget[codex]: the root→leaf concatenation chain overflows the 32 KiB byte cap
//                  (`project_doc_max_bytes`), which Codex silently truncates;
//   budget[node]:  the root node renders past its declared budget (900) in tokens.
// The pasted block is sized PAST the 32 KiB codex cap: the clean fixture root chain is
// only ~3.2 KiB, so a token-sized block would trip the node budget alone and never the
// codex byte cap — the codex class needs a chain that actually overflows on bytes.
#[test]
fn gallery_8_5_budget() {
    // `codex` byte cap (`project_doc_max_bytes`, SPEC §4.4). The block clears it so the
    // root→leaf chain overflows on bytes, not only the node budget on tokens.
    const CODEX_CAP_BYTES: usize = 32 * 1024;

    let fixture = Fixture::acme();
    let paragraph = "Prefer explicit names over clever abbreviations; keep functions small \
        and single-purpose; alphabetize imports; never leave a commented-out block in the \
        tree; write the test before the fix whenever the bug is reproducible at all. ";
    let mut prose = String::from("\n## Style notes\n\n");
    while prose.len() <= CODEX_CAP_BYTES {
        prose.push_str(paragraph);
    }
    fixture.append("AGENTS.md", &prose);
    fixture.commit("paste a style guide into the root free-prose area");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "mutated tree must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("budget"), "{out}");
    // budget[codex]: the root→leaf chain overflows the byte cap named by its config key.
    assert!(out.contains("codex"), "{out}");
    assert!(out.contains("project_doc_max_bytes"), "{out}");
    // budget[node]: the root node's declared budget, now overflowed by the prose block.
    assert!(out.contains("900"), "{out}");
}

// 8.6a Exhaustiveness: a new apps/api/ directory with 14 files and no AGENTS.md is
// covered by no node.
#[test]
fn gallery_8_6a_exhaustiveness_uncovered_dir() {
    let fixture = Fixture::acme();
    for i in 1..=API_FILE_COUNT {
        fixture.write(
            &format!("apps/api/lib/api/endpoint_{i}.ex"),
            &format!("defmodule AcmeApi.Endpoint{i} do\n  def call(conn), do: conn\nend\n"),
        );
    }
    fixture.commit("add apps/api with 14 uncovered source files");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "mutated tree must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("exhaustiveness"), "{out}");
    assert!(out.contains("apps/api"), "{out}");
}

// 8.6b Liveness: the db-reset command points at `mix ecto.reset`, whose alias is deleted
// from mix.exs — the task no longer resolves.
#[test]
fn gallery_8_6b_liveness_missing_task() {
    let fixture = Fixture::acme();
    fixture.delete_line_containing("mix.exs", "\"ecto.reset\":");
    fixture.commit("remove the ecto.reset mix alias");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "mutated tree must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("liveness"), "{out}");
    assert!(out.contains("ecto.reset"), "{out}");
}
