//! The glyph notation end to end (SPEC §2.5 tokens and the silent-prose rule, §2.6
//! `§ <NNNN>`, §3.1 item 2 region markers, §3.2 lock v2). One grammar, no dual-read:
//! a comment glyph that does not match is prose, an authored FIELD that does not match
//! is exit 2, and a referenced landmark that no comment declares fails referentially.

mod common;

use common::Fixture;

/// The billing node's landmark anchor as the fixture authors it (§2.4).
const BILLING_ANCHOR: &str = "anchor: ※ refund-cap";

/// Four comment lines that LOOK like tokens and are all prose (§2.5): a CJK annotation
/// mark with a non-slug payload, a `note:` payload that fails the slug lexeme, a claim
/// glyph opening an English sentence (no `/`, so no address), and a glyph glued to its
/// following character (no separating space, so not a token at all).
const PROSE_GLYPHS: &str = "\
# ※ 注意: hot path
# ※ note: retry twice
# ⊨ is the models glyph
# ※注意
defmodule AcmeWeb.Billing.Notes do
end
";

// ─── §2.5 silent non-match ───────────────────────────────────────────────────

#[test]
fn prose_glyphs_build_clean_and_declare_nothing() {
    let fixture = Fixture::acme();
    fixture.write("apps/web/lib/billing/notes.ex", PROSE_GLYPHS);
    fixture.commit("a billing file whose glyphs are prose");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());

    // Nothing from that file reached the anchor index (§3.2 `landmarks{}`).
    let lock = fixture.read(".stele/graph.lock");
    assert!(!lock.contains("注意"), "{lock}");
    assert!(!lock.contains("notes.ex"), "{lock}");
}

// ─── §4.1 a typo'd landmark fails referentially, never lexically ─────────────

#[test]
fn typo_in_a_referenced_landmark_fails_check_not_build() {
    let fixture = Fixture::acme();
    // The comment keeps its slug; only the node file's reference is mistyped, so the
    // slug has cardinality 0 — an exit-1 referential failure, not an exit-2 parse error.
    fixture.replace(
        "apps/web/lib/billing/AGENTS.md",
        BILLING_ANCHOR,
        "anchor: ※ refund-capp",
    );
    fixture.commit("typo the refund-cap anchor reference");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("※ refund-capp"), "{out}");
    assert!(out.contains("unresolved"), "{out}");
}

#[test]
fn malformed_anchor_field_is_exit_2_at_build() {
    let fixture = Fixture::acme();
    // The FIELD is authored schema, not scanned prose (§2.4/§2.5): a malformed slug
    // here is an input error, however silent the same bytes would be in a comment.
    fixture.replace(
        "apps/web/lib/billing/AGENTS.md",
        BILLING_ANCHOR,
        "anchor: ※ Refund-Cap",
    );
    fixture.commit("an anchor field that is not a slug");

    let build = fixture.run(&["build"]);
    let out = build.combined();
    assert_eq!(build.code, 2, "{out}");
    assert!(out.contains("Refund-Cap"), "{out}");
}

// ─── §2.6 decision references ────────────────────────────────────────────────

#[test]
fn decided_by_resolves_the_zero_padded_adr_number() {
    // The clean fixture already declares `decided_by: [§ 0007]` against
    // adr/0007-integer-cents.md; a clean check is the resolution proof.
    let fixture = Fixture::acme();
    assert!(
        fixture
            .read("apps/web/lib/billing/AGENTS.md")
            .contains("decided_by: [§ 0007]")
    );
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

#[test]
fn decided_by_number_that_does_not_match_the_file_fails_referentially() {
    let fixture = Fixture::acme();
    // `§ 7` is well-formed, so build accepts it; the ADR index is keyed by the number
    // as the FILENAME zero-pads it (§2.6), so it resolves to nothing — exit 1.
    fixture.replace(
        "apps/web/lib/billing/AGENTS.md",
        "decided_by: [§ 0007]",
        "decided_by: [§ 7]",
    );
    fixture.commit("drop the zero padding from the decision reference");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("§ 7"), "{out}");
    assert!(out.contains("no ADR"), "{out}");
}

#[test]
fn decided_by_in_the_pre_0_3_0_path_form_is_exit_2_naming_the_glyph_form() {
    let fixture = Fixture::acme();
    fixture.replace(
        "apps/web/lib/billing/AGENTS.md",
        "decided_by: [§ 0007]",
        "decided_by: [adr/0007]",
    );
    fixture.commit("the pre-0.3.0 decided_by path form");

    let build = fixture.run(&["build"]);
    let out = build.combined();
    assert_eq!(build.code, 2, "{out}");
    assert!(out.contains("§ <NNNN>"), "{out}");
}

// ─── §3.1 item 2 region markers ──────────────────────────────────────────────

#[test]
fn bare_and_annotated_begin_markers_name_the_same_router_region() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    // The fixture ships no `.stele/index/`, so materialize once before byte-diffing.
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());

    // The root carries an annotated marker, the containers a bare one; both are the
    // `router` region, so `emit --check` stays clean across the two spellings.
    assert!(
        fixture
            .read("AGENTS.md")
            .contains("<!-- @stele router · generated")
    );
    assert!(
        fixture
            .read("apps/web/AGENTS.md")
            .contains("<!-- @stele -->")
    );

    // Naming the region explicitly changes nothing.
    fixture.replace(
        "apps/web/AGENTS.md",
        "<!-- @stele -->",
        "<!-- @stele router -->",
    );
    fixture.commit("name the web router region explicitly");
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

#[test]
fn one_line_bare_region_round_trips() {
    let fixture = Fixture::acme();
    fixture.replace(
        "apps/web/lib/store/AGENTS.md",
        "<!-- @stele -->\n<!-- @end -->",
        "<!-- @stele --><!-- @end -->",
    );
    fixture.commit("collapse the store region to the one-line form");

    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let before = fixture.read("apps/web/lib/store/AGENTS.md");
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(fixture.read("apps/web/lib/store/AGENTS.md"), before);
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

#[test]
fn pre_0_3_0_region_markers_are_exit_2_naming_the_new_form() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.replace(
        "apps/web/AGENTS.md",
        "<!-- @stele -->\n<!-- @end -->",
        "<!-- stele:begin router -->\n<!-- stele:end -->",
    );
    fixture.commit("restore the pre-0.3.0 region markers");

    // The grammar moved: the old markers are not a region at all, so `emit` reports a
    // node with no generated region — and names the form that would be one.
    let emit = fixture.run(&["emit"]);
    let out = emit.combined();
    assert_eq!(emit.code, 2, "{out}");
    assert!(out.contains("@stele"), "{out}");
}

// ─── §3.2 the version-1 lock ─────────────────────────────────────────────────

#[test]
fn version_1_lock_is_exit_2_pointing_at_migrate() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let lock = fixture.read(".stele/graph.lock");
    fixture.write(
        ".stele/graph.lock",
        &lock.replacen("\"version\": 2", "\"version\": 1", 1),
    );
    fixture.commit("downgrade the lock to the pre-0.3.0 version");

    for verb in [&["check"][..], &["emit"][..], &["emit", "--check"][..]] {
        let run = fixture.run(verb);
        let out = run.combined();
        assert_eq!(run.code, 2, "{verb:?}: {out}");
        assert!(out.contains("version 1"), "{verb:?}: {out}");
        assert!(
            out.trim_end()
                .ends_with("run stele migrate, then stele build"),
            "{verb:?}: {out}"
        );
    }
}
