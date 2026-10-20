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
//! so a tool's text content is byte-identical to `stele <verb>`'s stdout (an error result
//! carries the message the CLI prints to stderr; `check` appends `exit: N`) — one code path
//! per verb, no duplicated rendering. Each call loads the COMMITTED lock fresh (via the
//! verb's own read path) and never writes the lock or a source (§5.2/§5.3); `check` still
//! rebuilds in memory and, with `run_commands`, runs the declared commands. A missing or
//! unknown-version lock is a per-call tool error result, not a startup failure.

use crate::assert::ALL_CLASSES;
use crate::cli::{self, Mode, VerbRender, Workspace};
use crate::config::AssertionClass;
use crate::model::ExitCode;
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;

/// Protocol versions this server will echo back verbatim when a client requests one of
/// them (§version-negotiation: "if the server supports the requested version it MUST
/// respond with the same version"). The stdio + tools wire shape is shared across these,
/// save one gap: 2025-03-26 requires receiving JSON-RPC batches, and serve answers an array
/// with invalid-request (fetched 2026-10-20 · modelcontextprotocol.io 2025-03-26/basic).
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

/// A request's outcome before its envelope: the `result`, or an error's `(code, message)`.
type Outcome = Result<Value, (i64, String)>;

/// `stele serve` (§5.2): the blocking MCP stdio loop. stdin EOF is graceful shutdown
/// (§lifecycle); a stdio error exits internal.
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
    eprintln!("stele serve: MCP stdio server ready (protocol {LATEST_PROTOCOL_VERSION})");
    let mut stdout = std::io::stdout();
    for line in std::io::stdin().lines() {
        // Compact serialization has no embedded newlines (transport requirement); the
        // trailing `\n` is the message delimiter.
        let written = line.and_then(|line| {
            handle_message(&workspace, line.trim()).map_or(Ok(()), |r| writeln!(stdout, "{r}"))
        });
        if let Err(e) = written {
            eprintln!("stele serve: stdio error: {e}");
            return ExitCode::Internal as i32;
        }
    }
    0
}

/// Parse and dispatch one JSON-RPC line: the response for a request, `None` when no reply
/// is owed (blank line, notification). Malformed input is an error response, never a crash.
fn handle_message(ws: &Workspace, text: &str) -> Option<Value> {
    let (id, outcome): (Value, Outcome) = match serde_json::from_str::<Value>(text) {
        Err(_) if text.is_empty() => return None,
        Err(_) => (Value::Null, Err((PARSE_ERROR, "Parse error".into()))),
        Ok(value) if !value.is_object() => (
            Value::Null,
            Err((
                INVALID_REQUEST,
                "Invalid Request: expected a JSON-RPC object".into(),
            )),
        ),
        // Absent `id`: treated as a notification and never answered, even on error.
        Ok(msg) => (
            msg.get("id")?.clone(),
            match msg["method"].as_str() {
                None => Err((INVALID_REQUEST, "Invalid Request: missing method".into())),
                Some("initialize") => Ok(initialize_result(msg.get("params"))),
                Some("ping") => Ok(json!({})),
                Some("tools/list") => Ok(json!({ "tools": tool_definitions() })),
                Some("tools/call") => tools_call(ws, msg.get("params")),
                Some(other) => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
            },
        ),
    };
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

/// The `initialize` result (§lifecycle): negotiate the protocol version — echo the
/// client's when supported, else advertise the latest — and declare the `tools`
/// capability plus server identity. Tools are static, so `listChanged` is not declared.
fn initialize_result(params: Option<&Value>) -> Value {
    let version = params
        .and_then(|p| p["protocolVersion"].as_str())
        .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        // The server name is also the tool namespace (§5.2): bare verbs on the wire, which a
        // client qualifies as `stele.<verb>`.
        "serverInfo": { "name": "stele", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "stele MCP server (SPEC §5.2): eight read-only query tools over the \
            committed graph lock. Mutating verbs (build/init/emit) are shell/CI-only and not \
            exposed here.",
    })
}

/// Every exposed tool definition (§5.2), also the source [`argv_for_tool`] derives argv
/// from. Schemas are 2020-12 JSON Schema objects with `additionalProperties:false`
/// (undeclared args are never forwarded). The whole set fits one `tools/list` page — no
/// pagination cursor.
fn tool_definitions() -> Value {
    json!([
        {
            "name": "root",
            "description": "Render the initialContext (§6) as text: identity, commands, hazard banner, router, index pointers.",
            "inputSchema": { "type": "object", "additionalProperties": false },
        },
        {
            "name": "node",
            "description": "One node, all fields (id accepts a final-segment abbreviation when unambiguous).",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Node id or unambiguous abbreviation." } },
                "required": ["id"],
                "additionalProperties": false,
            },
        },
        {
            "name": "unfold",
            "description": "A node plus its one-hop edge summaries (id, kind, purpose), expanded out to `depth` hops.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Node id or unambiguous abbreviation." },
                    "depth": { "type": "integer", "minimum": 1, "description": "Hop radius (default 1)." },
                },
                "required": ["id"],
                "additionalProperties": false,
            },
        },
        {
            "name": "invariants",
            "description": "Every invariant repo-wide, or — with `touching` — the owning node's invariants plus its ancestors' (upward exposure).",
            "inputSchema": {
                "type": "object",
                "properties": { "touching": { "type": "string", "description": "A repo-relative path; scopes to its owning node chain." } },
                "additionalProperties": false,
            },
        },
        {
            "name": "hazards",
            "description": "Every active hazard repo-wide, or just the hazards declared by one node.",
            "inputSchema": {
                "type": "object",
                "properties": { "node": { "type": "string", "description": "Node id or unambiguous abbreviation." } },
                "additionalProperties": false,
            },
        },
        {
            "name": "nodes",
            "description": "Every node as id, kind, purpose, optionally filtered to a single kind.",
            "inputSchema": {
                "type": "object",
                "properties": { "kind": { "type": "string", "description": "One of system | container | component | adr | anchor." } },
                "additionalProperties": false,
            },
        },
        {
            "name": "check",
            "description": "Rebuild the graph, compare it to the committed lock, and run the six assertion classes; result is the findings text with a trailing `exit:` status line (an error result has none).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "only": {
                        "type": "string",
                        "enum": ALL_CLASSES.map(AssertionClass::as_str),
                        "description": "Run exactly one assertion class.",
                    },
                    "run_commands": { "type": "boolean", "description": "Also execute each declared command (liveness, §4.6)." },
                },
                "additionalProperties": false,
            },
        },
        {
            "name": "blame",
            "description": "Walk history to the commit that staled a claim, addressed `<node-id>/<slug>` (§4.5).",
            "inputSchema": {
                "type": "object",
                "properties": { "claim": { "type": "string", "description": "Claim address `<node-id>/<slug>` (node-id may be abbreviated)." } },
                "required": ["claim"],
                "additionalProperties": false,
            },
        },
    ])
}

/// Handle `tools/call` (§server/tools). Bad params or an unknown tool is a JSON-RPC
/// protocol error (§Error Handling); a verb failure is an `isError` result.
fn tools_call(ws: &Workspace, params: Option<&Value>) -> Outcome {
    let params = params.ok_or((
        INVALID_PARAMS,
        "Invalid params: tools/call requires params".into(),
    ))?;
    let name = params["name"].as_str().ok_or((
        INVALID_PARAMS,
        "Invalid params: tools/call requires a tool name".into(),
    ))?;
    let argv = argv_for_tool(name, params.get("arguments"))
        .ok_or_else(|| (INVALID_PARAMS, format!("Unknown tool: {name}")))?;
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let VerbRender { exit, mut text } = cli::serve_render(ws, &argv_refs);
    // §5.3 input/internal errors (e.g. a missing lock) are `isError` results carrying the
    // CLI's message (§5.2); check otherwise gets its exit appended — clean runs print nothing.
    let is_error = exit >= ExitCode::Input as i32;
    if name == "check" && !is_error {
        text.push_str(&format!("exit: {exit}"));
    }
    Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

/// The CLI argv for a tool call, derived from its `inputSchema` (§5.2: CLI flags map to
/// named tool parameters): required string args as positionals, then each optional one as
/// its `--flag`. Absent or ill-typed args are omitted, leaving a missing required arg to
/// the verb's own usage check. `None` only for an unknown tool name.
fn argv_for_tool(name: &str, arguments: Option<&Value>) -> Option<Vec<String>> {
    let tools = tool_definitions();
    let schema = &tools.as_array()?.iter().find(|t| t["name"] == name)?["inputSchema"];
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
