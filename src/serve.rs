//! MCP server — the Claude-first-class tier (SPEC §5.2).
//!
//! `stele serve` speaks the Model Context Protocol over a hand-rolled, blocking
//! JSON-RPC 2.0 loop on stdin/stdout (newline-delimited messages, MCP stdio transport,
//! protocol 2025-11-25). stdout carries ONLY protocol messages; stderr is free for
//! logging. Eight read tools mirror the §5.1 read/query verbs one-for-one, named
//! identically and un-prefixed — the server's own name `stele` supplies the namespace
//! (`stele.root`, `stele.node`, …). The mutating verbs `build`/`init`/`emit` are NOT
//! exposed (§5.2: they write source or the lock and belong to the shell/CI tier).
//!
//! Every tool call reuses the CLI's own verb dispatch through [`crate::cli::serve_render`],
//! so a tool's text content is byte-identical to `stele <verb>`'s stdout — one code path
//! per verb, no duplicated rendering. Each call loads the COMMITTED lock fresh (via the
//! verb's own read path); serve never rebuilds and never writes (§5.2/§5.3). A missing or
//! unknown-version lock is a per-call tool error result, not a startup failure.

use crate::cli::{self, VerbRender};
use crate::model::ExitCode;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::Path;

/// The MCP protocol version this server implements and defaults to when the client asks
/// for one it does not recognize (§version-negotiation: respond with the latest supported).
const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Protocol versions this server will echo back verbatim when a client requests one of
/// them (§version-negotiation: "if the server supports the requested version it MUST
/// respond with the same version"). The stdio + tools wire shape is stable across these;
/// serve declares no version-gated optional capability, so each is honestly supported.
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// The server name — also the tool namespace (§5.2): tools are un-prefixed here and
/// resolve to `stele.<verb>` on the wire.
const SERVER_NAME: &str = "stele";

/// The §5.3 exit class at or above which a verb result is an ERROR (input/internal),
/// mapped to an `isError:true` tool result rather than a normal one; below it (0 clean,
/// 1 assertion findings) is a normal tool result.
const ERROR_EXIT_FLOOR: i32 = 2;

// JSON-RPC 2.0 error codes (the standard set).
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// `stele serve` (§5.2): reject the meaningless `--json` flag, refuse cleanly outside a
/// git repo, then run the blocking MCP stdio loop until stdin EOF. Returns the process
/// exit code. serve owns stdout for protocol framing, so `run` is intercepted BEFORE the
/// CLI's `--json` envelope machinery (see [`crate::cli::run`]).
pub fn run(root: &Path, args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--json") {
        eprintln!(
            "stele serve: --json is not valid for serve — it speaks MCP JSON-RPC on stdout, \
             not the --json envelope"
        );
        return ExitCode::Input as i32;
    }
    // Refuse cleanly (exit 2) outside a git work tree: serve's tools scan VCS-tracked
    // files (§2.4), so a non-repo cwd can never answer a query.
    if let Err(error) = cli::ensure_git_repo(root) {
        eprintln!("{error}");
        return error.exit as i32;
    }
    serve_loop(root)
}

/// The blocking read/dispatch/write loop (MCP stdio transport): one JSON-RPC message per
/// line in, one response line per request out, notifications answered with nothing. A
/// malformed line becomes a parse-error response and the loop continues (never a crash);
/// stdin EOF is graceful shutdown (§lifecycle, exit 0).
fn serve_loop(root: &Path) -> i32 {
    eprintln!("stele serve: MCP stdio server ready (protocol {LATEST_PROTOCOL_VERSION})");
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return 0,
            Ok(_) => {}
            Err(e) => {
                eprintln!("stele serve: stdin read error: {e}");
                return ExitCode::Internal as i32;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = handle_message(root, trimmed) {
            // Compact serialization has no embedded newlines (transport requirement); the
            // trailing `\n` is the message delimiter.
            let serialized = serde_json::to_string(&response)
                .unwrap_or_else(|_| String::from(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#));
            if writeln!(stdout, "{serialized}").is_err() || stdout.flush().is_err() {
                return ExitCode::Internal as i32;
            }
        }
    }
}

/// Parse and dispatch one JSON-RPC message. Returns the response `Value` for a request,
/// or `None` for a notification (no `id`) and for messages that warrant no reply. A
/// non-JSON line → parse error; a non-object or method-less request → invalid request; an
/// unknown method on a request → method-not-found (an unknown notification is ignored).
fn handle_message(root: &Path, text: &str) -> Option<Value> {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Some(error_response(Value::Null, PARSE_ERROR, "Parse error")),
    };
    let Some(obj) = value.as_object() else {
        return Some(error_response(
            Value::Null,
            INVALID_REQUEST,
            "Invalid Request: expected a JSON-RPC object",
        ));
    };
    // Absent `id` marks a notification (JSON-RPC): no response, even on error.
    let id = obj.get("id").cloned();
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return id.map(|id| error_response(id, INVALID_REQUEST, "Invalid Request: missing method"));
    };
    match method {
        "initialize" => id.map(|id| initialize_result(id, obj.get("params"))),
        "notifications/initialized" => None,
        "ping" => id.map(|id| success(id, json!({}))),
        "tools/list" => id.map(|id| success(id, tools_list())),
        "tools/call" => id.map(|id| tools_call(root, id, obj.get("params"))),
        other => {
            id.map(|id| error_response(id, METHOD_NOT_FOUND, &format!("Method not found: {other}")))
        }
    }
}

/// The `initialize` result (§lifecycle): negotiate the protocol version — echo the
/// client's when supported, else advertise the latest — and declare the `tools`
/// capability plus server identity. Tools are static, so `listChanged` is not declared.
fn initialize_result(id: Value, params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = match requested {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v,
        _ => LATEST_PROTOCOL_VERSION,
    };
    success(
        id,
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": "stele MCP server (SPEC §5.2): eight read-only query tools over the \
                committed graph lock. Mutating verbs (build/init/emit) are shell/CI-only and not \
                exposed here.",
        }),
    )
}

/// The `tools/list` result (§server/tools): the eight read verbs, each with a one-line
/// description drawn from SPEC §5.1 and a valid JSON Schema `inputSchema`. No pagination
/// cursor — the whole set fits one page.
fn tools_list() -> Value {
    json!({ "tools": tool_definitions() })
}

/// Every exposed tool definition (§5.2): `{name, description, inputSchema}`. Names are
/// un-prefixed (the server namespace supplies `stele.`); schemas are 2020-12 JSON Schema
/// objects with `additionalProperties:false` so only the declared params are accepted.
fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "root",
            "Render the initialContext (§6) as text: identity, commands, hazard banner, router, index pointers.",
            json!({ "type": "object", "additionalProperties": false }),
        ),
        tool(
            "node",
            "One node, all fields (id accepts a final-segment abbreviation when unambiguous).",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Node id or unambiguous abbreviation." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        ),
        tool(
            "unfold",
            "A node plus its one-hop edge summaries (id, kind, purpose), expanded out to `depth` hops.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Node id or unambiguous abbreviation." },
                    "depth": { "type": "integer", "minimum": 1, "description": "Hop radius (default 1)." },
                },
                "required": ["id"],
                "additionalProperties": false,
            }),
        ),
        tool(
            "invariants",
            "Every invariant repo-wide, or — with `touching` — the owning node's invariants plus its ancestors' (upward exposure).",
            json!({
                "type": "object",
                "properties": { "touching": { "type": "string", "description": "A repo-relative path; scopes to its owning node chain." } },
                "additionalProperties": false,
            }),
        ),
        tool(
            "hazards",
            "Every active hazard repo-wide, or just the hazards declared by one node.",
            json!({
                "type": "object",
                "properties": { "node": { "type": "string", "description": "Node id or unambiguous abbreviation." } },
                "additionalProperties": false,
            }),
        ),
        tool(
            "nodes",
            "Every node as id, kind, purpose, optionally filtered to a single kind.",
            json!({
                "type": "object",
                "properties": { "kind": { "type": "string", "description": "One of system | container | component | adr | anchor." } },
                "additionalProperties": false,
            }),
        ),
        tool(
            "check",
            "Run the six assertion classes over the committed graph; result is the findings text with a trailing `exit:` status line.",
            json!({
                "type": "object",
                "properties": {
                    "only": {
                        "type": "string",
                        "enum": ["referential", "structural", "exhaustiveness", "budget", "freshness", "liveness"],
                        "description": "Run exactly one assertion class.",
                    },
                    "run_commands": { "type": "boolean", "description": "Also execute each declared command (liveness, §4.6)." },
                },
                "additionalProperties": false,
            }),
        ),
        tool(
            "blame",
            "Walk history to the commit that staled a claim, addressed `<node-id>/<slug>` (§4.5).",
            json!({
                "type": "object",
                "properties": { "claim": { "type": "string", "description": "Claim address `<node-id>/<slug>` (node-id may be abbreviated)." } },
                "required": ["claim"],
                "additionalProperties": false,
            }),
        ),
    ]
}

/// One tool definition object.
fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

/// Handle `tools/call` (§server/tools): resolve the tool name to a CLI argv, dispatch it
/// through the shared verb renderer, and package the result. An unknown tool name or
/// missing params is a JSON-RPC protocol error (§Error Handling: unknown tools are
/// protocol errors); a verb-level input/internal error (exit ≥ 2, e.g. a missing lock)
/// is an `isError:true` tool result; clean output and assertion findings (exit 0/1) are
/// normal tool results.
fn tools_call(root: &Path, id: Value, params: Option<&Value>) -> Value {
    let Some(params) = params else {
        return error_response(
            id,
            INVALID_PARAMS,
            "Invalid params: tools/call requires params",
        );
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(
            id,
            INVALID_PARAMS,
            "Invalid params: tools/call requires a tool name",
        );
    };
    let Some(argv) = argv_for_tool(name, params.get("arguments")) else {
        return error_response(id, INVALID_PARAMS, &format!("Unknown tool: {name}"));
    };

    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let VerbRender { exit, text } = cli::serve_render(root, &argv_refs);
    if exit >= ERROR_EXIT_FLOOR {
        // Missing/unknown-version lock and other input/internal errors surface as a
        // tool-level error result carrying the CLI's message (§5.2).
        return success(id, tool_result(&text, true));
    }
    // The check verb prints nothing on a clean run and no status on findings; append the
    // §5.3 exit code so an agent always sees the outcome (0 clean, 1 findings).
    let content = if name == "check" {
        let mut text = text;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("exit: {exit}"));
        text
    } else {
        text
    };
    success(id, tool_result(&content, false))
}

/// Build the CLI argv for a tool call from its named arguments (§5.2: CLI flags map to
/// named tool parameters). An absent optional arg is simply omitted; a missing REQUIRED
/// arg is left out too, so the verb's own usage check produces the exit-2 error result
/// (one code path). Returns `None` only for an unknown tool name.
fn argv_for_tool(name: &str, arguments: Option<&Value>) -> Option<Vec<String>> {
    let string_arg = |key: &str| {
        arguments
            .and_then(|a| a.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let mut argv = vec![name.to_string()];
    match name {
        "root" => {}
        "node" => argv.extend(string_arg("id")),
        "unfold" => {
            argv.extend(string_arg("id"));
            if let Some(depth) = arguments
                .and_then(|a| a.get("depth"))
                .and_then(Value::as_i64)
            {
                argv.push("--depth".to_string());
                argv.push(depth.to_string());
            }
        }
        "invariants" => {
            if let Some(path) = string_arg("touching") {
                argv.push("--touching".to_string());
                argv.push(path);
            }
        }
        "hazards" => {
            if let Some(node) = string_arg("node") {
                argv.push("--node".to_string());
                argv.push(node);
            }
        }
        "nodes" => {
            if let Some(kind) = string_arg("kind") {
                argv.push("--kind".to_string());
                argv.push(kind);
            }
        }
        "check" => {
            if let Some(only) = string_arg("only") {
                argv.push("--only".to_string());
                argv.push(only);
            }
            if arguments
                .and_then(|a| a.get("run_commands"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                argv.push("--run-commands".to_string());
            }
        }
        "blame" => argv.extend(string_arg("claim")),
        _ => return None,
    }
    Some(argv)
}

/// A JSON-RPC 2.0 success response.
fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC 2.0 error response.
fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// An MCP tool result (§server/tools): unstructured text content plus the `isError` flag.
fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}
