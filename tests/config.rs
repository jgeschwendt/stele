//! SPEC §3.4 config loader + §5.3 `--json` envelope. The loader is exercised directly
//! against `stele::config` (defaults, overrides, unknown-key rejection); the envelope
//! is exercised end-to-end through the built binary (shape + exit agreement).

mod common;

use common::Fixture;
use serde_json::Value;
use stele::config::{self, AssertionClass};
use stele::model::ExitCode;

// ─── §3.4 loader: defaults, overrides, strictness ─────────────────────────────

// Absent `.stele/config.toml` → every default (SPEC §3.4).
#[test]
fn absent_config_yields_all_defaults() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config = config::load(dir.path()).expect("absent file is not an error");

    assert_eq!(config.exhaustiveness.depth, 1);
    assert_eq!(
        config.exhaustiveness.exclude,
        ["node_modules", "_build", "deps", "target"]
    );
    assert_eq!(config.budget.claude_root, 2000);
    assert_eq!(config.budget.codex_cap, 32768);
    assert_eq!(config.freshness.churn_threshold, None);
    assert_eq!(config.freshness.enforced_leash, None);
    assert!(config.freshness.node.is_empty());
    assert!(config.check.disable.is_empty());
}

// A present config with every knob set — including a per-node freshness override and
// a `check.disable` list — round-trips to the typed values (SPEC §3.4).
#[test]
fn overrides_parse_into_typed_values() {
    let config = load_str(
        "[exhaustiveness]\n\
         depth = 3\n\
         exclude = [\"dist\"]\n\
         [budget]\n\
         claude_root = 1500\n\
         codex_cap = 4096\n\
         [freshness]\n\
         churn_threshold = 40\n\
         enforced_leash = 90\n\
         [freshness.node.\"apps/web\"]\n\
         enforced_leash = 30\n\
         [check]\n\
         disable = [\"freshness\", \"liveness\"]\n",
    );

    assert_eq!(config.exhaustiveness.depth, 3);
    assert_eq!(config.exhaustiveness.exclude, ["dist"]);
    assert_eq!(config.budget.claude_root, 1500);
    assert_eq!(config.budget.codex_cap, 4096);
    assert_eq!(config.freshness.churn_threshold, Some(40));
    assert_eq!(config.freshness.enforced_leash, Some(90));
    let web = config
        .freshness
        .node
        .get("apps/web")
        .expect("per-node override present");
    assert_eq!(web.enforced_leash, Some(30));
    assert_eq!(web.churn_threshold, None);
    assert_eq!(
        config.check.disable,
        [AssertionClass::Freshness, AssertionClass::Liveness]
    );
}

// An unknown key anywhere → input error (exit 2) naming the key (SPEC §3.4).
#[test]
fn unknown_key_is_input_error_naming_the_key() {
    let err = load_str_err("[budget]\nclaude_root = 10\nbogus_key = 1\n");
    assert_eq!(err.exit, ExitCode::Input);
    assert!(err.message.contains("bogus_key"), "{}", err.message);
}

// An unknown `check.disable` class → input error (exit 2) naming the bad value (§3.4:
// the list is drawn from the six assertion classes).
#[test]
fn unknown_disable_class_is_input_error() {
    let err = load_str_err("[check]\ndisable = [\"referential\", \"nope\"]\n");
    assert_eq!(err.exit, ExitCode::Input);
    assert!(err.message.contains("nope"), "{}", err.message);
}

// The loader wiring reaches `check` end-to-end: a bad config makes `check` exit 2 and
// name the offending key, before any lock work (SPEC §3.4 read by check/emit).
#[test]
fn bad_config_makes_check_exit_2() {
    let fixture = Fixture::acme();
    fixture.write(".stele/config.toml", "[exhaustiveness]\nnonsense = true\n");
    fixture.commit("add a malformed config");

    let check = fixture.run(&["check"]);
    assert_eq!(check.code, 2, "{}", check.combined());
    assert!(
        check.combined().contains("nonsense"),
        "{}",
        check.combined()
    );
}

// ─── §5.3 `--json` envelope: shape + exit agreement ───────────────────────────

// `build --json` prints exactly one envelope with the §5.3 shape, ok:true, exit:0.
#[test]
fn build_json_envelope_shape() {
    let fixture = Fixture::acme();
    let build = fixture.run(&["build", "--json"]);
    assert_eq!(build.code, 0, "{}", build.combined());

    let envelope = sole_json(&build.stdout);
    assert_envelope_keys(&envelope);
    assert_eq!(envelope["stele"], env!("CARGO_PKG_VERSION"));
    assert_eq!(envelope["command"], "build");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["exit"], 0);
    assert_eq!(envelope["findings"], Value::Array(vec![]));
}

// `check --json` over a freshly-built clean acme: one envelope, ok:true, exit:0, and
// the process exit code agrees with the envelope's `exit`.
#[test]
fn check_json_envelope_shape_and_exit_agreement() {
    let fixture = Fixture::acme();
    assert_eq!(fixture.run(&["build"]).code, 0);

    let check = fixture.run(&["check", "--json"]);
    let envelope = sole_json(&check.stdout);
    assert_envelope_keys(&envelope);
    assert_eq!(envelope["command"], "check");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["exit"], 0);
    assert_eq!(check.code, 0, "{}", check.combined());
    assert_eq!(envelope["exit"].as_i64(), Some(i64::from(check.code)));
}

// An input error under `--json` surfaces as ok:false + exit:2 + `data.error`, with an
// empty `findings` array (input errors are not findings, §5.3), and the process exit
// code agrees. `check` with no lock is the representative input error.
#[test]
fn error_json_envelope_carries_data_error_not_findings() {
    let fixture = Fixture::acme();
    let check = fixture.run(&["check", "--json"]);

    let envelope = sole_json(&check.stdout);
    assert_envelope_keys(&envelope);
    assert_eq!(envelope["command"], "check");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["exit"], 2);
    assert_eq!(check.code, 2, "{}", check.combined());
    assert_eq!(envelope["findings"], Value::Array(vec![]));
    assert!(
        envelope["data"]["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("run stele build")),
        "{envelope}"
    );
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Load a config from a literal TOML body written into a temp `.stele/config.toml`.
fn load_str(body: &str) -> config::Config {
    load_result(body).expect("well-formed config")
}

fn load_str_err(body: &str) -> stele::model::SteleError {
    load_result(body).expect_err("expected an input error")
}

fn load_result(body: &str) -> stele::model::Result<config::Config> {
    let dir = tempfile::tempdir().expect("temp dir");
    let stele_dir = dir.path().join(".stele");
    std::fs::create_dir_all(&stele_dir).expect("create .stele");
    std::fs::write(stele_dir.join("config.toml"), body).expect("write config");
    config::load(dir.path())
}

/// Parse `stdout` as exactly one JSON object (the §5.3 "exactly one JSON object on
/// stdout" guarantee): the whole stream must be a single object with nothing trailing.
fn sole_json(stdout: &str) -> Value {
    let trimmed = stdout.trim_end_matches('\n');
    assert!(
        !trimmed.contains('\n'),
        "expected exactly one JSON object, got multiple lines: {stdout:?}"
    );
    let value: Value = serde_json::from_str(trimmed)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}): {stdout:?}"));
    assert!(value.is_object(), "envelope is not a JSON object: {value}");
    value
}

/// The six §5.3 envelope keys are all present.
fn assert_envelope_keys(envelope: &Value) {
    for key in ["stele", "command", "ok", "exit", "data", "findings"] {
        assert!(
            envelope.get(key).is_some(),
            "missing key {key:?}: {envelope}"
        );
    }
}
