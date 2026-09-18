//! `stele migrate` (SPEC §5.1 command pipeline, §5.3 process contract): the one bridge
//! from the pre-0.3.0 `stele:` grammar to the glyph notation (§2.5). The oracle is a
//! round trip — the acme fixture pushed BACK to the retired notation must migrate to
//! byte-equality with the shipped fixture, file by file — plus the scoping rules that
//! keep a quotation a quotation, and the §5.3 exits (0 always, warn on a dirty tree).
//!
//! The retired tokens are never spelled literally in this file: `migrate` self-hosts over
//! this repo's own sources (Phase 4), and a literal followed by a space is exactly what it
//! rewrites. Every fixture builds them from the constants below.

mod common;

use common::{Fixture, RunResult};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The shipped acme fixture — the byte-equality oracle for a full migration.
const FIXTURE_ACME: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/acme");

// ─── the retired notation, never spelled literally ───────────────────────────

const LANDMARK: &str = "stele:landmark";
const CLAIM: &str = "stele:claim";
const REGION_BEGIN: &str = "<!-- stele:begin";
const REGION_END: &str = "<!-- stele:end -->";

/// The ten fixture files that carry retired notation once [`reverse`] has run: six node
/// files (block fields and/or region markers) and four code files (comment tokens).
const REVERSED_FILES: [&str; 10] = [
    "AGENTS.md",
    "apps/web/AGENTS.md",
    "apps/web/lib/billing/AGENTS.md",
    "apps/web/lib/billing/charge.ex",
    "apps/web/lib/billing/refund.ex",
    "apps/web/lib/store/AGENTS.md",
    "apps/worker/AGENTS.md",
    "apps/worker/lib/dunning.ex",
    "packages/shared/AGENTS.md",
    "packages/shared/src/money.ts",
];

/// Push one file's contents BACK to the pre-0.3.0 notation — the inverse of the rewrite
/// under test, scoped exactly as `migrate` scopes itself. Node files (`AGENTS.md`) lose
/// the glyphs in their authored `anchor:`/`decided_by:` fields and their region markers;
/// every other file loses its comment-token glyphs. Generated-region PROSE keeps its
/// glyphs: it was never notation the old scanner read (markdown counts HTML comments
/// only), so `migrate` must not touch it — and `emit` re-renders it either way.
fn reverse(rel: &str, contents: &str) -> String {
    if !rel.ends_with("AGENTS.md") {
        return contents
            .replace("※ ", &format!("{LANDMARK} "))
            .replace("⊨ ", &format!("{CLAIM} "));
    }
    let mut out = String::new();
    for line in contents.split_inclusive('\n') {
        let reversed = if let Some(rest) = line.trim_start().strip_prefix("decided_by:") {
            line.replace(rest, &rest.replace("§ ", "adr/"))
        } else {
            line.replace("anchor: ※ ", "anchor: lm:")
                .replace("<!-- @stele -->", &format!("{REGION_BEGIN} router -->"))
                .replace("<!-- @stele ", &format!("{REGION_BEGIN} "))
                .replace("<!-- @end -->", REGION_END)
        };
        out.push_str(&reversed);
    }
    out
}

/// Every file under `root` as a repo-root-relative POSIX path, `.git/` excluded, sorted.
fn walk(root: &Path) -> Vec<String> {
    fn visit(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                visit(&path, base, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .expect("under base")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

/// A committed acme copy whose every file has been pushed back to the pre-0.3.0 notation
/// — the state a repo is in the moment before its one `stele migrate`.
fn reversed_acme() -> Fixture {
    let fixture = Fixture::acme();
    let mut reversed = Vec::new();
    for rel in walk(&fixture.root) {
        let before = fixture.read(&rel);
        let after = reverse(&rel, &before);
        if after != before {
            fixture.write(&rel, &after);
            reversed.push(rel);
        }
    }
    assert_eq!(
        reversed, REVERSED_FILES,
        "the reversal touched a different file set than expected"
    );
    fixture.commit("revert acme to the pre-0.3.0 notation");
    fixture
}

/// Assert every file in the fixture is byte-identical to the shipped acme fixture.
fn assert_matches_shipped_fixture(fixture: &Fixture) {
    let shipped = PathBuf::from(FIXTURE_ACME);
    let expected = walk(&shipped);
    assert_eq!(walk(&fixture.root), expected, "the file roster drifted");
    for rel in expected {
        let want = fs::read_to_string(shipped.join(&rel)).expect("read shipped fixture");
        assert_eq!(
            fixture.read(&rel),
            want,
            "{rel} did not migrate byte-exactly"
        );
    }
}

// ─── 1. the round trip ───────────────────────────────────────────────────────

#[test]
fn a_pre_0_3_0_repo_migrates_to_byte_equality_with_the_shipped_fixture() {
    let fixture = reversed_acme();

    let migrate = fixture.run(&["migrate"]);
    let out = migrate.combined();
    assert_eq!(migrate.code, 0, "{out}");
    assert_matches_shipped_fixture(&fixture);

    // The report names every file it touched, and counts them (§5.1).
    assert!(out.contains("10 file(s)"), "{out}");
    for rel in REVERSED_FILES {
        assert!(out.contains(rel), "{rel} missing from the report:\n{out}");
    }
}

// ─── 2. idempotence (§5.3) ───────────────────────────────────────────────────

#[test]
fn a_second_run_rewrites_nothing_and_says_so() {
    let fixture = reversed_acme();
    assert_eq!(fixture.run(&["migrate"]).code, 0);
    fixture.commit("migrate to the glyph notation");

    let again = fixture.run(&["migrate"]);
    let out = again.combined();
    assert_eq!(again.code, 0, "{out}");
    assert!(out.contains("nothing to rewrite"), "{out}");
    assert!(out.contains("0 file(s)"), "{out}");
    assert_matches_shipped_fixture(&fixture);
}

// ─── 3. scan scope: `.steleignore` (§2.4) ────────────────────────────────────

#[test]
fn a_steleignored_file_is_left_untouched() {
    let fixture = Fixture::acme();
    let vendored = format!("# {LANDMARK} vendored-one\ndefmodule Vendor do\nend\n");
    fixture.write(".steleignore", "vendor/\n");
    fixture.write("vendor/legacy.ex", &vendored);
    fixture.commit("vendor a steleignored tree carrying the retired notation");

    let migrate = fixture.run(&["migrate"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());
    assert_eq!(fixture.read("vendor/legacy.ex"), vendored);
    assert!(
        !migrate.combined().contains("vendor/legacy.ex"),
        "{}",
        migrate.combined()
    );
}

// ─── 4. markdown scoping (§2.5) ──────────────────────────────────────────────

#[test]
fn markdown_migrates_html_comments_and_leaves_quotations_alone() {
    let fixture = Fixture::acme();
    let doc = format!(
        "# notes\n\nQuoting `{LANDMARK}` in prose.\n\n```elixir\n# {LANDMARK} fenced-one\n```\n\n\
         <!-- {LANDMARK} declared-one -->\n"
    );
    fixture.write("docs/notes.md", &doc);
    fixture.commit("a docs page that both quotes and declares");

    let migrate = fixture.run(&["migrate"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());

    let after = fixture.read("docs/notes.md");
    assert!(after.contains("<!-- ※ declared-one -->"), "{after}");
    assert!(
        after.contains(&format!("# {LANDMARK} fenced-one")),
        "a fenced quotation was rewritten:\n{after}"
    );
    assert!(
        after.contains(&format!("`{LANDMARK}` in prose")),
        "a prose quotation was rewritten:\n{after}"
    );
}

// ─── 5. region markers (§3.1 item 2) ─────────────────────────────────────────

#[test]
fn every_region_marker_form_migrates() {
    let fixture = Fixture::bare();
    let cases = [
        (
            "annotated",
            format!("{REGION_BEGIN} router · generated · do not hand-edit -->\n{REGION_END}\n"),
            "<!-- @stele router · generated · do not hand-edit -->\n<!-- @end -->\n",
        ),
        (
            "bare-router",
            format!("{REGION_BEGIN} router -->\n{REGION_END}\n"),
            "<!-- @stele -->\n<!-- @end -->\n",
        ),
        (
            "nameless",
            format!("{REGION_BEGIN} -->\n{REGION_END}\n"),
            "<!-- @stele -->\n<!-- @end -->\n",
        ),
        (
            "named",
            format!("{REGION_BEGIN} index -->\n{REGION_END}\n"),
            "<!-- @stele index -->\n<!-- @end -->\n",
        ),
        (
            "one-line",
            format!("{REGION_BEGIN} router -->{REGION_END}\n"),
            "<!-- @stele --><!-- @end -->\n",
        ),
    ];
    for (name, before, _) in &cases {
        fixture.write(&format!("{name}/AGENTS.md"), before);
    }
    fixture.commit("one node file per region-marker form");

    let migrate = fixture.run(&["migrate"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());
    for (name, _, expected) in &cases {
        assert_eq!(
            fixture.read(&format!("{name}/AGENTS.md")),
            *expected,
            "{name}"
        );
    }
}

// ─── 6. decision references (§2.6) ───────────────────────────────────────────

#[test]
fn decided_by_migrates_in_both_list_forms_and_under_any_adr_dir() {
    let fixture = Fixture::bare();
    fixture.write(
        "flow/AGENTS.md",
        "```stele\nkind: container\npurpose: flow\nedges:\n  \
         decided_by: [adr/0007, doc/adr/0012, docs/adr/0031]\n```\n",
    );
    fixture.write(
        "block/AGENTS.md",
        "```stele\nkind: container\npurpose: block\nedges:\n  decided_by:\n    - doc/adr/0007\n    \
         - adr/0012\n  depends:\n    - packages/shared\n```\n",
    );
    fixture.commit("two decided_by spellings");

    let migrate = fixture.run(&["migrate"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());

    let flow = fixture.read("flow/AGENTS.md");
    assert!(
        flow.contains("decided_by: [§ 0007, § 0012, § 0031]"),
        "{flow}"
    );
    let block = fixture.read("block/AGENTS.md");
    assert!(block.contains("    - § 0007\n    - § 0012\n"), "{block}");
    // A later block list is not a decision list — `depends` entries survive verbatim.
    assert!(block.contains("    - packages/shared\n"), "{block}");
}

// ─── 7. the migrated repo is green end to end (§5.1) ─────────────────────────

#[test]
fn migrate_then_build_leaves_check_and_emit_green() {
    let fixture = reversed_acme();
    assert_eq!(fixture.run(&["migrate"]).code, 0);
    fixture.commit("stele migrate");

    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
    // The fixture ships no `.stele/index/`, so materialize once before the byte-diff.
    assert_eq!(fixture.run(&["emit"]).code, 0);
    let emit_check = fixture.run(&["emit", "--check"]);
    assert_eq!(emit_check.code, 0, "{}", emit_check.combined());
}

// ─── 8. undercover (§3.5) ────────────────────────────────────────────────────

/// Run the binary in `dir`, the way the undercover suite does (only `git` is spawned, so
/// the inherited PATH suffices).
fn run_in(dir: &Path, args: &[&str]) -> RunResult {
    let out = common::isolate_git(Command::new(common::BIN).args(args).current_dir(dir))
        .output()
        .expect("spawn stele");
    RunResult {
        code: out.status.code().expect("stele terminated by signal"),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    }
}

#[test]
fn undercover_migrates_the_overlay_and_the_work_tree_without_leaking() {
    let fixture = Fixture::bare();
    fixture.write(
        "apps/web/lib/app.ex",
        &format!("defmodule Acme.App do\n  # {LANDMARK} app-core\n  def hello, do: :world\nend\n"),
    );
    fixture.commit("a work tree carrying the retired notation");
    assert_eq!(fixture.run(&["init", "--undercover"]).code, 0);

    // An overlay node authored in the retired notation: block field, decision, markers.
    fixture.write(
        ".stele/tree/AGENTS.md",
        &format!(
            "# root\n\n```stele\nkind: system\npurpose: undercover root\ninvariants:\n  \
             - claim: hello answers\n    anchor: lm:app-core\nedges:\n  decided_by: [adr/0007]\n\
             ```\n\n{REGION_BEGIN} router -->\n{REGION_END}\n"
        ),
    );
    let exclude_before = fs::read_to_string(fixture.path(".git/info/exclude")).expect("exclude");

    let migrate = fixture.run(&["migrate"]);
    let out = migrate.combined();
    assert_eq!(migrate.code, 0, "{out}");

    // Both halves of the undercover scope moved (§3.5): the overlay node source…
    let overlay = fixture.read(".stele/tree/AGENTS.md");
    assert!(overlay.contains("anchor: ※ app-core"), "{overlay}");
    assert!(overlay.contains("decided_by: [§ 0007]"), "{overlay}");
    assert!(
        overlay.contains("<!-- @stele -->\n<!-- @end -->\n"),
        "{overlay}"
    );
    // …and the tracked work-tree comment.
    assert!(
        fixture.read("apps/web/lib/app.ex").contains("# ※ app-core"),
        "{}",
        fixture.read("apps/web/lib/app.ex")
    );
    assert!(out.contains(".stele/tree/AGENTS.md"), "{out}");

    // The managed exclude fence is an internal block, NOT notation (§2.5/§3.5) — byte-identical.
    assert_eq!(
        fs::read_to_string(fixture.path(".git/info/exclude")).expect("exclude"),
        exclude_before
    );
    assert!(exclude_before.contains("# stele:begin undercover"), "fence");

    // Nothing the engine owns surfaces in `git status`: only the rewritten tracked source does.
    let status = String::from_utf8_lossy(
        &common::isolate_git(
            Command::new("git")
                .args(["status", "--porcelain"])
                .current_dir(&fixture.root),
        )
        .output()
        .expect("spawn git")
        .stdout,
    )
    .into_owned();
    assert_eq!(status.trim(), "M apps/web/lib/app.ex", "{status}");
}

#[test]
fn undercover_migrate_runs_from_a_subdirectory() {
    let fixture = Fixture::bare();
    fixture.write("apps/web/lib/app.ex", "defmodule Acme.App do\nend\n");
    fixture.commit("a bare work tree");
    assert_eq!(fixture.run(&["init", "--undercover"]).code, 0);
    fixture.write(
        ".stele/tree/apps/AGENTS.md",
        &format!("```stele\nkind: container\npurpose: apps\n```\n\n{REGION_BEGIN} router -->\n{REGION_END}\n"),
    );

    let migrate = run_in(&fixture.path("apps/web"), &["migrate"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());
    assert!(
        fixture
            .read(".stele/tree/apps/AGENTS.md")
            .contains("<!-- @stele -->\n<!-- @end -->\n"),
        "{}",
        fixture.read(".stele/tree/apps/AGENTS.md")
    );
}

// ─── 9. the dirty-tree warning (§5.1) ────────────────────────────────────────

#[test]
fn a_dirty_work_tree_warns_but_still_migrates() {
    let fixture = reversed_acme();
    fixture.write("README.md", "uncommitted\n");

    let migrate = fixture.run(&["migrate"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());
    assert!(
        migrate.stderr.contains("uncommitted changes"),
        "no dirty-tree warning:\n{}",
        migrate.combined()
    );
    // The warning is not a refusal — the rewrite still happened.
    assert!(
        fixture
            .read("apps/web/lib/billing/refund.ex")
            .contains("# ※ refund-cap"),
    );
}

// ─── §5.3: bad flags, and the clean-repo success ─────────────────────────────

#[test]
fn an_unknown_flag_is_an_input_error_and_writes_nothing() {
    let fixture = reversed_acme();
    let before = fixture.read("apps/web/lib/billing/refund.ex");

    let migrate = fixture.run(&["migrate", "--force"]);
    let out = migrate.combined();
    assert_eq!(migrate.code, 2, "{out}");
    assert!(out.contains("usage: stele migrate"), "{out}");
    assert_eq!(fixture.read("apps/web/lib/billing/refund.ex"), before);
}

#[test]
fn a_repo_with_nothing_to_migrate_exits_zero() {
    let fixture = Fixture::acme();
    let migrate = fixture.run(&["migrate"]);
    let out = migrate.combined();
    assert_eq!(migrate.code, 0, "{out}");
    assert!(out.contains("nothing to rewrite"), "{out}");
}

#[test]
fn the_json_envelope_carries_the_touched_files() {
    let fixture = reversed_acme();
    let migrate = fixture.run(&["migrate", "--json"]);
    assert_eq!(migrate.code, 0, "{}", migrate.combined());

    let envelope: serde_json::Value =
        serde_json::from_str(migrate.stdout.trim()).expect("one JSON envelope");
    assert_eq!(envelope["command"], "migrate");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["exit"], 0);
    assert_eq!(envelope["data"]["files"], REVERSED_FILES.len());
    let listed: Vec<String> = envelope["data"]["rewrote"]
        .as_array()
        .expect("rewrote array")
        .iter()
        .map(|v| v.as_str().expect("path").to_string())
        .collect();
    assert_eq!(listed, REVERSED_FILES);
}
