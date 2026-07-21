//! The read/query CLI surface (SPEC §5.1, §5.3) over a committed lock: `root`, `node`,
//! `unfold`, `invariants`, `hazards`, `nodes`, and `check --report`, plus the shared
//! lock-presence gate and the `--json` envelope. Each probe copies the acme fixture
//! (`tests/common`), builds the lock, then exercises one verb — the successCriteria
//! probes 1–7 and 10 as integration tests. Probes 8–9 (the full adoption walkthrough)
//! live in `tests/walkthrough.rs`.

mod common;

use common::Fixture;

/// Build the committed lock, asserting success — the precondition every read verb needs
/// (§5.3: a missing lock is exit 2 "run stele build").
fn built() -> Fixture {
    let fixture = Fixture::acme();
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    fixture
}

// Probe 1 — `stele root` renders the §6 initialContext: identity, command table,
// hazard banner, router, index pointers, and the two engine lines, in order.
#[test]
fn root_renders_the_six_item_initial_context() {
    let fixture = built();
    let root = fixture.run(&["root"]);
    assert_eq!(root.code, 0, "{}", root.combined());
    let out = root.stdout;

    // Item 1: identity line (the system node's purpose).
    assert!(
        out.starts_with("Subscription billing platform"),
        "identity line first:\n{out}"
    );
    // Item 2: the command table (from the system node's commands).
    assert!(out.contains("## Commands"), "{out}");
    assert!(out.contains("mise install && mix deps.get"), "{out}");
    // Items 3–6 reuse the emit renderers.
    assert!(out.contains("## Hazards (2 active)"), "{out}");
    assert!(out.contains("→ lm:dunning-batch"), "{out}");
    assert!(out.contains("## Map"), "{out}");
    assert!(out.contains("`stele unfold apps/web`"), "{out}");
    assert!(
        out.contains("All invariants: `.stele/index/invariants.md`"),
        "{out}"
    );
    assert!(out.contains("MCP: `stele serve`."), "{out}");
    assert!(
        out.contains("No engine → everything above is complete"),
        "{out}"
    );
}

// Probe 2 — `stele node <abbrev>` resolves an unambiguous final-segment abbreviation
// and prints every field of the resolved node.
#[test]
fn node_resolves_abbreviation_and_prints_all_fields() {
    let fixture = built();
    let node = fixture.run(&["node", "billing"]);
    assert_eq!(node.code, 0, "{}", node.combined());
    let out = node.stdout;

    assert!(out.starts_with("apps/web/lib/billing (component)"), "{out}");
    assert!(out.contains("purpose: Charges, refunds"), "{out}");
    assert!(
        out.contains("test: MIX_ENV=test mix test apps/web/test/billing"),
        "{out}"
    );
    assert!(
        out.contains("depends: apps/web/lib/store, packages/shared"),
        "{out}"
    );
    assert!(out.contains("decided_by: adr/0007"), "{out}");
    // Claims carry their slug, prose, anchor, and resolved location.
    assert!(
        out.contains("[refund-cap]")
            && out.contains("lm:refund-cap → apps/web/lib/billing/refund.ex:18"),
        "{out}"
    );
    assert!(out.contains("[webhook-verify]"), "{out}");
    assert!(out.contains("budget: 600"), "{out}");
}

// Probe 3 — an unknown node id is an input error (exit 2) that lists candidates.
#[test]
fn node_unknown_id_exits_two_with_candidates() {
    let fixture = built();
    let node = fixture.run(&["node", "does-not-exist"]);
    assert_eq!(node.code, 2, "{}", node.combined());
    let out = node.combined();
    assert!(out.contains("no node \"does-not-exist\""), "{out}");
    // The candidate list names real node ids.
    assert!(out.contains("apps/web/lib/billing"), "{out}");
}

// Probe 4 — `stele unfold <id> --depth 2` prints the node plus neighbours out to two
// hops (depth 1: children + depends; depth 2: their neighbours).
#[test]
fn unfold_expands_two_hops() {
    let fixture = built();
    let unfold = fixture.run(&["unfold", "apps/web", "--depth", "2"]);
    assert_eq!(unfold.code, 0, "{}", unfold.combined());
    let out = unfold.stdout;

    assert!(out.starts_with("apps/web (container)"), "{out}");
    assert!(out.contains("neighbours (depth 2):"), "{out}");
    // Hop 1: the two child components.
    assert!(out.contains("apps/web/lib/billing · component"), "{out}");
    assert!(out.contains("apps/web/lib/store · component"), "{out}");
    // Hop 2: reached only through a hop-1 node's depends (billing/store → shared).
    assert!(out.contains("packages/shared · container"), "{out}");

    // Depth 1 (default) does NOT reach the second hop.
    let shallow = fixture.run(&["unfold", "apps/web"]);
    assert_eq!(shallow.code, 0, "{}", shallow.combined());
    assert!(
        !shallow.stdout.contains("packages/shared"),
        "depth 1 must not reach hop 2:\n{}",
        shallow.stdout
    );
}

// Probe 5 — `stele invariants --touching <path>` surfaces the owning node's invariants
// PLUS its ancestors' (upward exposure): billing pulls the SYSTEM money invariant.
#[test]
fn invariants_touching_surfaces_ancestor_exposure() {
    let fixture = built();
    let inv = fixture.run(&["invariants", "--touching", "apps/web/lib/billing"]);
    assert_eq!(inv.code, 0, "{}", inv.combined());
    let out = inv.stdout;

    // billing's own two invariants…
    assert!(
        out.contains("apps/web/lib/billing · billing-idempotency"),
        "{out}"
    );
    assert!(out.contains("apps/web/lib/billing · refund-cap"), "{out}");
    // …and the system-node money invariant, surfaced upward (EXAMPLE §7 step 3).
    assert!(out.contains("/ · money-type"), "{out}");

    // A sibling that does NOT descend from billing sees only the system invariant.
    let store = fixture.run(&["invariants", "--touching", "apps/web/lib/store"]);
    assert_eq!(store.code, 0, "{}", store.combined());
    assert!(store.stdout.contains("/ · money-type"), "{}", store.stdout);
    assert!(
        !store.stdout.contains("refund-cap"),
        "store must not see billing's invariant:\n{}",
        store.stdout
    );
}

// Probe 6 — `stele hazards --node <id>` reports just that node's hazards (no upward
// exposure, the §4.2 contrast to invariants).
#[test]
fn hazards_filtered_by_node() {
    let fixture = built();
    let hz = fixture.run(&["hazards", "--node", "apps/worker"]);
    assert_eq!(hz.code, 0, "{}", hz.combined());
    assert!(
        hz.stdout.contains("apps/worker · dunning-batch"),
        "{}",
        hz.stdout
    );
    // The billing webhook hazard belongs to another node and must not appear.
    assert!(
        !hz.stdout.contains("webhook-verify"),
        "node filter leaked another node's hazard:\n{}",
        hz.stdout
    );

    // Unfiltered, every active hazard shows.
    let all = fixture.run(&["hazards"]);
    assert_eq!(all.code, 0, "{}", all.combined());
    assert!(all.stdout.contains("dunning-batch"), "{}", all.stdout);
    assert!(all.stdout.contains("webhook-verify"), "{}", all.stdout);
}

// Probe 7 — `stele nodes --kind <kind>` lists nodes filtered by kind.
#[test]
fn nodes_filtered_by_kind() {
    let fixture = built();
    let containers = fixture.run(&["nodes", "--kind", "container"]);
    assert_eq!(containers.code, 0, "{}", containers.combined());
    let out = containers.stdout;
    assert!(out.contains("apps/web · container"), "{out}");
    assert!(out.contains("apps/worker · container"), "{out}");
    assert!(out.contains("packages/shared · container"), "{out}");
    // A component must be filtered out.
    assert!(
        !out.contains("apps/web/lib/billing"),
        "kind filter leaked a component:\n{out}"
    );
}

// Probe 10 — the shared lock-presence gate (§5.3) and the `check --report` governance
// section (§4.2), including the `--json` envelope shape.
#[test]
fn lock_gate_and_report_and_json_envelope() {
    // (a) A read verb with no committed lock is exit 2 "run stele build".
    let fresh = Fixture::acme();
    let ungated = fresh.run(&["root"]);
    assert_eq!(ungated.code, 2, "{}", ungated.combined());
    assert!(
        ungated.combined().contains("run stele build"),
        "{}",
        ungated.combined()
    );

    // …and the gate rides the JSON envelope too (ok:false, exit:2).
    let ungated_json = fresh.run(&["root", "--json"]);
    assert_eq!(ungated_json.code, 2, "{}", ungated_json.combined());
    assert!(
        ungated_json.stdout.contains("\"ok\":false") && ungated_json.stdout.contains("\"exit\":2"),
        "{}",
        ungated_json.stdout
    );

    // (b) After build, `check --report` exits 0 and prints the allow-entries section
    // (empty on acme — the header still shows the governance surface).
    let fixture = built();
    let report = fixture.run(&["check", "--report"]);
    assert_eq!(report.code, 0, "{}", report.combined());
    assert!(
        report.combined().contains("allow entries (0):"),
        "{}",
        report.combined()
    );

    // (c) A successful verb's `--json` envelope carries the verb name and ok:true.
    let root_json = fixture.run(&["root", "--json"]);
    assert_eq!(root_json.code, 0, "{}", root_json.combined());
    assert!(
        root_json.stdout.contains("\"command\":\"root\"")
            && root_json.stdout.contains("\"ok\":true"),
        "{}",
        root_json.stdout
    );
}

// Probe 11 — `stele --version` prints `stele <CARGO_PKG_VERSION>` to stdout and exits 0
// WITHOUT a lock or git repo (scripts/install.sh calls it to confirm the landed binary
// runs). The bare `Fixture` is a git repo with no lock; the flag must short-circuit
// before any lock/repo gate.
#[test]
fn version_flag_prints_version_without_a_lock() {
    let expected = format!("stele {}\n", env!("CARGO_PKG_VERSION"));
    let fixture = Fixture::bare();
    for flag in ["--version", "-V"] {
        let out = fixture.run(&[flag]);
        assert_eq!(out.code, 0, "{}", out.combined());
        assert_eq!(out.stdout, expected, "{flag}: {}", out.combined());
    }
}
