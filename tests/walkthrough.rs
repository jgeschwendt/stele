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
#[ignore = "phase E"]
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
#[ignore = "phase E"]
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
