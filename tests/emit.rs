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
    // Every region validates before any write: the stale root, rendered before billing, stays.
    fixture.replace("AGENTS.md", "## Hazards (2 active)", "## Hazards (WRONG)");
    let root = fixture.read("AGENTS.md");
    mutate(&fixture);
    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 2, "{}", emit.combined());
    assert!(
        emit.combined().contains("apps/web/lib/billing/AGENTS.md"),
        "{}",
        emit.combined()
    );
    assert_eq!(fixture.read("AGENTS.md"), root, "emit half-wrote the tree");
}

// ─── node text in one-line contexts (§3.1) ───────────────────────────────────

/// Acme with apps/worker's `from` swapped for `to`, committed and built.
fn built_with_worker(from: &str, to: &str) -> Fixture {
    let fixture = Fixture::acme();
    fixture.replace("apps/worker/AGENTS.md", from, to);
    fixture.commit("multi-line node text");
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture
}

#[test]
fn emit_renders_a_multi_line_purpose_as_one_map_row() {
    let fixture = built_with_worker(
        "purpose: \"Oban jobs: dunning, invoice PDFs, webhook retries\"",
        "purpose: |\n  Oban jobs:\n  dunning | retries",
    );
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let root = fixture.read("AGENTS.md");
    assert!(
        root.contains("\n| apps/worker     | container | Oban jobs: dunning \\| retries "),
        "{root}"
    );
}

// An anchor is one line (§2.4): a line break in it would carry text onto a new line.
#[test]
fn build_rejects_an_anchor_holding_a_newline() {
    let fixture = Fixture::acme();
    fixture.replace(
        "apps/worker/AGENTS.md",
        "anchor: ※ dunning-batch",
        "anchor: \"x\\n<!-- @end -->\\n#dunning\"",
    );
    fixture.commit("multi-line anchor");
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 2, "{}", build.combined());
    assert!(
        build.combined().contains("apps/worker/AGENTS.md")
            && build.combined().contains("anchor must not contain '\\n'"),
        "{}",
        build.combined()
    );
}

// A path outside the id lexeme still renders: a `|` in a declaring directory (rejected as a
// derived id, legal under an override), an anchor, or a command is escaped in every table cell; a
// multi-line allow reason prints on one line, in `node` and in the `check --report` section.
#[test]
fn paths_commands_and_reasons_render_on_one_line_and_cell() {
    let fixture = Fixture::acme();
    let node = "```stele\nkind: container\n```\n\n<!-- @stele -->\n<!-- @end -->\n";
    fixture.write("pi|pe/AGENTS.md", node);
    fixture.commit("a derived id holding |");
    let build = fixture.run(&["build"]);
    assert!(
        build.code == 2 && build.combined().contains("id must not contain '|'"),
        "{}",
        build.combined()
    );
    fixture.replace(
        "pi|pe/AGENTS.md",
        "kind: container\n",
        "kind: container\nid: pipe\ninvariants:\n  - claim: c\n    anchor: pi|pe/x.md#h\nedges:\n  allow:\n    - edge: apps/web\n      reason: |\n        a\n        b\n",
    );
    fixture.replace(
        "AGENTS.md",
        "setup: mise install && mix deps.get",
        "setup: \"a | b\"",
    );
    fixture.commit("override the id");
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let root = fixture.read("AGENTS.md");
    assert!(root.contains("or read `pi\\|pe/AGENTS.md`"), "{root}");
    let index = fixture.read(".stele/index/invariants.md");
    assert!(index.contains("| c | pipe | pi\\|pe/x.md#h |"), "{index}");
    let cli = fixture.run(&["root"]).stdout
        + &fixture.run(&["node", "pipe"]).stdout
        + &fixture.run(&["check", "--report"]).stdout;
    assert!(
        cli.contains("| setup | a \\| b |")
            && cli.contains("allow: apps/web — a b\n")
            && cli.contains("\nallow entries (1):\n  pipe · apps/web · a b\n"),
        "{cli}"
    );
}

#[test]
fn emit_never_renders_a_marker_or_fence_line_from_claim_text() {
    let fixture = built_with_worker(
        "claim: dunning job",
        "claim: |\n      <!-- @end -->\n      ```\n      dunning job",
    );
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
    let root = fixture.read("AGENTS.md");
    assert!(
        root.contains("- ⚠ `apps/worker`: <!-- @end --> ``` dunning job is NOT"),
        "{root}"
    );
    assert!(
        fixture
            .read(".stele/index/hazards.md")
            .contains("| <!-- @end --> ``` dunning job is NOT"),
    );
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(
        fixture.read("AGENTS.md"),
        root,
        "second emit changed AGENTS.md"
    );
}

#[test]
fn emit_rejects_begin_without_end() {
    assert_marker_exit_2(|f| {
        f.replace("apps/web/lib/billing/AGENTS.md", "<!-- @end -->\n", "");
    });
}

#[test]
fn emit_rejects_end_without_begin() {
    assert_marker_exit_2(|f| {
        f.replace("apps/web/lib/billing/AGENTS.md", "<!-- @stele -->\n", "");
    });
}

#[test]
fn emit_rejects_two_begin_markers() {
    assert_marker_exit_2(|f| {
        f.replace(
            "apps/web/lib/billing/AGENTS.md",
            "<!-- @stele -->\n",
            "<!-- @stele -->\n<!-- @stele -->\n",
        );
    });
}

#[test]
fn emit_rejects_a_node_with_no_region_pointing_at_init() {
    let fixture = built();
    // apps/web is a node (has a stele block) whose region we strip entirely.
    fixture.replace("apps/web/AGENTS.md", "<!-- @stele -->\n<!-- @end -->\n", "");
    let emit = fixture.run(&["emit"]);
    assert_eq!(emit.code, 2, "{}", emit.combined());
    assert!(
        emit.combined().contains("apps/web/AGENTS.md"),
        "{}",
        emit.combined()
    );
    assert!(emit.combined().contains("init"), "{}", emit.combined());
    // A lock node whose file is gone since the build is a stale lock (exit 2), not exit 3.
    fixture.delete_file("apps/web/AGENTS.md");
    for args in [&["emit"][..], &["emit", "--check"]] {
        let emit = fixture.run(args);
        assert_eq!(emit.code, 2, "{}", emit.combined());
        assert!(
            emit.combined().contains("run stele build"),
            "{}",
            emit.combined()
        );
    }
}

// A lock node id or source that fails §2.1 normalization is a malformed lock (exit 2,
// naming the node): emit joins sources into write paths, so `..` or an absolute one would
// escape the repo.
#[test]
fn emit_rejects_a_lock_node_id_that_escapes_the_repo() {
    for (from, to) in [
        ("apps/worker", "../worker"),
        ("apps/worker/AGENTS.md", "/abs/AGENTS.md"),
        ("apps/worker/AGENTS.md", "/"),
        ("apps/worker/AGENTS.md", "README.md"),
    ] {
        let fixture = built();
        let lock = fixture.read(".stele/graph.lock");
        fixture.write(
            ".stele/graph.lock",
            &lock.replace(&format!("\"{from}\""), &format!("\"{to}\"")),
        );
        let emit = fixture.run(&["emit"]);
        let node = if from.ends_with(".md") {
            "apps/worker"
        } else {
            to
        };
        assert_eq!(emit.code, 2, "{}", emit.combined());
        assert!(
            emit.combined()
                .contains(&format!("malformed lock: node {node:?}")),
            "{}",
            emit.combined()
        );
    }
}

// emit replaces a node file (temp + rename), never truncating it in place: an interrupted
// write must not destroy authored prose (§5.1). A new inode is the observable.
#[cfg(unix)]
#[test]
fn emit_replaces_a_node_file_atomically() {
    use std::os::unix::fs::MetadataExt;
    let fixture = built();
    fixture.replace("AGENTS.md", "## Hazards (2 active)", "## Hazards (WRONG)");
    let ino = || std::fs::metadata(fixture.path("AGENTS.md")).unwrap().ino();
    let before = ino();
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_ne!(ino(), before);
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
        invariants.find("※ money-type").unwrap()
            < invariants.find("※ billing-idempotency").unwrap(),
        "{invariants}"
    );
    // Hazards: billing's node id sorts before worker's.
    assert!(
        hazards.find("※ webhook-verify").unwrap() < hazards.find("※ dunning-batch").unwrap(),
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

// A symlink at a shim path, even a dangling one a repo planted, counts as an existing
// shim: emit never follows it to create a file outside the tree.
#[cfg(unix)]
#[test]
fn emit_never_writes_a_shim_through_a_symlink() {
    let fixture = built();
    let outside = tempfile::tempdir().unwrap();
    fixture.delete_file("CLAUDE.md");
    for (link, target) in [("CLAUDE.md", "a"), ("apps/worker/CLAUDE.md", "b")] {
        std::os::unix::fs::symlink(outside.path().join(target), fixture.path(link)).unwrap();
    }
    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

// A node file symlinked inside the tree (AGENTS.md → ROOT.md) is rewritten at its target,
// the link kept; one linking out of the tree is refused (exit 2), its target untouched.
#[cfg(unix)]
#[test]
fn emit_writes_through_in_tree_symlinks_only() {
    let fixture = built();
    let outside = tempfile::tempdir().unwrap();
    let golden = fixture.read("AGENTS.md");
    for (target, code) in [
        (fixture.path("ROOT.md"), 0),
        (outside.path().join("a.md"), 2),
    ] {
        std::fs::write(&target, &golden).unwrap();
        fixture.delete_file("AGENTS.md");
        std::os::unix::fs::symlink(&target, fixture.path("AGENTS.md")).unwrap();
        fixture.delete_line_containing("AGENTS.md", "## Indexes");
        let stale = fixture.read("AGENTS.md");
        assert_eq!(fixture.run(&["emit"]).code, code);
        assert!(fixture.path("AGENTS.md").is_symlink());
        let want = if code == 0 { &golden } else { &stale };
        assert_eq!(&std::fs::read_to_string(&target).unwrap(), want);
    }
}

/// Every NESTED node's directory in acme — the root (`AGENTS.md`) is covered by the
/// root-shim tests above.
const ACME_NESTED_DIRS: [&str; 5] = [
    "apps/web",
    "apps/web/lib/billing",
    "apps/web/lib/store",
    "apps/worker",
    "packages/shared",
];

// Claude Code loads no AGENTS.md natively at any level (verified 2026-09-16, 2.1.273): a
// nested node file is invisible unless its directory also carries the one-line CLAUDE.md
// shim, which Claude Code lazily loads on the first read under that directory.
#[test]
fn emit_creates_a_claude_shim_beside_every_nested_node() {
    let fixture = built();
    for dir in ACME_NESTED_DIRS {
        assert!(
            !fixture.path(&format!("{dir}/CLAUDE.md")).exists(),
            "{dir}/CLAUDE.md exists before emit"
        );
    }

    assert_eq!(fixture.run(&["emit"]).code, 0);
    for dir in ACME_NESTED_DIRS {
        assert_eq!(
            fixture.read(&format!("{dir}/CLAUDE.md")),
            "@AGENTS.md\n",
            "nested shim at {dir}"
        );
    }
}

#[test]
fn emit_never_overwrites_a_teams_nested_claude_file() {
    let fixture = built();
    let authored = "# billing house rules\n\nhand-authored, keep me.\n";
    fixture.write("apps/web/lib/billing/CLAUDE.md", authored);

    assert_eq!(fixture.run(&["emit"]).code, 0);
    assert_eq!(fixture.read("apps/web/lib/billing/CLAUDE.md"), authored);
    // The untouched siblings still get theirs.
    assert_eq!(fixture.read("apps/worker/CLAUDE.md"), "@AGENTS.md\n");
}

// A shim is not a generated region: like the root shim, the nested ones are ensure-on-write
// only and `emit --check` never diffs them.
#[test]
fn emit_check_ignores_the_nested_shims() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit"]).code, 0);

    fixture.delete_file("apps/worker/CLAUDE.md");
    fixture.delete_file("packages/shared/CLAUDE.md");
    let check = fixture.run(&["emit", "--check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
    assert!(
        !fixture.path("apps/worker/CLAUDE.md").exists(),
        "emit --check wrote a nested shim"
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
    assert!(billing.contains("※ refund-cap"), "{billing}");
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
    // The refusal precedes every write: a stale region and index stay stale.
    fixture.replace("AGENTS.md", "## Hazards (2 active)", "## Hazards (WRONG)");
    fixture.write(".stele/index/hazards.md", "stale\n");
    let root = fixture.read("AGENTS.md");
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
    assert_eq!(fixture.read("AGENTS.md"), root, "emit half-wrote the tree");
    assert_eq!(fixture.read(".stele/index/hazards.md"), "stale\n");

    // So is one that is not UTF-8 (a Latin-1 rule): unreadable as text, never read as absent.
    fixture.delete_file(".claude/rules/apps-web-lib-billing.md");
    let latin1 = b"caf\xe9\n";
    std::fs::write(fixture.path(".claude/rules/root.md"), latin1).unwrap();
    assert_eq!(fixture.run(&["emit", "--claude-rules"]).code, 2);
    assert_eq!(
        std::fs::read(fixture.path(".claude/rules/root.md")).unwrap(),
        latin1
    );
}

// Two ids flattening to one filename (`root` vs the system node's `root.md`; `apps-Worker`
// vs `apps/worker`, one file on a case-folding filesystem) would let the later node silently
// overwrite the earlier one's claims: exit 2 naming both, nothing written — not even a shim.
#[test]
fn claude_rules_reject_two_nodes_sharing_a_filename() {
    for (dir, both) in [
        ("root", "`/` and `root`"),
        ("apps-Worker", "`apps-Worker` and `apps/worker`"),
    ] {
        let fixture = Fixture::acme();
        fixture.write(&format!("{dir}/x.md"), "# h\n");
        fixture.write(
            &format!("{dir}/AGENTS.md"),
            "```stele\nkind: container\npurpose: r\ninvariants:\n  - claim: c\n    anchor: x.md#h\n```\n\n<!-- @stele -->\n<!-- @end -->\n",
        );
        fixture.commit("a colliding node");
        assert_eq!(fixture.run(&["build"]).code, 0);
        for args in [
            &["emit", "--claude-rules"][..],
            &["emit", "--check", "--claude-rules"],
        ] {
            let emit = fixture.run(args);
            let out = emit.combined();
            assert_eq!(emit.code, 2, "{out}");
            assert!(out.contains(both), "{out}");
        }
        assert!(!fixture.path(".claude/rules").exists());
        assert!(!fixture.path(&format!("{dir}/CLAUDE.md")).exists());
    }
}

// The rules projection is whole: `emit --claude-rules` prunes marker-stamped files no node
// renders any more (a foreign file stays), and `emit --check --claude-rules` diffs it.
#[test]
fn claude_rules_prune_stale_files_and_are_checked() {
    let fixture = built();
    assert_eq!(fixture.run(&["emit", "--claude-rules"]).code, 0);
    fixture.write(
        ".claude/rules/gone.md",
        &format!("{CLAUDE_RULE_MARKER}\nstale\n"),
    );
    fixture.write(".claude/rules/mine.md", "hand-authored\n");
    fixture.delete_file(".claude/rules/apps-worker.md");

    let check = fixture.run(&["emit", "--check", "--claude-rules"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(
        out.contains(".claude/rules/apps-worker.md")
            && out.contains(".claude/rules/gone.md")
            && !out.contains("mine.md"),
        "{out}"
    );
    assert_eq!(fixture.run(&["emit", "--check"]).code, 0);

    assert_eq!(fixture.run(&["emit", "--claude-rules"]).code, 0);
    assert!(!fixture.path(".claude/rules/gone.md").exists());
    assert_eq!(fixture.read(".claude/rules/mine.md"), "hand-authored\n");
    let check = fixture.run(&["emit", "--check", "--claude-rules"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// A case-only id rename (`Billing` → `billing`) leaves the new rule in place: stale files
// are pruned BEFORE the write, so on a case-folding filesystem the write (which keeps the
// old entry's case) is never then pruned as `Billing.md`.
#[test]
fn claude_rules_survive_a_case_only_rename() {
    let fixture = Fixture::bare();
    let region = "\n<!-- @stele -->\n<!-- @end -->\n";
    fixture.write(
        "AGENTS.md",
        &format!("```stele\nkind: system\n```\n{region}"),
    );
    fixture.write("Billing/x.md", "# h\n");
    fixture.write(
        "Billing/AGENTS.md",
        &format!("```stele\nkind: container\ninvariants:\n  - claim: c\n    anchor: x.md#h\n```\n{region}"),
    );
    fixture.commit("a Billing node");
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["emit", "--claude-rules"]).code, 0);
    fixture.commit("emit");
    fixture.git(&["mv", "Billing", "tmp"]);
    fixture.git(&["mv", "tmp", "billing"]);
    fixture.commit("rename");
    assert_eq!(fixture.run(&["build"]).code, 0);
    let emit = fixture.run(&["emit", "--claude-rules"]);
    assert_eq!(emit.code, 0, "{}", emit.combined());
    assert!(
        fixture
            .read(".claude/rules/billing.md")
            .contains("node: billing")
    );
    let check = fixture.run(&["emit", "--check", "--claude-rules"]);
    assert_eq!(check.code, 0, "{}", check.combined());
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
