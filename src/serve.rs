//! MCP server — the Claude-first-class tier (SPEC §5.2).
//!
//! `stele serve` speaks the Model Context Protocol over a hand-rolled, blocking
//! JSON-RPC 2.0 loop on stdin/stdout (newline-delimited messages, MCP stdio transport,
//! protocol 2025-11-25). stdout carries ONLY protocol messages; stderr is free for
//! logging. Eight read tools mirror the §5.1 read/query verbs one-for-one, named
//! identically and un-prefixed — the server's own name `stele` supplies the namespace
//! (`stele.root`, `stele.node`, …). The mutating verbs `build`/`init`/`emit`/`migrate`
//! are NOT exposed (§5.2: they write source or the lock and belong to the shell/CI tier).
//!
//! Every tool call reuses the CLI's own verb dispatch through [`crate::cli::serve_render`],
//! so a tool's text content is byte-identical to `stele <verb>`'s stdout — one code path
//! per verb, no duplicated rendering. Each call loads the COMMITTED lock fresh (via the
//! verb's own read path); serve never rebuilds and never writes (§5.2/§5.3). A missing or
//! unknown-version lock is a per-call tool error result, not a startup failure.

use crate::cli::{self, Mode, VerbRender, Workspace};
use crate::model::ExitCode;
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;

/// Protocol versions this server will echo back verbatim when a client requests one of
/// them (§version-negotiation: "if the server supports the requested version it MUST
/// respond with the same version"). The stdio + tools wire shape is stable across these;
/// serve declares no version-gated optional capability, so each is honestly supported.
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// The version this server implements and answers an unrecognized request with
/// (§version-negotiation: respond with the latest supported).
const LATEST_PROTOCOL_VERSION: &str = SUPPORTED_PROTOCOL_VERSIONS[3];

// JSON-RPC 2.0 error codes (the standard set).
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// `stele serve` (§5.2): runs the blocking MCP stdio loop until stdin EOF. Intercepted
/// BEFORE the CLI's `--json` envelope machinery (see [`crate::cli::run`]) — serve owns stdout.
pub fn run(root: &Path, args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--json") {
        eprintln!(
            "stele serve: --json is not valid for serve — it speaks MCP JSON-RPC on stdout, \
             not the --json envelope"
        );
        return ExitCode::Input as i32;
    }
    // Queries read the graph home (§3.5) but scan the work tree's VCS-tracked files (§2.4),
    // so a non-repo cwd can never answer one.
    let workspace = cli::resolve_workspace(root);
    if let Err(error) = cli::ensure_git_repo(&workspace.work) {
        eprintln!("{error}");
        return error.exit as i32;
    }
    if workspace.mode == Mode::Undercover {
        eprintln!(
            "stele serve: undercover mode — reading the private graph at {}",
            workspace.home.display()
        );
    }
    match serve_loop(&workspace) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("stele serve: stdio error: {e}");
            ExitCode::Internal as i32
        }
    }
}

/// The blocking read/dispatch/write loop (MCP stdio transport): one JSON-RPC message per
/// line in, one response line per request out, notifications answered with nothing. A
/// malformed line becomes a parse-error response and the loop continues (never a crash);
/// stdin EOF is graceful shutdown (§lifecycle).
fn serve_loop(ws: &Workspace) -> std::io::Result<()> {
    eprintln!("stele serve: MCP stdio server ready (protocol {LATEST_PROTOCOL_VERSION})");
    let mut stdout = std::io::stdout();
    for line in std::io::stdin().lines() {
        let line = line?;
        let text = line.trim();
        if !text.is_empty()
            && let Some(response) = handle_message(ws, text)
        {
            // Compact serialization has no embedded newlines (transport requirement); the
            // trailing `\n` is the message delimiter.
            writeln!(stdout, "{response}")?;
        }
    }
    Ok(())
}

/// Parse and dispatch one JSON-RPC message. Returns the response `Value` for a request,
/// or `None` for a notification (no `id`) and for messages that warrant no reply. A
/// non-JSON line → parse error; a non-object or method-less request → invalid request; an
/// unknown method on a request → method-not-found (an unknown notification is ignored).
fn handle_message(ws: &Workspace, text: &str) -> Option<Value> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Some(error_response(Value::Null, PARSE_ERROR, "Parse error"));
    };
    let Some(obj) = value.as_object() else {
        return Some(error_response(
            Value::Null,
            INVALID_REQUEST,
            "Invalid Request: expected a JSON-RPC object",
        ));
    };
    // Absent `id` marks a notification (JSON-RPC): no response, even on error.
    let id = obj.get("id").cloned()?;
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Some(error_response(
            id,
            INVALID_REQUEST,
            "Invalid Request: missing method",
        ));
    };
    Some(match method {
        "initialize" => initialize_result(id, obj.get("params")),
        "notifications/initialized" => return None,
        "ping" => success(id, json!({})),
        "tools/list" => success(id, json!({ "tools": tool_definitions() })),
        "tools/call" => tools_call(ws, id, obj.get("params")),
        other => error_response(id, METHOD_NOT_FOUND, &format!("Method not found: {other}")),
    })
}

/// The `initialize` result (§lifecycle): negotiate the protocol version — echo the
/// client's when supported, else advertise the latest — and declare the `tools`
/// capability plus server identity. Tools are static, so `listChanged` is not declared.
fn initialize_result(id: Value, params: Option<&Value>) -> Value {
    let version = params
        .and_then(|p| p["protocolVersion"].as_str())
        .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    success(
        id,
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            // The server name is also the tool namespace (§5.2): `stele.<verb>` on the wire.
            "serverInfo": { "name": "stele", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "stele MCP server (SPEC §5.2): eight read-only query tools over the \
                committed graph lock. Mutating verbs (build/init/emit) are shell/CI-only and not \
                exposed here.",
        }),
    )
}

/// Every exposed tool definition (§5.2): `{name, description, inputSchema}`. Names are
/// un-prefixed (the server namespace supplies `stele.`); schemas are 2020-12 JSON Schema
/// objects with `additionalProperties:false` so only the declared params are accepted. The
/// whole set fits one `tools/list` page — no pagination cursor.
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
/// protocol errors).
fn tools_call(ws: &Workspace, id: Value, params: Option<&Value>) -> Value {
    let Some(params) = params else {
        return error_response(
            id,
            INVALID_PARAMS,
            "Invalid params: tools/call requires params",
        );
    };
    let Some(name) = params["name"].as_str() else {
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
    let VerbRender { exit, mut text } = cli::serve_render(ws, &argv_refs);
    // §5.3 input/internal errors (e.g. a missing lock) are `isError` results carrying the
    // CLI's message (§5.2); check otherwise gets its exit appended — clean runs print nothing.
    let is_error = exit >= ExitCode::Input as i32;
    if name == "check" && !is_error {
        text.push_str(&format!("exit: {exit}"));
    }
    success(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
    )
}

/// The CLI argv for a tool call, derived from its `inputSchema` (§5.2: CLI flags map to
/// named tool parameters): required string args as positionals, then each optional one as
/// its `--flag`. Absent or ill-typed args are omitted, leaving a missing required arg to
/// the verb's own usage check. `None` only for an unknown tool name.
fn argv_for_tool(name: &str, arguments: Option<&Value>) -> Option<Vec<String>> {
    let tool = tool_definitions().into_iter().find(|t| t["name"] == name)?;
    let schema = &tool["inputSchema"];
    let required = schema["required"].as_array().cloned().unwrap_or_default();
    let arg = |key: &str| arguments.and_then(|a| a.get(key));
    let mut argv = vec![name.to_string()];
    argv.extend(
        required
            .iter()
            .filter_map(|k| Some(arg(k.as_str()?)?.as_str()?.to_string())),
    );
    let optional = schema["properties"].as_object().into_iter().flatten();
    for (key, prop) in optional.filter(|(k, _)| !required.contains(&json!(k))) {
        let flag = format!("--{}", key.replace('_', "-"));
        match (prop["type"].as_str(), arg(key)) {
            (Some("string"), Some(Value::String(s))) => argv.extend([flag, s.clone()]),
            (Some("integer"), Some(v)) if v.is_i64() => argv.extend([flag, v.to_string()]),
            (Some("boolean"), Some(Value::Bool(true))) => argv.push(flag),
            _ => {}
        }
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
