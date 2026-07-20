//! Lockfile byte-contract tests (SPEC §3.2): build determinism, the
//! serialize→parse→serialize round-trip fixpoint over adversarial strings, and
//! the canonical key order on a real acme build.

mod common;

use common::Fixture;
use std::collections::BTreeMap;
use std::path::PathBuf;
use stele::lock::{self, Lock};
use stele::model::{Allow, Claim, ClaimKind, Edges, Graph, Node, NodeKind, Verified};

const LOCK_PATH: &str = ".stele/graph.lock";

// ─── determinism ──────────────────────────────────────────────────────────────

// Two consecutive builds of identical sources produce byte-identical locks
// (§3.2 canonical serialization is a pure function of the graph).
#[test]
fn build_is_deterministic_across_runs() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let first = fixture.read(LOCK_PATH);
    assert_eq!(fixture.run(&["build"]).code, 0);
    let second = fixture.read(LOCK_PATH);
    assert_eq!(first, second, "second build diverged from the first");
}

// A freshly built lock survives a check against itself: the on-disk bytes equal
// the canonical serialization of the rebuilt graph (§5.1 byte-compare).
#[test]
fn check_matches_a_freshly_built_lock() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// ─── round-trip fixpoint over adversarial strings ─────────────────────────────

// serialize → parse → serialize reaches a fixpoint on a graph that exercises
// every escaping path: raw non-ASCII, named escapes, a C0 control, empty
// containers, null scalars, and a present-vs-absent `verified` watermark (§3.2).
#[test]
fn round_trip_is_a_fixpoint() {
    let lock = Lock::from_graph(&adversarial_graph());
    let once = lock::to_canonical_string(&lock);
    let reparsed = lock::parse_lock(&once).expect("canonical output must re-parse");
    let twice = lock::to_canonical_string(&reparsed);
    assert_eq!(once, twice, "serialization is not a fixpoint");
}

// The canonical byte invariants (§3.2): exactly one trailing LF, no trailing
// whitespace on any line, named escapes used, C0 controls as \u00xx, and
// non-ASCII emitted raw (never \u-escaped).
#[test]
fn canonical_bytes_obey_the_escaping_contract() {
    let out = lock::to_canonical_string(&Lock::from_graph(&adversarial_graph()));

    assert!(out.ends_with('\n'), "missing trailing LF");
    assert!(!out.ends_with("\n\n"), "more than one trailing LF");
    for line in out.split('\n') {
        assert_eq!(
            line,
            line.trim_end(),
            "trailing whitespace on line {line:?}"
        );
    }

    assert!(out.contains("\\n"), "newline not escaped");
    assert!(out.contains("\\t"), "tab not escaped");
    assert!(out.contains("\\\""), "quote not escaped");
    assert!(out.contains("\\\\"), "backslash not escaped");
    assert!(out.contains("\\u0001"), "C0 control not escaped as \\u00xx");
    assert!(out.contains('é') && out.contains('世'), "non-ASCII not raw");
    assert!(!out.contains("\\u00e9"), "non-ASCII was \\u-escaped");
}

// ─── canonical key order on real acme output ──────────────────────────────────

// The on-disk acme lock sorts every object's keys by Unicode scalar value
// (§3.2): top-level adrs < landmarks < nodes < version, and the first node
// object's fields budget < claims < commands < contains < declared < extracted
// < id < kind < purpose.
#[test]
fn acme_lock_has_canonical_key_order() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let lock = fixture.read(LOCK_PATH);

    assert_in_order(
        &lock,
        &["\"adrs\"", "\"landmarks\"", "\"nodes\"", "\"version\""],
    );
    assert_in_order(
        &lock,
        &[
            "\"budget\"",
            "\"claims\"",
            "\"commands\"",
            "\"contains\"",
            "\"declared\"",
            "\"extracted\"",
            "\"id\"",
            "\"kind\"",
            "\"purpose\"",
        ],
    );
}

// contains[] is tree-derived at build (§3.2): the system node contains its direct
// children, and a deeper node is contained by its nearest ancestor, not the root.
#[test]
fn acme_lock_derives_the_containment_tree() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let lock = lock::parse_lock(&fixture.read(LOCK_PATH)).expect("acme lock parses");

    assert_eq!(
        lock.nodes["/"].contains,
        vec!["apps/web", "apps/worker", "packages/shared"],
    );
    assert_eq!(
        lock.nodes["apps/web"].contains,
        vec!["apps/web/lib/billing", "apps/web/lib/store"],
    );
    assert!(lock.nodes["apps/web/lib/billing"].contains.is_empty());
}

// build stamps a non-null structural digest (§4.5) on a resolved claim in a
// parseable language: acme's billing/refund-cap binds `changeset/2` in refund.ex
// (elixir), so its `verified.digest` is a 64-hex sha256, never null.
#[test]
fn acme_lock_stamps_a_structural_digest_on_a_resolved_claim() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let lock = lock::parse_lock(&fixture.read(LOCK_PATH)).expect("acme lock parses");

    let claim = lock.nodes["apps/web/lib/billing"]
        .claims
        .iter()
        .find(|c| c.id == "refund-cap")
        .expect("billing declares refund-cap");
    let digest = claim
        .verified
        .as_ref()
        .expect("resolved claim carries a verified watermark")
        .digest
        .as_ref()
        .expect("elixir is parseable, so digest is non-null");
    assert_eq!(
        digest.len(),
        64,
        "digest is not a sha256 hex string: {digest}"
    );
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "digest is not lowercase hex: {digest}"
    );
}

// ─── fixtures & helpers ───────────────────────────────────────────────────────

/// Assert each needle appears, and in the given order, by first occurrence.
fn assert_in_order(haystack: &str, needles: &[&str]) {
    let mut cursor = 0;
    for needle in needles {
        let at = haystack[cursor..]
            .find(needle)
            .map(|offset| cursor + offset)
            .unwrap_or_else(|| panic!("missing {needle}"));
        cursor = at + needle.len();
    }
}

/// A two-node graph whose scalars exercise every serialization path.
fn adversarial_graph() -> Graph {
    let mut graph = Graph::default();
    graph.insert(root_node()).unwrap();
    graph.insert(child_node()).unwrap();
    graph
}

fn root_node() -> Node {
    let mut commands = BTreeMap::new();
    commands.insert(
        "test".to_string(),
        "echo \"line1\nline2\"\ttrailing".to_string(),
    );
    Node {
        kind: NodeKind::System,
        id: "/".to_string(),
        // Quote, backslash, newline, tab, C0 control, and non-ASCII in one string.
        purpose: Some("quote \" back \\ nl \n tab \t ctrl \u{01} unicode é 世 🚀".to_string()),
        commands,
        invariants: vec![Claim::authored(
            ClaimKind::Invariant,
            "money is integer cents — never a float".to_string(),
            "lm:money-type".to_string(),
            Some("packages/shared/test/money.test.ts".to_string()),
            "money-type".to_string(),
        )],
        hazards: Vec::new(),
        edges: Edges {
            depends: Vec::new(),
            decided_by: vec!["adr/0007".to_string()],
            allow: Vec::new(),
        },
        budget: Some(900),
        source: PathBuf::from("AGENTS.md"),
        extracted_imports: Vec::new(),
        contains: Vec::new(),
    }
}

fn child_node() -> Node {
    let mut claim = Claim::authored(
        ClaimKind::Hazard,
        "webhook handler must not write inside the verification transaction".to_string(),
        "lm:webhook-verify".to_string(),
        None,
        "webhook-verify".to_string(),
    );
    // A stamped watermark round-trips through the present-`verified` path.
    claim.verified = Some(Verified {
        sha: "e3f19ac".to_string(),
        digest: Some("a17d".to_string()),
    });
    Node {
        kind: NodeKind::Component,
        id: "apps/web".to_string(),
        purpose: None,
        commands: BTreeMap::new(),
        invariants: Vec::new(),
        hazards: vec![claim],
        edges: Edges {
            depends: vec!["packages/shared".to_string()],
            decided_by: Vec::new(),
            allow: vec![Allow {
                edge: "packages/shared".to_string(),
                reason: "dynamic dispatch the extractor cannot see".to_string(),
            }],
        },
        budget: None,
        source: PathBuf::from("apps/web/AGENTS.md"),
        extracted_imports: Vec::new(),
        contains: Vec::new(),
    }
}
