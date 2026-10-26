//! The MCP tier — `stele serve` (SPEC §5.2), driven end-to-end against the real binary.

mod common;

use common::{BIN, Fixture, isolate_git};
use serde_json::{Value, json};
use std::fs::File;
use std::process::Command;

/// Feed `messages` to `stele serve` as newline-delimited stdin, closed for EOF shutdown
/// (§lifecycle), and parse stdout: one response per request, in order.
fn exchange(fixture: &Fixture, messages: &[Value]) -> Vec<Value> {
    let input = messages
        .iter()
        .map(|m| format!("{m}\n"))
        .collect::<String>();
    let out = fixture.run_with_stdin(&["serve"], input);
    assert_eq!(out.code, 0, "serve must exit 0 on EOF:\n{}", out.combined());
    responses(&out.stdout)
}

fn responses(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect(l))
        .inspect(|r: &Value| assert_eq!(r["jsonrpc"], "2.0", "{r}"))
        .collect()
}

fn init(id: usize, version: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"protocolVersion":version}})
}

fn call(id: usize, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params})
}

#[test]
fn handshake_negotiates_version_and_lists_eight_tools() {
    let responses = exchange(
        &Fixture::bare(),
        &[
            init(1, "2025-06-18"),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            init(3, "1.0.0"),
        ],
    );
    // A supported older version → echoed verbatim; an unsupported one → the latest.
    let result = &responses[0]["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert!(result["capabilities"]["tools"].is_object(), "{result}");
    assert_eq!(result["serverInfo"]["name"], "stele");

    // Exactly the eight read verbs — no mutating verb leaks in — each an object schema.
    let tools = responses[1]["result"]["tools"].as_array().expect("tools");
    let shape: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!([
                t["name"],
                t["inputSchema"]["type"],
                t["inputSchema"]["required"]
            ])
        })
        .collect();
    assert_eq!(
        json!(shape),
        json!([
            ["root", "object", null],
            ["node", "object", ["id"]],
            ["unfold", "object", ["id"]],
            ["invariants", "object", null],
            ["hazards", "object", null],
            ["nodes", "object", null],
            ["check", "object", null],
            ["blame", "object", ["claim"]],
        ])
    );
    assert!(
        tools
            .iter()
            .all(|t| t["description"].as_str().is_some_and(|d| !d.is_empty())
                && t["inputSchema"]["additionalProperties"] == false),
        "{tools:?}"
    );
    assert_eq!(responses[2]["result"]["protocolVersion"], "2025-11-25");
}

/// Serve text is byte-identical to CLI stdout (§5.2 shared render path); `check` appends
/// its exit status, and findings (Gallery 8.1: store imports billing) stay a normal result.
#[test]
fn tool_calls_match_cli_stdout() {
    let fixture = Fixture::acme();
    fixture.insert_line_at(
        "apps/web/lib/store/subscription.ex",
        9,
        "  alias AcmeWeb.Billing.Charge",
    );
    fixture.replace("AGENTS.md", "commands:\n", "commands:\n  flake: false\n");
    fixture.commit("store imports billing (undeclared edge); a failing command");
    assert_eq!(fixture.run(&["build"]).code, 0);

    let rows: [(&[&str], Value, &str); 9] = [
        (&["root"], json!({"name":"root"}), ""),
        (
            &["node", "billing"],
            json!({"name":"node","arguments":{"id":"billing"}}),
            "",
        ),
        (
            &["unfold", "apps/web", "--depth", "2"],
            json!({"name":"unfold","arguments":{"id":"apps/web","depth":2}}),
            "",
        ),
        (
            &["unfold", "apps/web"],
            json!({"name":"unfold","arguments":{"id":"apps/web","depth":"2"}}),
            "",
        ),
        (
            &["invariants", "--touching", "apps/worker"],
            json!({"name":"invariants","arguments":{"touching":"apps/worker"}}),
            "",
        ),
        (
            &["hazards", "--node", "billing"],
            json!({"name":"hazards","arguments":{"node":"billing"}}),
            "",
        ),
        (
            &["nodes", "--kind", "component"],
            json!({"name":"nodes","arguments":{"kind":"component"}}),
            "",
        ),
        (
            &["check", "--run-commands"],
            json!({"name":"check","arguments":{"run_commands":true}}),
            "exit: 1",
        ),
        (
            &["check", "--only", "liveness"],
            json!({"name":"check","arguments":{"only":"liveness","run_commands":false}}),
            "exit: 0",
        ),
    ];
    let messages: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(id, (_, params, _))| call(id, params.clone()))
        .collect();
    for ((argv, _, tail), response) in rows.iter().zip(exchange(&fixture, &messages)) {
        let text = fixture.run(argv).stdout + tail;
        assert_eq!(
            response["result"],
            json!({"content":[{"type":"text","text":text}],"isError":false}),
            "{argv:?}"
        );
    }
}

#[test]
fn malformed_and_unknown_inputs_are_resilient() {
    let responses = exchange(
        &Fixture::bare(),
        &[
            json!("not-an-object"),
            // A notification's method sent with an id is a request, and every request is answered.
            json!({"jsonrpc":"2.0","id":5,"method":"notifications/initialized"}),
            call(6, json!({"name":"nope"})),
            json!({"jsonrpc":"2.0","id":8}),
            json!({"jsonrpc":"2.0","id":9,"method":"tools/call"}),
            call(10, json!({})),
            // An ill-typed required argument is left out, so the verb's own usage line answers.
            call(11, json!({"name":"node","arguments":{"id":5}})),
            json!({"jsonrpc":"2.0","method":"notifications/unknown"}),
            json!({"jsonrpc":"2.0","id":7,"method":"ping"}),
        ],
    );
    let pairs: Vec<Value> = responses
        .iter()
        .map(|r| json!([r["id"], r["error"].get("code").unwrap_or(&r["result"])]))
        .collect();
    assert_eq!(
        json!(pairs),
        json!([
            [null, -32600],
            [5, -32601],
            [6, -32602],
            [8, -32600],
            [9, -32602],
            [10, -32602],
            [11, {"content":[{"type":"text","text":"usage: stele node <id>"}],"isError":true}],
            [7, {}],
        ])
    );
}

/// Undercover with no lock: the stderr note, a tool-error result (its line padded with
/// non-JSON whitespace), a skipped blank line, and a parse error for a non-JSON line and for a
/// non-UTF-8 one, which never ends the stream; only a stdio error does (exit 3: a directory
/// for stdin fails the read).
#[test]
fn raw_stream_survives_until_a_stdio_error() {
    let fixture = Fixture::bare();
    fixture.write(".stele/undercover", "stele undercover\n");
    let check = call(1, json!({"name":"check"}));
    let mut stdin = format!("\u{a0}{check}\u{b}\n \t\nnot json\n").into_bytes();
    stdin.extend(b"\xff\n");
    let out = fixture.run_with_stdin(&["serve"], stdin);
    assert_eq!(out.code, 0, "{}", out.combined());
    assert!(out.stderr.contains("undercover mode"), "{}", out.stderr);
    let responses = responses(&out.stdout);
    assert_eq!(responses.len(), 3, "{responses:?}");
    assert_eq!(responses[0]["result"]["isError"], true, "{}", responses[0]);
    assert!(
        responses[0]["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("run stele build") && !t.contains("exit:")),
        "{}",
        responses[0]
    );
    for response in &responses[1..] {
        assert_eq!(
            [&response["id"], &response["error"]["code"]],
            [&Value::Null, &json!(-32700)]
        );
    }
    let out = isolate_git(
        Command::new(BIN)
            .arg("serve")
            .current_dir(&fixture.root)
            .stdin(File::open(&fixture.root).unwrap()),
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("stele serve: stdio error"));
}

/// `--json` is a bad flag and a non-repo cwd is refused: exit 2, stdout (reserved for
/// protocol messages) left empty.
#[test]
fn cli_guards_refuse_before_serving() {
    let fixture = Fixture::bare();
    std::fs::remove_dir_all(fixture.path(".git")).expect("de-git the fixture");
    for (args, needle) in [
        (&["serve", "--json"][..], "--json"),
        (&["serve"], "not a git repository"),
    ] {
        let out = fixture.run(args);
        assert_eq!(out.code, 2, "{args:?}: {}", out.combined());
        assert!(out.stdout.is_empty(), "{args:?}: {:?}", out.stdout);
        assert!(out.stderr.contains(needle), "{args:?}: {}", out.stderr);
    }
}
