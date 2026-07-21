//! The MCP tier — `stele serve` (SPEC §5.2), driven end-to-end against the real binary.
//! Each probe spawns `stele serve` on a scratch acme copy, feeds a batch of
//! newline-delimited JSON-RPC messages on stdin (closed to signal EOF), and parses the
//! newline-delimited responses off stdout. The server reads to EOF and exits (§lifecycle
//! shutdown), so the captured stdout holds one response per request, in order.
//!
//! Coverage: the full initialize/initialized handshake with version negotiation, the
//! eight-tool `tools/list` snapshot with valid schemas, per-tool equivalence against the
//! CLI's own stdout (the shared render path, §5.2), a findings-carrying `check` result,
//! the missing-lock tool error, malformed-input resilience, unknown-method/tool errors,
//! and clean EOF shutdown, plus the two CLI-side guards (`--json`, non-repo cwd).

mod common;

use common::Fixture;
use serde_json::{Value, json};

/// The eight read verbs `serve` exposes (§5.2), un-prefixed within the `stele` namespace.
/// Mutating verbs (build/init/emit) are deliberately absent.
const EXPECTED_TOOLS: [&str; 8] = [
    "blame",
    "check",
    "hazards",
    "invariants",
    "node",
    "nodes",
    "root",
    "unfold",
];

/// The MCP protocol version this server implements (server/tools + lifecycle, 2025-11-25).
const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Build the committed lock on a fresh acme copy — the precondition the read tools need
/// (§5.3: a missing lock is a per-call tool error, tested separately).
fn built() -> Fixture {
    let fixture = Fixture::acme();
    let build = fixture.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.combined());
    fixture
}

/// Feed `messages` to `stele serve` as newline-delimited JSON on stdin, then parse the
/// response lines. Asserts a clean exit (0) — EOF shutdown must always succeed.
fn exchange(fixture: &Fixture, messages: &[Value]) -> Vec<Value> {
    let mut input: String = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    input.push('\n');
    let out = fixture.run_with_stdin(&["serve"], &input);
    assert_eq!(out.code, 0, "serve must exit 0 on EOF:\n{}", out.combined());
    out.stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad response {l:?}: {e}")))
        .collect()
}

/// The text content of a successful `tools/call` result.
fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result text")
}

// ── the handshake: initialize → initialized → tools/list ─────────────────────

#[test]
fn handshake_negotiates_version_and_lists_eight_tools() {
    let fixture = built();
    let responses = exchange(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion": LATEST_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name":"probe","version":"0"}
            }}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ],
    );

    // The notification produced no response: two requests → two responses.
    assert_eq!(responses.len(), 2, "{responses:?}");

    // initialize: version echoed, tools capability declared, server identity `stele`.
    let init = &responses[0];
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], LATEST_PROTOCOL_VERSION);
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "tools capability must be declared:\n{init}"
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "stele");

    // tools/list: exactly the eight read verbs, each with a valid JSON Schema.
    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools array");
    let mut names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("tool name"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, EXPECTED_TOOLS, "the eight exposed verbs");
    // No mutating verb leaks in.
    for forbidden in ["build", "init", "emit", "serve"] {
        assert!(
            !names.contains(&forbidden),
            "{forbidden} must not be exposed"
        );
    }

    for tool in tools {
        let schema = &tool["inputSchema"];
        assert_eq!(
            schema["type"], "object",
            "inputSchema must be an object schema:\n{tool}"
        );
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "each tool carries a one-line description:\n{tool}"
        );
    }

    // Required-parameter shape: node/unfold/blame each require their positional arg.
    let by_name = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap().clone();
    assert_eq!(by_name("node")["inputSchema"]["required"], json!(["id"]));
    assert_eq!(by_name("unfold")["inputSchema"]["required"], json!(["id"]));
    assert_eq!(
        by_name("blame")["inputSchema"]["required"],
        json!(["claim"])
    );
}

#[test]
fn version_negotiation_echoes_supported_else_latest() {
    let fixture = built();
    let responses = exchange(
        &fixture,
        &[
            // An unsupported version → the server advertises its latest.
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1.0.0"}}),
            // A supported older version → echoed verbatim.
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        ],
    );
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        LATEST_PROTOCOL_VERSION
    );
    assert_eq!(responses[1]["result"]["protocolVersion"], "2025-06-18");
}

// ── tool-call equivalence: serve text == CLI stdout (§5.2 shared render path) ─

#[test]
fn tool_calls_match_cli_stdout() {
    let fixture = built();

    // (verb argv, tool name, tool arguments) — each pair must render identically.
    let cases: [(&[&str], Value); 4] = [
        (&["root"], json!({"name":"root","arguments":{}})),
        (
            &["node", "billing"],
            json!({"name":"node","arguments":{"id":"billing"}}),
        ),
        (
            &["unfold", "apps/web", "--depth", "2"],
            json!({"name":"unfold","arguments":{"id":"apps/web","depth":2}}),
        ),
        (
            &["invariants", "--touching", "apps/web/lib/billing"],
            json!({"name":"invariants","arguments":{"touching":"apps/web/lib/billing"}}),
        ),
    ];

    for (argv, call_params) in cases {
        let cli = fixture.run(argv);
        assert_eq!(cli.code, 0, "{}", cli.combined());
        let responses = exchange(
            &fixture,
            &[json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":call_params})],
        );
        assert_eq!(
            responses[0]["result"]["isError"], false,
            "{argv:?} tool result must not be an error:\n{}",
            responses[0]
        );
        assert_eq!(
            tool_text(&responses[0]),
            cli.stdout,
            "{argv:?}: tool text must be byte-identical to CLI stdout"
        );
    }
}

// ── the check tool: findings + exit status line, not an error result ─────────

#[test]
fn check_tool_carries_findings_and_exit_line() {
    // Gallery 8.1: an undeclared cross-boundary import — store imports billing. The tree
    // still builds; `check` then reports a structural violation (exit 1).
    let fixture = Fixture::acme();
    fixture.insert_line_at(
        "apps/web/lib/store/subscription.ex",
        9,
        "  alias AcmeWeb.Billing.Charge",
    );
    fixture.commit("store imports billing (undeclared edge)");
    assert_eq!(fixture.run(&["build"]).code, 0);

    let responses = exchange(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check","arguments":{}}}),
        ],
    );
    let result = &responses[0]["result"];
    // Findings are a NORMAL result (isError:false) carrying the findings text (§5.2).
    assert_eq!(result["isError"], false, "{result}");
    let text = tool_text(&responses[0]);
    assert!(text.contains("structural"), "findings text:\n{text}");
    assert!(
        text.trim_end().ends_with("exit: 1"),
        "check result must end with the exit status:\n{text}"
    );
}

#[test]
fn check_tool_clean_repo_reports_exit_zero() {
    let fixture = built();
    let responses = exchange(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check","arguments":{}}}),
        ],
    );
    assert_eq!(responses[0]["result"]["isError"], false);
    assert!(
        tool_text(&responses[0]).trim_end().ends_with("exit: 0"),
        "{}",
        tool_text(&responses[0])
    );
}

// ── error surfaces: missing lock, malformed input, unknown method/tool ───────

#[test]
fn missing_lock_is_a_tool_error_result() {
    // No `build` → no committed lock; the tool call is a tool-level error (§5.2), not a
    // protocol error, carrying the run-stele-build message.
    let fixture = Fixture::acme();
    let responses = exchange(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"root","arguments":{}}}),
        ],
    );
    let result = &responses[0]["result"];
    assert_eq!(result["isError"], true, "{result}");
    assert!(
        tool_text(&responses[0]).contains("run stele build"),
        "{result}"
    );
}

#[test]
fn malformed_and_unknown_inputs_are_resilient() {
    let fixture = built();
    let responses = exchange(
        &fixture,
        &[
            // Valid JSON but not a JSON-RPC object (a bare string) → invalid request.
            json!("not-an-object"),
            json!({"jsonrpc":"2.0","id":5,"method":"bogus/method"}),
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope"}}),
            // A notification with an unknown method → ignored, no response.
            json!({"jsonrpc":"2.0","method":"notifications/unknown"}),
            // The server is still alive and answering afterwards.
            json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}),
        ],
    );
    // Four requests answered, the unknown notification ignored.
    assert_eq!(responses.len(), 4, "{responses:?}");

    // The bare-string line: invalid request (id null), no crash.
    assert_eq!(responses[0]["error"]["code"], -32600, "{}", responses[0]);
    assert!(responses[0]["id"].is_null());
    // Unknown method → method-not-found.
    assert_eq!(responses[1]["id"], 5);
    assert_eq!(responses[1]["error"]["code"], -32601, "{}", responses[1]);
    // Unknown tool name → invalid params (a protocol error, §Error Handling).
    assert_eq!(responses[2]["id"], 6);
    assert_eq!(responses[2]["error"]["code"], -32602, "{}", responses[2]);
    // Still serving: the trailing tools/list returns the eight tools.
    assert_eq!(responses[3]["id"], 7);
    assert_eq!(
        responses[3]["result"]["tools"].as_array().map(Vec::len),
        Some(8)
    );
}

#[test]
fn truly_malformed_line_yields_parse_error() {
    // A line that is not valid JSON at all → JSON-RPC parse error (-32700), id null.
    let fixture = built();
    let out = fixture.run_with_stdin(&["serve"], "this is not json\n");
    assert_eq!(out.code, 0, "{}", out.combined());
    let response: Value = serde_json::from_str(out.stdout.lines().next().expect("a response"))
        .expect("parse error response");
    assert_eq!(response["error"]["code"], -32700, "{response}");
    assert!(response["id"].is_null(), "{response}");
}

#[test]
fn eof_shuts_down_cleanly() {
    // Just an initialize, then stdin closes → the server responds and exits 0.
    let fixture = built();
    let out = fixture.run_with_stdin(
        &["serve"],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
    );
    assert_eq!(out.code, 0, "{}", out.combined());
    assert!(out.stdout.contains("\"protocolVersion\""), "{}", out.stdout);
}

// ── the CLI-side guards ──────────────────────────────────────────────────────

#[test]
fn serve_rejects_json_flag() {
    let fixture = built();
    // `--json` is meaningless for serve; it is rejected as a bad flag (exit 2), and
    // nothing is written to stdout (which is reserved for protocol messages).
    let out = fixture.run_with_stdin(&["serve", "--json"], "");
    assert_eq!(out.code, 2, "{}", out.combined());
    assert!(
        out.stdout.is_empty(),
        "stdout must stay clean: {:?}",
        out.stdout
    );
    assert!(out.stderr.contains("--json"), "{}", out.stderr);
}

#[test]
fn serve_refuses_outside_a_git_repo() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_stele"))
        .arg("serve")
        .current_dir(dir.path())
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn stele serve");
    assert_eq!(
        out.status.code(),
        Some(2),
        "must refuse cleanly outside a repo"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not a git repository"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
