//! Lock-file data model and canonical serialization (SPEC §3.2).
//!
//! The lock is the compiled graph, committed to `.stele/graph.lock`; it is the
//! engine's sole read path. Its serialization is a byte-diff contract: `check`
//! byte-compares a fresh in-memory rebuild against the committed file, so the
//! writer here is hand-rolled rather than delegated to a generic pretty-printer.
//! Reading is strict `serde_json` (an unknown `version` is rejected outright,
//! never best-effort parsed); the writer is the canonical serializer.
//!
//! Byte contract (§3.2): UTF-8, LF newlines, exactly one trailing LF, no trailing
//! whitespace, 2-space pretty-print. Object keys are emitted in Unicode-scalar
//! order; `depends`/`decided_by`/`allow` keep authored order while
//! `imports`/`contains`/`claims`/`landmarks` sort by id/slug. Integers only.
//! Strings use minimal escaping — `\" \\ \b \f \n \r \t` plus mandatory C0
//! controls as `\u00xx`; every other character, non-ASCII included, is raw.

use crate::model::{Claim, Graph, Node, Result, SteleError, Verified};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The only lock format this engine reads or writes (§3.2). A committed lock with
/// any other `version` is rejected (exit 2), never best-effort parsed.
pub const LOCK_VERSION: u32 = 1;

/// Pretty-print indent width in spaces (§3.2 "2-space pretty-print").
const INDENT_WIDTH: usize = 2;

// ─── the lock data model (§3.2) ──────────────────────────────────────────────

/// The whole compiled graph. `BTreeMap` keys iterate in Unicode-scalar order —
/// which for UTF-8 `String` is byte order, exactly the canonical key order — so
/// `nodes`/`adrs`/`landmarks`/`commands` are pre-sorted by construction.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    #[serde(default)]
    pub adrs: BTreeMap<String, Adr>,
    #[serde(default)]
    pub landmarks: BTreeMap<String, Landmark>,
    #[serde(default)]
    pub nodes: BTreeMap<String, LockNode>,
    pub version: u32,
}

/// A compiled ADR entry keyed by its node id in `adrs` (§3.2).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adr {
    pub number: i64,
    pub path: String,
    pub status: String,
}

/// A resolved landmark keyed by its slug in `landmarks` (§3.2).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Landmark {
    pub file: String,
    pub line: i64,
    pub node: String,
}

/// A node object — every field present: empty containers `[]`/`{}`, absent
/// scalars `null` (§3.2). Field order here is irrelevant; the serializer emits
/// keys in canonical order regardless.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockNode {
    pub budget: Option<i64>,
    pub claims: Vec<LockClaim>,
    pub commands: BTreeMap<String, String>,
    pub contains: Vec<String>,
    pub declared: LockDeclared,
    pub extracted: LockExtracted,
    pub id: String,
    pub kind: String,
    pub purpose: Option<String>,
}

/// Authored edges (§2.2/§2.3): `depends`/`decided_by`/`allow` keep authored order.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockDeclared {
    pub allow: Vec<LockAllow>,
    pub decided_by: Vec<String>,
    pub depends: Vec<String>,
}

/// A tolerated cross-boundary edge (§4.2).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockAllow {
    pub edge: String,
    pub reason: String,
}

/// Extracted truth (§2.2). `imports` is sorted; Phase C fills it (empty until then).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockExtracted {
    pub imports: Vec<String>,
}

/// A claim object with its required `kind` discriminator (§3.2) so invariants and
/// hazards round-trip through one `claims[]` array. `id` is the derived slug.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockClaim {
    pub anchor: String,
    pub enforced_by: Option<String>,
    pub id: String,
    pub kind: String,
    pub resolved: Option<String>,
    pub text: String,
    pub verified: Option<LockVerified>,
}

/// Freshness watermark (§4.5), stamped by `build`. `digest` is absent for
/// parser-less languages. Carried over verbatim by `check`'s rebuild.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockVerified {
    pub digest: Option<String>,
    pub sha: String,
}

impl From<&Verified> for LockVerified {
    fn from(verified: &Verified) -> Self {
        Self {
            digest: verified.digest.clone(),
            sha: verified.sha.clone(),
        }
    }
}

// ─── building the lock from the in-memory graph (§5.1 pipeline) ───────────────

impl Lock {
    /// Compile the in-memory graph into a lock. `adrs`/`landmarks` and each
    /// node's `extracted.imports` stay empty until Phase C; every claim's
    /// `resolved`/`verified` is stamped `null` here, since anchor resolution and
    /// digesting do not exist yet. `contains` IS derived now — it is tree data
    /// already available from the node set.
    pub fn from_graph(graph: &Graph) -> Self {
        let contains = compute_contains(&graph.nodes);
        let mut nodes = BTreeMap::new();
        for node in &graph.nodes {
            let node_contains = contains.get(&node.id).cloned().unwrap_or_default();
            nodes.insert(node.id.clone(), LockNode::from_node(node, node_contains));
        }
        Self {
            adrs: BTreeMap::new(),
            landmarks: BTreeMap::new(),
            nodes,
            version: LOCK_VERSION,
        }
    }

    /// Copy each claim's `verified` watermark from the committed lock into this
    /// freshly-built one, matched by node id + claim id (§4.5 pinned decision).
    /// `check` rebuilds in-memory for the byte-compare but must NOT re-stamp
    /// `verified` — else any source edit would flip the compare to exit 2 and
    /// freshness (exit 1) could never fire. `build`, the sole stamper, never
    /// calls this.
    pub fn carry_over_verified(&mut self, committed: &Lock) {
        for (id, node) in &mut self.nodes {
            let Some(committed_node) = committed.nodes.get(id) else {
                continue;
            };
            for claim in &mut node.claims {
                if let Some(prior) = committed_node.claims.iter().find(|c| c.id == claim.id) {
                    claim.verified = prior.verified.clone();
                }
            }
        }
    }
}

impl LockNode {
    fn from_node(node: &Node, contains: Vec<String>) -> Self {
        let mut claims: Vec<LockClaim> = node
            .invariants
            .iter()
            .chain(node.hazards.iter())
            .map(LockClaim::from_claim)
            .collect();
        claims.sort_by(|a, b| a.id.cmp(&b.id));

        let mut imports = node.extracted_imports.clone();
        imports.sort();

        Self {
            budget: node.budget.map(|b| b as i64),
            claims,
            commands: node.commands.clone(),
            contains,
            declared: LockDeclared {
                allow: node
                    .edges
                    .allow
                    .iter()
                    .map(|a| LockAllow {
                        edge: a.edge.clone(),
                        reason: a.reason.clone(),
                    })
                    .collect(),
                decided_by: node.edges.decided_by.clone(),
                depends: node.edges.depends.clone(),
            },
            extracted: LockExtracted { imports },
            id: node.id.clone(),
            kind: node.kind.as_str().to_string(),
            purpose: node.purpose.clone(),
        }
    }
}

impl LockClaim {
    fn from_claim(claim: &Claim) -> Self {
        Self {
            anchor: claim.anchor.clone(),
            enforced_by: claim.enforced_by.clone(),
            id: claim.slug.clone(),
            kind: claim.kind.as_str().to_string(),
            resolved: claim.resolved.clone(),
            text: claim.text.clone(),
            verified: claim.verified.as_ref().map(LockVerified::from),
        }
    }
}

/// The containment tree (§3.2 `contains[]`): each node's directly-nested child
/// node ids, sorted. A node's parent is the DEEPEST other node that is an id
/// ancestor of it (the system node `/` is the ancestor of every other node), so
/// `apps/web/lib/billing` is contained by `apps/web`, never also by `/`.
fn compute_contains(nodes: &[Node]) -> BTreeMap<String, Vec<String>> {
    let mut contains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for child in nodes {
        if let Some(parent) = nearest_parent(nodes, &child.id) {
            contains
                .entry(parent.to_string())
                .or_default()
                .push(child.id.clone());
        }
    }
    for children in contains.values_mut() {
        children.sort();
    }
    contains
}

/// The deepest node whose id is an ancestor of `child_id`, or `None` for the
/// system node (which has no parent).
fn nearest_parent<'a>(nodes: &'a [Node], child_id: &str) -> Option<&'a str> {
    if child_id == crate::model::SYSTEM_ID {
        return None;
    }
    nodes
        .iter()
        .filter(|n| n.id != child_id && is_id_ancestor(&n.id, child_id))
        .max_by_key(|n| n.id.len())
        .map(|n| n.id.as_str())
}

/// Whether node id `ancestor` contains node id `descendant` by path nesting. The
/// system id `/` is an ancestor of every other node; otherwise `descendant` must
/// begin with `ancestor` followed by a `/` (so `apps/web` contains `apps/web/x`
/// but not `apps/website`).
fn is_id_ancestor(ancestor: &str, descendant: &str) -> bool {
    if ancestor == crate::model::SYSTEM_ID {
        return true;
    }
    descendant
        .strip_prefix(ancestor)
        .is_some_and(|rest| rest.starts_with('/'))
}

// ─── reading (strict `serde_json`) ────────────────────────────────────────────

/// The `version` alone, read leniently so an unknown-version lock is rejected
/// (exit 2) BEFORE its node shape is validated against this engine's schema
/// (§3.2 "never best-effort parse").
#[derive(Deserialize)]
struct VersionProbe {
    version: u32,
}

/// The committed lock's `version`, or an input error (exit 2) if the text is not
/// valid JSON carrying an integer `version`.
pub fn read_version(text: &str) -> Result<u32> {
    let probe: VersionProbe = serde_json::from_str(text)
        .map_err(|e| SteleError::input_msg(format!("malformed lock: {e}")))?;
    Ok(probe.version)
}

/// Strict-parse a committed lock (a stray field is an input error, exit 2). The
/// caller checks `version` first via [`read_version`]; this validates the shape.
pub fn parse_lock(text: &str) -> Result<Lock> {
    serde_json::from_str(text).map_err(|e| SteleError::input_msg(format!("malformed lock: {e}")))
}

// ─── canonical serialization (the byte contract, §3.2) ────────────────────────

/// A minimal JSON tree the canonical writer emits. `Object` entries are held in
/// canonical (Unicode-scalar) key order — [`Json::object`] sorts on construction,
/// so callers may list keys in any order.
enum Json {
    Array(Vec<Json>),
    Int(i64),
    Null,
    Object(Vec<(String, Json)>),
    Str(String),
}

impl Json {
    /// An object with keys sorted into canonical (Unicode-scalar) order. Sorting
    /// `String` keys is a byte-wise compare, which for UTF-8 equals scalar order.
    fn object(mut entries: Vec<(String, Json)>) -> Self {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Json::Object(entries)
    }
}

/// Serialize a lock to its canonical bytes (§3.2): 2-space pretty-print, sorted
/// keys, exactly one trailing LF. This is the sole writer of the lock format.
pub fn to_canonical_string(lock: &Lock) -> String {
    let mut out = String::new();
    write_json(&mut out, &lock_to_json(lock), 0);
    out.push('\n');
    out
}

fn lock_to_json(lock: &Lock) -> Json {
    Json::object(vec![
        ("adrs".to_string(), adrs_to_json(&lock.adrs)),
        ("landmarks".to_string(), landmarks_to_json(&lock.landmarks)),
        ("nodes".to_string(), nodes_to_json(&lock.nodes)),
        ("version".to_string(), Json::Int(lock.version as i64)),
    ])
}

fn adrs_to_json(adrs: &BTreeMap<String, Adr>) -> Json {
    Json::object(
        adrs.iter()
            .map(|(id, adr)| {
                (
                    id.clone(),
                    Json::object(vec![
                        ("number".to_string(), Json::Int(adr.number)),
                        ("path".to_string(), Json::Str(adr.path.clone())),
                        ("status".to_string(), Json::Str(adr.status.clone())),
                    ]),
                )
            })
            .collect(),
    )
}

fn landmarks_to_json(landmarks: &BTreeMap<String, Landmark>) -> Json {
    Json::object(
        landmarks
            .iter()
            .map(|(slug, lm)| {
                (
                    slug.clone(),
                    Json::object(vec![
                        ("file".to_string(), Json::Str(lm.file.clone())),
                        ("line".to_string(), Json::Int(lm.line)),
                        ("node".to_string(), Json::Str(lm.node.clone())),
                    ]),
                )
            })
            .collect(),
    )
}

fn nodes_to_json(nodes: &BTreeMap<String, LockNode>) -> Json {
    Json::object(
        nodes
            .iter()
            .map(|(id, node)| (id.clone(), node_to_json(node)))
            .collect(),
    )
}

fn node_to_json(node: &LockNode) -> Json {
    Json::object(vec![
        ("budget".to_string(), opt_int(node.budget)),
        (
            "claims".to_string(),
            Json::Array(node.claims.iter().map(claim_to_json).collect()),
        ),
        ("commands".to_string(), str_map_to_json(&node.commands)),
        ("contains".to_string(), str_array_to_json(&node.contains)),
        ("declared".to_string(), declared_to_json(&node.declared)),
        (
            "extracted".to_string(),
            Json::object(vec![(
                "imports".to_string(),
                str_array_to_json(&node.extracted.imports),
            )]),
        ),
        ("id".to_string(), Json::Str(node.id.clone())),
        ("kind".to_string(), Json::Str(node.kind.clone())),
        ("purpose".to_string(), opt_str(&node.purpose)),
    ])
}

fn declared_to_json(declared: &LockDeclared) -> Json {
    Json::object(vec![
        (
            "allow".to_string(),
            Json::Array(
                declared
                    .allow
                    .iter()
                    .map(|a| {
                        Json::object(vec![
                            ("edge".to_string(), Json::Str(a.edge.clone())),
                            ("reason".to_string(), Json::Str(a.reason.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "decided_by".to_string(),
            str_array_to_json(&declared.decided_by),
        ),
        ("depends".to_string(), str_array_to_json(&declared.depends)),
    ])
}

fn claim_to_json(claim: &LockClaim) -> Json {
    Json::object(vec![
        ("anchor".to_string(), Json::Str(claim.anchor.clone())),
        ("enforced_by".to_string(), opt_str(&claim.enforced_by)),
        ("id".to_string(), Json::Str(claim.id.clone())),
        ("kind".to_string(), Json::Str(claim.kind.clone())),
        ("resolved".to_string(), opt_str(&claim.resolved)),
        ("text".to_string(), Json::Str(claim.text.clone())),
        ("verified".to_string(), verified_to_json(&claim.verified)),
    ])
}

fn verified_to_json(verified: &Option<LockVerified>) -> Json {
    match verified {
        None => Json::Null,
        Some(v) => Json::object(vec![
            ("digest".to_string(), opt_str(&v.digest)),
            ("sha".to_string(), Json::Str(v.sha.clone())),
        ]),
    }
}

fn opt_int(value: Option<i64>) -> Json {
    value.map_or(Json::Null, Json::Int)
}

fn opt_str(value: &Option<String>) -> Json {
    value.as_ref().map_or(Json::Null, |s| Json::Str(s.clone()))
}

fn str_array_to_json(values: &[String]) -> Json {
    Json::Array(values.iter().map(|s| Json::Str(s.clone())).collect())
}

fn str_map_to_json(map: &BTreeMap<String, String>) -> Json {
    Json::object(
        map.iter()
            .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
            .collect(),
    )
}

fn write_json(out: &mut String, value: &Json, depth: usize) {
    match value {
        Json::Array(items) => write_array(out, items, depth),
        Json::Int(n) => out.push_str(&n.to_string()),
        Json::Null => out.push_str("null"),
        Json::Object(entries) => write_object(out, entries, depth),
        Json::Str(s) => write_string(out, s),
    }
}

fn write_array(out: &mut String, items: &[Json], depth: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('\n');
        push_indent(out, depth + 1);
        write_json(out, item, depth + 1);
    }
    out.push('\n');
    push_indent(out, depth);
    out.push(']');
}

fn write_object(out: &mut String, entries: &[(String, Json)], depth: usize) {
    if entries.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push('{');
    for (i, (key, value)) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('\n');
        push_indent(out, depth + 1);
        write_string(out, key);
        out.push_str(": ");
        write_json(out, value, depth + 1);
    }
    out.push('\n');
    push_indent(out, depth);
    out.push('}');
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth * INDENT_WIDTH {
        out.push(' ');
    }
}

/// Emit a JSON string with §3.2 minimal escaping: the named escapes, C0 controls
/// as lowercase `\u00xx`, everything else (non-ASCII included) raw.
fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
