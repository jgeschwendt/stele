//! `stele emit` oracle tests (SPEC §3.1 items 2–3, §6, §6.1, §3.3, §5.1). The
//! committed acme fixture is the byte-identity oracle: `build` → `emit` must leave
//! every hand-authored AGENTS.md byte-identical (the root and billing regions are the
//! reverse-engineered render targets), and `emit --check` must then exit 0. The rest
//! pins the failure modes — malformed markers and a missing region (exit 2), region
//! divergence (exit 1), index determinism, and the `--claude-rules` opt-in.

mod common;

use common::Fixture;

/// Every hand-authored AGENTS.md in acme (walkthrough §7 shares this list).
const ACME_AGENTS: [&str; 6] = [
    "AGENTS.md",
    "apps/web/AGENTS.md",
    "apps/web/lib/billing/AGENTS.md",
    "apps/web/lib/store/AGENTS.md",
    "apps/worker/AGENTS.md",
    "packages/shared/AGENTS.md",
];

/// A committed acme fixture with a built lock — the precondition for every `emit` run.
fn built() -> Fixture {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture
}

// ─── the byte-identity oracle ────────────────────────────────────────────────

#[test]
fn emit_reproduces_every_authored_region_byte_for_byte() {
    let fixture = built();
    let before: Vec<String> = ACME_AGENTS.iter().map(|p| fixture.read(p)).collect();

    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 0, "{}", emit.combined());
    for (path, was) in ACME_AGENTS.iter().zip(&before) {
        assert_eq!(&fixture.read(path), was, "emit changed {path}");
    }

    // The renderer is the oracle for its own output: a fresh check must be clean.
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

#[test]
fn emit_is_idempotent_second_run_is_a_no_op() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let once: Vec<String> = ACME_AGENTS.iter().map(|p| fixture.read(p)).collect();
    assert_eq!(fixture.run(&["emit"]).code, 0);
    for (path, was) in ACME_AGENTS.iter().zip(&once) {
        assert_eq!(&fixture.read(path), was, "second emit changed {path}");
    }
}

// ─── divergence (exit 1) — assertion-failure semantics, rewrite-inside-only ───

#[test]
fn emit_check_flags_a_diverged_region_by_file() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);

    // A hand-edit strictly inside the root region.
    fixture.replace(
        "AGENTS.md",
        "## Hazards (2 active)",
        "## Hazards (7 active)",
    );
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 1, "{}", check.combined());
    assert!(
        check.combined().contains("AGENTS.md"),
        "{}",
        check.combined()
    );
}

#[test]
fn emit_check_flags_a_missing_index_file() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);
    fixture.delete_file(".stele/index/hazards.md");

    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 1, "{}", check.combined());
    assert!(
        check.combined().contains(".stele/index/hazards.md"),
        "{}",
        check.combined()
    );
}

#[test]
fn emit_rewrites_only_between_markers_and_preserves_outside_prose() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);

    // Corrupt the region and add free prose AFTER the end marker (§3.1 item 3).
    fixture.replace("AGENTS.md", "## Hazards (2 active)", "## Hazards (WRONG)");
    fixture.append("AGENTS.md", "\nHand-authored prose outside the region.\n");

    assert_eq!(fixture.run(&["emit"]).code, 0);
    let restored = fixture.read("AGENTS.md");
    assert!(
        restored.contains("## Hazards (2 active)"),
        "region not restored: {restored}"
    );
    assert!(
        restored.contains("Hand-authored prose outside the region."),
        "outside prose lost: {restored}"
    );
}

// ─── malformed markers (exit 2, naming the file) ─────────────────────────────

/// Corrupt billing's markers, then assert `emit` is an exit-2 input error naming it.
fn assert_marker_exit_2(mutate: impl FnOnce(&Fixture)) {
    let fixture = built();
    mutate(&fixture);
    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 2, "{}", emit.combined());
    assert!(
        emit.combined().contains("apps/web/lib/billing/AGENTS.md"),
        "{}",
        emit.combined()
    );
}

#[test]
fn emit_rejects_begin_without_end() {
    assert_marker_exit_2(|f| {
        f.replace("apps/web/lib/billing/AGENTS.md", "<!-- stele:end -->\n", "");
    });
}

#[test]
fn emit_rejects_end_without_begin() {
    assert_marker_exit_2(|f| {
        f.replace(
            "apps/web/lib/billing/AGENTS.md",
            "<!-- stele:begin router -->\n",
            "",
        );
    });
}

#[test]
fn emit_rejects_two_begin_markers() {
    assert_marker_exit_2(|f| {
        f.replace(
            "apps/web/lib/billing/AGENTS.md",
            "<!-- stele:begin router -->\n",
            "<!-- stele:begin router -->\n<!-- stele:begin router -->\n",
        );
    });
}

#[test]
fn emit_rejects_a_node_with_no_region_pointing_at_init() {
    let fixture = built();
    // apps/web is a node (has a stele block) whose region we strip entirely.
    fixture.replace(
        "apps/web/AGENTS.md",
        "<!-- stele:begin router -->\n<!-- stele:end -->\n",
        "",
    );
    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 2, "{}", emit.combined());
    assert!(
        emit.combined().contains("apps/web/AGENTS.md"),
        "{}",
        emit.combined()
    );
    assert!(emit.combined().contains("init"), "{}", emit.combined());
}

// ─── transpose indexes (§6.1) ────────────────────────────────────────────────

#[test]
fn emit_writes_deterministic_transpose_indexes() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);

    let invariants = fixture.read(".stele/index/invariants.md");
    let hazards = fixture.read(".stele/index/hazards.md");

    // Ordered by node id then slug: the system node's money-type precedes billing's.
    assert!(
        invariants.find("lm:money-type").unwrap()
            < invariants.find("lm:billing-idempotency").unwrap(),
        "{invariants}"
    );
    // Hazards: billing's node id sorts before worker's.
    assert!(
        hazards.find("lm:webhook-verify").unwrap() < hazards.find("lm:dunning-batch").unwrap(),
        "{hazards}"
    );

    // Byte-stable across a re-emit.
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(fixture.read(".stele/index/invariants.md"), invariants);
    assert_eq!(fixture.read(".stele/index/hazards.md"), hazards);
}

// ─── CLAUDE.md shim (§3.3) ───────────────────────────────────────────────────

#[test]
fn emit_creates_the_claude_shim_when_missing() {
    let fixture = built();
    fixture.delete_file("CLAUDE.md");
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(fixture.read("CLAUDE.md"), "@AGENTS.md\n");
}

#[test]
fn emit_never_overwrites_a_teams_claude_file() {
    let fixture = built();
    fixture.write("CLAUDE.md", "@AGENTS.md\n\n# team additions\n");
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(
        fixture.read("CLAUDE.md"),
        "@AGENTS.md\n\n# team additions\n"
    );
}

// ─── --claude-rules opt-in (§3.3) ────────────────────────────────────────────

#[test]
fn claude_rules_are_opt_in() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert!(
        !fixture.path(".claude/rules").exists(),
        "plain emit must not write .claude/rules"
    );

    assert_eq!(fixture.run(&["emit", "--claude-rules"]).code, 0);
    // One file per node with claims: the system node, billing, worker.
    assert!(fixture.path(".claude/rules/root.md").exists());
    assert!(
        fixture
            .path(".claude/rules/apps-web-lib-billing.md")
            .exists()
    );
    assert!(fixture.path(".claude/rules/apps-worker.md").exists());
    // No file for a claim-free node.
    assert!(!fixture.path(".claude/rules/packages-shared.md").exists());

    let billing = fixture.read(".claude/rules/apps-web-lib-billing.md");
    assert!(billing.contains("node: apps/web/lib/billing"), "{billing}");
    assert!(billing.contains("lm:refund-cap"), "{billing}");
}

/// The marker line that gates every generated `.claude/rules/*.md` overwrite (§3.3, F8).
const CLAUDE_RULE_MARKER: &str =
    "<!-- stele:generated — do not edit; regenerated by `stele emit --claude-rules` -->";

// P3 (F8): `emit --claude-rules` must NEVER clobber a hand-authored .claude/rules file.
// A generated file opens with the stele marker; a target lacking it is foreign and makes
// emit exit 2 naming it (the file left untouched), while a marker-carrying target
// regenerates in place.
#[test]
fn claude_rules_refuses_to_overwrite_a_foreign_file() {
    let fixture = built();

    // Every generated file opens with the marker.
    assert_eq!(fixture.run(&["emit", "--claude-rules"]).code, 0);
    let root_rule = fixture.read(".claude/rules/root.md");
    assert!(
        root_rule.starts_with(CLAUDE_RULE_MARKER),
        "generated rule must open with the marker:\n{root_rule}"
    );

    // A marker-carrying target regenerates in place (idempotent), exit 0.
    let rerun = fixture.run(&["emit", "--claude-rules"]);
    assert_eq!(rerun.code, 0, "{}", rerun.combined());
    assert_eq!(fixture.read(".claude/rules/root.md"), root_rule);

    // A hand-authored file at a target slug (no marker) is foreign: exit 2 naming it,
    // bytes preserved.
    let foreign = "# my own billing rules\n\nhand-authored, keep me.\n";
    fixture.write(".claude/rules/apps-web-lib-billing.md", foreign);
    let clobber = fixture.run(&["emit", "--claude-rules"]);
    let out = clobber.combined();
    assert_eq!(
        clobber.code, 2,
        "foreign file must abort with exit 2: {out}"
    );
    assert!(
        out.contains("apps-web-lib-billing.md"),
        "the error must name the foreign file: {out}"
    );
    assert_eq!(
        fixture.read(".claude/rules/apps-web-lib-billing.md"),
        foreign,
        "the foreign file must be left byte-identical"
    );
}

// ─── §4.4 budget chain counts plain (non-node) AGENTS.md on the path (F6) ─────

// A VCS-tracked AGENTS.md that declares no `stele` node is still a SPEC §3.1 degradation
// file every Codex/Claude harness loads on the directory path it walks. The codex budget
// chain (§4.4) must therefore count it — walking node ids alone lets a plain AGENTS.md
// sitting between two nodes escape the truncation check (the F6 defect). The extra file is
// built in the SCRATCH copy at runtime, never committed to the fixture.
#[test]
fn budget_codex_counts_a_plain_agents_md_between_nodes() {
    const CODEX_CAP_BYTES: usize = 32 * 1024;

    let fixture = Fixture::acme();
    // apps/web/lib is a real directory on the path to apps/web/lib/billing (and store) but
    // backs NO node. A plain AGENTS.md here — sized past the codex cap — overflows every
    // leaf chain routed through it, but ONLY if that chain counts non-node files.
    let mut plain = String::from("# lib overview\n\nNot a stele node — plain guidance.\n\n");
    while plain.len() <= CODEX_CAP_BYTES {
        plain.push_str("Keep billing and store logic separate; never cross-import. ");
    }
    fixture.write("apps/web/lib/AGENTS.md", &plain);
    fixture.commit("add a plain (non-node) AGENTS.md between web and billing");

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "a tree with a plain AGENTS.md must still build"
    );
    let check = fixture.run(&["check"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    // budget[codex]: the overflow is reported against the byte cap's config key.
    assert!(out.contains("codex"), "{out}");
    assert!(out.contains("project_doc_max_bytes"), "{out}");
    // The overflowing leaf sits UNDER apps/web/lib — proof the plain file was counted.
    assert!(
        out.contains("apps/web/lib/billing") || out.contains("apps/web/lib/store"),
        "{out}"
    );
}
