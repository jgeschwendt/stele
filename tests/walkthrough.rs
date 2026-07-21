//! SPEC §7 adoption path, in order: init → build → check --freeze → check. Two starting
//! points — a bare tree the test materializes, and the acme fixture whose hand-authored
//! AGENTS.md files init must leave byte-identical (idempotent, non-destructive). Ignored
//! until the CLI + init land (plan Phase E).

mod common;

use common::Fixture;

const ACME_AGENTS: [&str; 6] = [
    "AGENTS.md",
    "apps/web/AGENTS.md",
    "apps/web/lib/billing/AGENTS.md",
    "apps/web/lib/store/AGENTS.md",
    "apps/worker/AGENTS.md",
    "packages/shared/AGENTS.md",
];

// (a) On a bare two-directory tree with no AGENTS.md, the full adoption path is green.
#[test]
fn walkthrough_bare_tree_init_build_check() {
    let fixture = Fixture::bare();
    fixture.write(
        "apps/web/lib/app.ex",
        "defmodule Acme.App do\n  def hello, do: :world\nend\n",
    );
    fixture.write(
        "apps/web/lib/router.ex",
        "defmodule Acme.Router do\n  def route(_conn), do: :ok\nend\n",
    );
    fixture.write(
        "packages/shared/src/index.ts",
        "export const version = 1;\n",
    );
    fixture.commit("materialize a bare two-directory tree");

    assert_eq!(fixture.run(&["init"]).code, 0);
    assert_eq!(fixture.run(&["build"]).code, 0);
    assert_eq!(fixture.run(&["check", "--freeze"]).code, 0);
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// (b) On acme, init leaves every existing AGENTS.md byte-identical, then build → check.
#[test]
fn walkthrough_init_is_idempotent_on_acme() {
    let fixture = Fixture::acme();
    let before: Vec<String> = ACME_AGENTS.iter().map(|p| fixture.read(p)).collect();

    assert_eq!(fixture.run(&["init"]).code, 0);
    for (path, was) in ACME_AGENTS.iter().zip(&before) {
        assert_eq!(&fixture.read(path), was, "init mutated {path}");
    }

    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.combined());
}

// P2 (F10): `stele init` must NEVER abort mid-scaffold when a proposed target is
// gitignored. It pre-filters the ignored proposal (skipping it with a notice), scaffolds
// and stages the rest, and exits 0 — no half-done tree, no orphaned file at the ignored
// path.
#[test]
fn init_skips_a_gitignored_proposal_and_exits_zero() {
    let fixture = Fixture::bare();
    fixture.write("apps/web/app.ex", "defmodule Acme.App do\nend\n");
    fixture.write("vendor/thing.txt", "opaque\n");
    // The container `init` would propose for `vendor/` is gitignored.
    fixture.write(".gitignore", "vendor/AGENTS.md\n");
    fixture.commit("bare tree with a gitignored vendor node target");

    let init = fixture.run(&["init"]);
    assert_eq!(init.code, 0, "init must not abort: {}", init.combined());
    assert!(
        init.stderr.contains("vendor/AGENTS.md") && init.stderr.contains(".gitignore"),
        "expected a skip notice for the ignored target: {}",
        init.combined()
    );
    // The ignored path was never written (no orphaned, unstageable scaffold).
    assert!(
        !fixture.path("vendor/AGENTS.md").exists(),
        "init wrote into a gitignored path"
    );
    // The un-ignored proposals were scaffolded and staged; the tree builds cleanly. (The
    // skipped vendor/ dir stays legitimately uncovered — that is the correct consequence
    // of an untrackable node target, not a scaffolding failure.)
    assert!(fixture.path("AGENTS.md").exists());
    assert!(fixture.path("apps/AGENTS.md").exists());
    assert_eq!(fixture.run(&["build"]).code, 0);
}

// `.stele/` is the engine's own home (§4.3 IGNORED_DIRS): its `graph.lock` is VCS-tracked,
// so it surfaces in the tracked-file listing — but `init` must NEVER propose it as a node.
// After a build commits the lock, a re-`init` must leave `.stele/AGENTS.md` absent.
#[test]
fn init_never_proposes_the_dot_stele_dir_as_a_node() {
    let fixture = Fixture::bare();
    fixture.write("apps/web/app.ex", "defmodule Acme.App do\nend\n");
    fixture.commit("bare tree with one app dir");

    // First init + build lands a tracked .stele/graph.lock, so .stele/ now holds a tracked
    // file and appears in `git ls-files` — the exact condition that used to mis-propose it.
    assert_eq!(fixture.run(&["init"]).code, 0);
    assert_eq!(fixture.run(&["build"]).code, 0);
    fixture.commit("commit the graph lock");
    assert!(
        fixture.path(".stele/graph.lock").exists(),
        "precondition: the lock must be tracked so .stele/ is in the tracked listing"
    );

    let reinit = fixture.run(&["init"]);
    assert_eq!(reinit.code, 0, "{}", reinit.combined());
    assert!(
        !fixture.path(".stele/AGENTS.md").exists(),
        "init proposed .stele/ as a node: {}",
        reinit.combined()
    );
}

// `stele init --json` (the sole flag a mutating verb accepts, §5.3) still runs and prints
// exactly one success envelope to stdout.
#[test]
fn init_json_still_scaffolds_and_prints_one_envelope() {
    let fixture = Fixture::bare();
    fixture.write("apps/web/app.ex", "defmodule Acme.App do\nend\n");
    fixture.commit("bare tree for a --json init");

    let init = fixture.run(&["init", "--json"]);
    assert_eq!(init.code, 0, "{}", init.combined());
    let value: serde_json::Value =
        serde_json::from_str(init.stdout.trim()).expect("init --json stdout is one JSON object");
    assert_eq!(value["ok"], true, "{}", init.stdout);
    assert!(
        fixture.path("AGENTS.md").exists() && fixture.path("apps/AGENTS.md").exists(),
        "init --json did not scaffold the expected nodes"
    );
}
