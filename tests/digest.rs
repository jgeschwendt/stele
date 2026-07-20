//! AST-region structural digest tests (SPEC §4.5, EXAMPLE 8.4). The (a)/(b) cases
//! run through the real `stele build` on scratch acme copies, diffing the on-disk
//! lock's `refund-cap` digest across rebuilds; the (c)/(d) cases and the binding
//! rules (immediate-precedence landmark, no-definition fallback, symbol anchor,
//! scope isolation) are unit tests over `anchors::digest_for_claim` on inline
//! sources.

mod common;

use common::Fixture;
use std::fs;
use stele::anchors::digest_for_claim;
use stele::lock::{self, LockClaim};

const LOCK_PATH: &str = ".stele/graph.lock";
/// The acme node whose `refund-cap` claim binds `changeset/2` (EXAMPLE 8.4).
const BILLING: &str = "apps/web/lib/billing";
/// The `changeset/2` body as authored in the acme fixture (the digested region).
const CHANGESET_BODY: &str =
    "    refund\n    |> cast(attrs, @castable_fields)\n    |> validate_refund_cap()";

// ─── (a)/(b): through the real binary, diffing lock digests ───────────────────

/// (a) A formatting-only edit inside the bound `changeset/2` — collapsing the pipe
/// onto one line, same tokens — leaves the digest byte-identical (§4.5: the AST
/// ignores whitespace).
#[test]
fn formatting_only_edit_keeps_the_digest() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let before = refund_cap_digest(&fixture);
    assert!(
        is_sha256_hex(&before),
        "baseline digest not 64-hex: {before}"
    );

    // Same tokens, different whitespace: a pure reformat of the bound region.
    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        CHANGESET_BODY,
        "    refund |> cast(attrs, @castable_fields) |> validate_refund_cap()",
    );
    assert_eq!(fixture.run(&["build"]).code, 0);

    assert_eq!(
        before,
        refund_cap_digest(&fixture),
        "formatting-only edit changed the digest"
    );
}

/// (b) A token-level semantic edit inside the bound `changeset/2` — swapping the
/// cast's field list — changes the digest (§4.5: the guarded region's AST changed).
#[test]
fn semantic_edit_inside_bound_region_changes_the_digest() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let before = refund_cap_digest(&fixture);

    fixture.replace(
        "apps/web/lib/billing/refund.ex",
        "|> cast(attrs, @castable_fields)",
        "|> cast(attrs, [:amount_cents])",
    );
    assert_eq!(fixture.run(&["build"]).code, 0);

    assert_ne!(
        before,
        refund_cap_digest(&fixture),
        "semantic edit did not change the digest"
    );
    assert!(is_sha256_hex(&refund_cap_digest(&fixture)));
}

// ─── (c)/(d) + binding rules: unit over digest_for_claim ──────────────────────

/// An elixir module whose `# stele:landmark` comment for `cap` (line 4) immediately
/// precedes `def changeset/1` past an intervening `@doc` and blank scope. `other/1` and the
/// `@castable_fields` attribute sit OUTSIDE the bound region.
const ELIXIR: &str = "\
defmodule M do
  @castable_fields [:a, :b]

  # stele:landmark cap
  @doc \"the cap changeset\"
  def changeset(attrs) do
    attrs |> cast(@castable_fields)
  end

  def other(y) do
    y + 1
  end
end
";

/// (c) A comment added inside the bound region does not change the digest (§4.5:
/// comment subtrees are dropped from the serialization).
#[test]
fn comment_churn_inside_bound_region_is_stable() {
    let base = elixir_cap_digest(ELIXIR).expect("elixir has a parser");
    let churned = ELIXIR.replace(
        "    attrs |> cast(@castable_fields)",
        "    # a fresh inline note\n    attrs |> cast(@castable_fields)",
    );
    assert_eq!(base, elixir_cap_digest(&churned).unwrap());
}

/// A parser-less language (markdown) yields no digest — the §2.4 churn-count
/// fallback (Phase D5), a different mechanism from the (d) AST binding fallback
/// below.
#[test]
fn parserless_language_has_no_digest() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("NOTES.md"), "# stele:landmark doc\ntext\n").unwrap();
    assert_eq!(
        digest_for_claim(dir.path(), "lm:doc", "NOTES.md:1").unwrap(),
        None,
    );
}

/// The landmark binds the NEXT definition (`changeset/1`), not the enclosing module
/// or the later `other/1`: edits OUTSIDE the bound region leave the digest stable
/// (the `@castable_fields` value, `other/1`'s body), an edit INSIDE it fires.
#[test]
fn landmark_binds_only_the_immediately_following_definition() {
    let base = elixir_cap_digest(ELIXIR).unwrap();

    // Outside the bound `changeset/1`: the attribute VALUE it references.
    let attr = ELIXIR.replace("@castable_fields [:a, :b]", "@castable_fields [:a]");
    assert_eq!(base, elixir_cap_digest(&attr).unwrap(), "attr edit leaked");

    // Outside: a sibling definition's body.
    let sibling = ELIXIR.replace("y + 1", "y + 2");
    assert_eq!(base, elixir_cap_digest(&sibling).unwrap(), "sibling leaked");

    // Inside the bound region: a token change fires.
    let inside = ELIXIR.replace("cast(@castable_fields)", "cast([:a])");
    assert_ne!(
        base,
        elixir_cap_digest(&inside).unwrap(),
        "inside did not fire"
    );
}

/// (d) A landmark preceding NO definition in its scope falls back to its strictly-
/// enclosing named node, then the whole file (§4.5). Two tiers:
///
/// Tier 1 — a landmark inside `wrapper`'s body precedes no definition among its
/// block siblings, so it binds the enclosing `wrapper`: an edit INSIDE `wrapper`
/// fires; an edit to the sibling `other` is invisible (the region is `wrapper`, not
/// the file).
///
/// Tier 2 — a top-level landmark with no definition after it and no enclosing named
/// node binds the whole file: an edit to a top-level item that in Tier 1 would have
/// been an out-of-region sibling now fires, because the region is the entire file.
#[test]
fn landmark_with_no_following_definition_falls_back_to_enclosing_then_file() {
    // Tier 1: landmark on line 2 sits inside `wrapper`'s block, before no definition.
    const ENCLOSED: &str = "\
fn wrapper(a: u32) -> u32 {
    // stele:landmark note
    a + 1
}

fn other(b: u32) -> u32 {
    b * 2
}
";
    let enclosed_base = rust_landmark_digest(ENCLOSED, 2).unwrap();
    assert!(is_sha256_hex(&enclosed_base));

    // Inside the enclosing `wrapper`: fires.
    let inside = ENCLOSED.replace("a + 1", "a + 2");
    assert_ne!(
        enclosed_base,
        rust_landmark_digest(&inside, 2).unwrap(),
        "edit inside the enclosing fn did not fire"
    );
    // Outside it (sibling `other`): stable — the region is `wrapper`, not the file.
    let sibling = ENCLOSED.replace("b * 2", "b * 3");
    assert_eq!(
        enclosed_base,
        rust_landmark_digest(&sibling, 2).unwrap(),
        "sibling fn leaked — fallback bound the file, not the enclosing fn"
    );

    // Tier 2: top-level landmark on line 4, no definition after it, no enclosing
    // named node → the whole file.
    const TOP_LEVEL: &str = "\
const A: u32 = 1;
const B: u32 = 2;

// stele:landmark tail
";
    let file_base = rust_landmark_digest(TOP_LEVEL, 4).unwrap();
    assert!(is_sha256_hex(&file_base));

    // A top-level item edit fires: the bound region is the entire file.
    let file_edit = TOP_LEVEL.replace("const A: u32 = 1", "const A: u32 = 9");
    assert_ne!(
        file_base,
        rust_landmark_digest(&file_edit, 4).unwrap(),
        "file-level edit did not fire — fallback did not reach the whole file"
    );
}

/// A `<path>#<symbol>` anchor digests exactly the resolved symbol's definition node:
/// editing another function is invisible; editing the bound one fires.
#[test]
fn symbol_anchor_digests_only_its_definition() {
    const RUST: &str = "\
fn alpha(a: u32) -> u32 {
    a + 1
}

fn beta(b: u32) -> u32 {
    b * 2
}
";
    let base = rust_symbol_digest(RUST, "alpha").unwrap();
    assert!(is_sha256_hex(&base));

    let other = RUST.replace("b * 2", "b * 3");
    assert_eq!(
        base,
        rust_symbol_digest(&other, "alpha").unwrap(),
        "beta leaked into alpha"
    );

    let own = RUST.replace("a + 1", "a + 2");
    assert_ne!(
        base,
        rust_symbol_digest(&own, "alpha").unwrap(),
        "alpha edit did not fire"
    );
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// The `refund-cap` claim's digest from the freshly built acme lock; panics if the
/// claim is missing or its digest is null (both are regressions this suite guards).
fn refund_cap_digest(fixture: &Fixture) -> String {
    let lock = lock::parse_lock(&fixture.read(LOCK_PATH)).expect("acme lock parses");
    let claim = billing_refund_cap(&lock.nodes[BILLING].claims);
    claim
        .verified
        .as_ref()
        .expect("refund-cap is resolved, so verified is stamped")
        .digest
        .clone()
        .expect("elixir is parseable, so digest is non-null")
}

fn billing_refund_cap(claims: &[LockClaim]) -> &LockClaim {
    claims
        .iter()
        .find(|c| c.id == "refund-cap")
        .expect("billing declares refund-cap")
}

/// The digest of `lm:cap` (landmark on line 4) over an inline elixir `m.ex`.
fn elixir_cap_digest(source: &str) -> Option<String> {
    digest_inline("m.ex", source, "lm:cap", "m.ex:4")
}

/// The digest of `m.rs#<symbol>` over an inline rust `m.rs`.
fn rust_symbol_digest(source: &str, symbol: &str) -> Option<String> {
    digest_inline("m.rs", source, &format!("m.rs#{symbol}"), "m.rs:1")
}

/// The digest of an `lm:note` landmark on 1-based `line` over an inline rust `m.rs`.
fn rust_landmark_digest(source: &str, line: usize) -> Option<String> {
    digest_inline("m.rs", source, "lm:note", &format!("m.rs:{line}"))
}

/// Write `source` to `name` in a temp dir and digest `anchor` at `resolved`.
fn digest_inline(name: &str, source: &str, anchor: &str, resolved: &str) -> Option<String> {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(name), source).unwrap();
    digest_for_claim(dir.path(), anchor, resolved).unwrap()
}

/// Whether `s` is 64 lowercase hex characters (a sha256 digest, §4.5).
fn is_sha256_hex(s: &str) -> bool {
    const SHA256_HEX_LEN: usize = 64;
    s.len() == SHA256_HEX_LEN
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}
