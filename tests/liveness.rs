//! The liveness class (SPEC §4.6), driven through the real binary on acme-fixture
//! copies. Every declared command is tokenized (POSIX-flavored word-splitting), leading
//! `VAR=value` assignments are dropped, shell operators (`&&`, `||`, `|`, `;`) split it
//! into independently-resolved segments, and each segment's first token resolves on PATH,
//! as a repo-relative executable, or as a task/script/recipe of a detected runner
//! (mix/cargo/just/npm/pnpm/yarn). `--run-commands` additionally executes each command.
//!
//! Runner/tool names resolve host-independently because [`common::Fixture::run`] forces
//! the child PATH through a `stub-bin` of exit-0 stubs (mise, mix, cargo, …), so these
//! probes are identical on a dev box and a bare CI host.

mod common;

use common::Fixture;

/// Count the `✗ liveness:` finding lines in a `check` render.
fn liveness_lines(out: &str) -> usize {
    out.lines().filter(|l| l.contains("✗ liveness")).count()
}

/// Insert command entries at the head of the root `commands:` map (order is irrelevant
/// to the graph, so prepending is safe) and commit.
fn add_root_commands(fixture: &Fixture, entries: &str) {
    fixture.replace("AGENTS.md", "commands:\n", &format!("commands:\n{entries}"));
    fixture.commit("add probe commands");
}

// Probe 1 — a clean acme resolves every command: mise (PATH stub), mix builtins
// (`deps.get`, `test`), and mix aliases (`precommit`, `ecto.reset`). Liveness is clean.
#[test]
fn probe1_clean_acme_liveness_is_clean() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check", "--only", "liveness"]);
    let out = check.combined();
    assert_eq!(check.code, 0, "{out}");
    assert_eq!(liveness_lines(&out), 0, "{out}");
}

// Probe 2 — deleting the `ecto.reset` mix alias un-resolves `mix ecto.reset` (EXAMPLE
// 8.6b), through `--only liveness` in isolation from the other classes.
#[test]
fn probe2_mix_alias_deletion_unresolves() {
    let fixture = Fixture::acme();
    fixture.delete_line_containing("mix.exs", "\"ecto.reset\":");
    fixture.commit("remove the ecto.reset mix alias");

    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check", "--only", "liveness"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("command / :db-reset"), "{out}");
    assert!(out.contains("`mix ecto.reset`"), "{out}");
    assert!(out.contains("task not found in mix.exs"), "{out}");
    assert_eq!(liveness_lines(&out), 1, "{out}");
}

// Probe 3 — a leading `VAR=value` assignment is skipped (so `mix` is seen as the
// executable), and an unknown mix task is flagged. The `precommit` alias still present
// means the reason is the task, not the runner.
#[test]
fn probe3_env_assignment_skipped_and_unknown_mix_task() {
    let fixture = Fixture::acme();
    add_root_commands(&fixture, "  migrate: MIX_ENV=prod mix nosuchtask\n");

    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check", "--only", "liveness"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("command / :migrate"), "{out}");
    assert!(out.contains("`MIX_ENV=prod mix nosuchtask`"), "{out}");
    assert!(out.contains("task not found in mix.exs"), "{out}");
    assert_eq!(liveness_lines(&out), 1, "{out}");
}

// Probe 4 — shell operators split a command into segments resolved independently: only
// the `mix bogus` segment fails; the `mise install` and `true` segments resolve.
#[test]
fn probe4_shell_operators_split_segments() {
    let fixture = Fixture::acme();
    add_root_commands(&fixture, "  chain: mise install && mix bogus || true\n");

    assert_eq!(fixture.run(&["build"]).code, 0);
    let check = fixture.run(&["check", "--only", "liveness"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(out.contains("command / :chain → `mix bogus`"), "{out}");
    assert!(out.contains("task not found in mix.exs"), "{out}");
    // Exactly one segment failed — the clean `mise install` / `true` segments do not.
    assert_eq!(liveness_lines(&out), 1, "{out}");
}

// Probe 5 — runner variety: npm/yarn scripts (root package.json), cargo subcommands
// (builtin + `.cargo/config.toml` alias), and just recipes (justfile). The valid tasks
// resolve; the three missing ones each produce their runner-specific finding.
#[test]
fn probe5_runner_variety_npm_cargo_just() {
    let fixture = Fixture::acme();
    fixture.write(
        "package.json",
        "{\n  \"scripts\": { \"build\": \"tsc\", \"lint\": \"eslint .\" }\n}\n",
    );
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    fixture.write(
        ".cargo/config.toml",
        "[alias]\nxtask = \"run --package xtask\"\n",
    );
    fixture.write(
        "justfile",
        "build:\n\techo hi\ndeploy target:\n\techo {{target}}\n",
    );
    add_root_commands(
        &fixture,
        "  js-ok: npm run build\n  \
         js-bad: yarn run missing\n  \
         rs-ok: cargo xtask\n  \
         rs-builtin: cargo build\n  \
         rs-bad: cargo frobnicate\n  \
         just-ok: just deploy prod\n  \
         just-bad: just teardown\n",
    );

    assert_eq!(
        fixture.run(&["build"]).code,
        0,
        "{}",
        fixture.run(&["build"]).combined()
    );
    let check = fixture.run(&["check", "--only", "liveness"]);
    let out = check.combined();
    assert_eq!(check.code, 1, "{out}");
    assert!(
        out.contains("`yarn run missing` — script not found in package.json"),
        "{out}"
    );
    assert!(
        out.contains("`cargo frobnicate` — subcommand not found"),
        "{out}"
    );
    assert!(
        out.contains("`just teardown` — recipe not found in justfile"),
        "{out}"
    );
    // Only the three missing tasks fail; the four valid ones (script, alias, builtin,
    // recipe) resolve.
    assert_eq!(liveness_lines(&out), 3, "{out}");
}

// Probe 6 — `--run-commands` executes each command from the repo root. All the stubbed
// acme commands exit 0; a `false` command exits non-zero and is reported. Resolution
// alone (without the flag) leaves it clean, proving the flag gates execution.
#[test]
fn probe6_run_commands_executes_and_reports_nonzero() {
    let fixture = Fixture::acme();
    add_root_commands(&fixture, "  flake: false\n");
    assert_eq!(fixture.run(&["build"]).code, 0);

    // Resolution only: `false` is on PATH, so nothing fires.
    let resolve = fixture.run(&["check", "--only", "liveness"]);
    assert_eq!(resolve.code, 0, "{}", resolve.combined());

    // With execution: `false` exits 1 and is reported; the stubbed runners exit 0.
    let run = fixture.run(&["check", "--only", "liveness", "--run-commands"]);
    let out = run.combined();
    assert_eq!(run.code, 1, "{out}");
    assert!(
        out.contains("command / :flake → `false` — exited 1"),
        "{out}"
    );
    assert_eq!(liveness_lines(&out), 1, "{out}");
}
