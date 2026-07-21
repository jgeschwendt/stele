//! The assertion suite (SPEC §4) — framework plus the referential class (§4.1).
//!
//! `check` rebuilds the graph in-memory and byte-compares it to the committed lock
//! (§5.3); AFTER that compare it runs the enabled assertion classes over that graph.
//! Any [`Finding`] maps to exit 1 ("repo out of spec", never a tool malfunction —
//! that is exit 2/3). Each class is independently toggleable: `check.disable` skips
//! classes (§3.4) and `--only <class>` runs exactly one (§5.1). Human output follows
//! the EXAMPLE §8 gallery shape; the same findings populate the `--json` envelope's
//! `findings[]` (§5.3). All six classes — referential (§4.1), structural (§4.2),
//! exhaustiveness (§4.3), budget (§4.4), freshness (§4.5), and liveness (§4.6) — are
//! landed.

use crate::anchors::{self, RegionDigest};
use crate::config::{AssertionClass, Config};
use crate::emit;
use crate::lock::{Lock, LockVerified};
use crate::model::{
    Claim, ClaimAnchor, ClaimLookup, Graph, LANDMARK_ANCHOR_PREFIX, Node, NodeKind, Resolution,
    Result, SYSTEM_ID, SteleError,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The `stele:landmark` comment token (§2.5), quoted verbatim in the §4.1 unresolved
/// message so the fix ("re-add the comment") is unambiguous.
const LANDMARK_COMMENT_TOKEN: &str = "stele:landmark";
/// The lowercased ADR status that fails a `decided_by` target (§2.6/§4.1).
const SUPERSEDED_STATUS: &str = "superseded";

/// The six assertion classes in §4 declaration order (§4.1 referential … §4.6
/// liveness). Order is meaningful — findings are reported in this order, matching the
/// EXAMPLE §8 gallery — so it is deliberately NOT alpha-sorted.
pub const ALL_CLASSES: [AssertionClass; 6] = [
    AssertionClass::Referential,
    AssertionClass::Structural,
    AssertionClass::Exhaustiveness,
    AssertionClass::Budget,
    AssertionClass::Freshness,
    AssertionClass::Liveness,
];

impl AssertionClass {
    /// The wire/flag name of a class (§5.1 `--only`, §5.3 `findings[].class`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::Exhaustiveness => "exhaustiveness",
            Self::Freshness => "freshness",
            Self::Liveness => "liveness",
            Self::Referential => "referential",
            Self::Structural => "structural",
        }
    }

    /// Parse a `--only <class>` value, or `None` when it names no class (the caller
    /// maps that to an exit-2 bad-flag error, §5.3).
    pub fn parse(name: &str) -> Option<Self> {
        ALL_CLASSES.into_iter().find(|class| class.as_str() == name)
    }
}

// ─── findings (§5.3 findings[]) ───────────────────────────────────────────────

/// A finding's severity (§5.3). Every Phase D finding is an error; the enum leaves
/// room for a future warn tier without reshaping the envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
        }
    }
}

/// A source location in a finding (§5.3 `findings[].locations[]`). `text` is a
/// human-only enrichment (the reference's own source line, EXAMPLE 8.1); the
/// `--json` envelope carries only `{file, line}`.
#[derive(Clone, Debug)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub text: Option<String>,
}

/// One assertion finding (§4, §5.3). `details` are human-only enrichment lines (the
/// gallery's context lines, e.g. "claim … is now unanchored"); the `--json` envelope
/// carries only `{class, severity, node, message, fix, locations}`. `profile` is the
/// §4.4 budget sub-label (`codex`/`node`/`claude`): the class stays `budget` for
/// `--only`/`disable`/`json` (§3.4/§5.1), while the human render prints `budget[codex]`
/// and the JSON message carries the `[codex]` prefix so the profile is machine-visible.
#[derive(Clone, Debug)]
pub struct Finding {
    pub class: AssertionClass,
    pub profile: Option<&'static str>,
    pub severity: Severity,
    pub node: Option<String>,
    pub message: String,
    pub details: Vec<String>,
    pub locations: Vec<Location>,
    pub fix: Option<String>,
}

impl Finding {
    /// An error finding with no details/locations/fix; the builders below add them.
    fn error(class: AssertionClass, node: Option<String>, message: String) -> Self {
        Self {
            class,
            profile: None,
            severity: Severity::Error,
            node,
            message,
            details: Vec::new(),
            locations: Vec::new(),
            fix: None,
        }
    }

    /// Tag a finding with its §4.4 budget profile (`codex`/`node`/`claude`).
    fn profile(mut self, profile: &'static str) -> Self {
        self.profile = Some(profile);
        self
    }

    /// The display class for the human render (§4.4): `budget[codex]` when a profile
    /// is set, else the bare class name (`referential`, `structural`, …).
    fn display_class(&self) -> String {
        match self.profile {
            Some(profile) => format!("{}[{profile}]", self.class.as_str()),
            None => self.class.as_str().to_string(),
        }
    }

    fn detail(mut self, line: impl Into<String>) -> Self {
        self.details.push(line.into());
        self
    }

    fn location(mut self, file: impl Into<String>, line: usize) -> Self {
        self.locations.push(Location {
            file: file.into(),
            line,
            text: None,
        });
        self
    }

    /// A location carrying the reference's own source line (§4.2 structural), printed
    /// after `file:line` in the human render; the JSON envelope still omits `text`.
    fn location_text(
        mut self,
        file: impl Into<String>,
        line: usize,
        text: impl Into<String>,
    ) -> Self {
        self.locations.push(Location {
            file: file.into(),
            line,
            text: Some(text.into()),
        });
        self
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    /// The §5.3 `findings[]` JSON shape. `details` stay human-only; `node`/`fix` are
    /// `null` when absent. A §4.4 budget `profile` is folded into `message` as a
    /// leading `[codex]` tag (the envelope schema carries no profile field) so the
    /// class stays `budget` yet the profile remains machine-readable.
    pub fn to_json(&self) -> Value {
        let message = match self.profile {
            Some(profile) => format!("[{profile}] {}", self.message),
            None => self.message.clone(),
        };
        json!({
            "class": self.class.as_str(),
            "severity": self.severity.as_str(),
            "node": self.node,
            "message": message,
            "fix": self.fix,
            "locations": self.locations
                .iter()
                .map(|loc| json!({ "file": loc.file, "line": loc.line }))
                .collect::<Vec<_>>(),
        })
    }
}

/// The human render of a finding list (EXAMPLE §8 gallery shape): each finding is a
/// `✗ <class>: <message>` line, then its locations (4-space indent, `file:line`), its
/// detail lines (2-space indent), and its `fix:` line (2-space indent) when set. The
/// trailing `exit 1` in the gallery is the PROCESS exit, not printed text.
pub fn render_human(findings: &[Finding]) -> String {
    let mut out = String::new();
    for finding in findings {
        out.push_str(&format!(
            "✗ {}: {}\n",
            finding.display_class(),
            finding.message
        ));
        for loc in &finding.locations {
            match &loc.text {
                Some(text) => out.push_str(&format!("    {}:{}  {}\n", loc.file, loc.line, text)),
                None => out.push_str(&format!("    {}:{}\n", loc.file, loc.line)),
            }
        }
        for detail in &finding.details {
            out.push_str(&format!("  {detail}\n"));
        }
        if let Some(fix) = &finding.fix {
            out.push_str(&format!("  fix: {fix}\n"));
        }
    }
    out
}

// ─── the registry ─────────────────────────────────────────────────────────────

/// Everything an assertion class reads: the rebuilt graph plus the check-time config
/// (§3.4), the VCS-tracked file set (§4.1 enforced_by, §4.3 exhaustiveness scope), and
/// the committed lock — the sole carrier of `verified {sha, digest}` on the check path
/// (§4.5; `build` stamps it, `check` never re-stamps, so the rebuilt graph's claims
/// carry no watermark and freshness reads it from here).
pub struct Context<'a> {
    pub committed: &'a Lock,
    pub config: &'a Config,
    pub graph: &'a Graph,
    pub root: &'a Path,
    /// `check --run-commands` (§4.6): additionally EXECUTE each declared command from
    /// the repo root (the off-by-default bonfires tier). Resolution runs regardless.
    pub run_commands: bool,
    pub tracked: &'a [PathBuf],
}

impl Context<'_> {
    /// Whether `rel` (a repo-root-relative POSIX path) is a VCS-tracked file (§4.1
    /// enforced_by / §2.4 scan scope: tracked implies committed).
    fn tracks(&self, rel: &str) -> bool {
        self.tracked
            .iter()
            .any(|path| path.to_string_lossy().replace('\\', "/") == rel)
    }
}

/// Run the enabled assertion classes over the rebuilt graph (§4). `only` selects
/// exactly one class (§5.1 `--only`); otherwise `config.check.disable` skips classes
/// (§3.4). Findings come back in class order (§4.1–§4.6); the caller maps a non-empty
/// result to exit 1 (§5.3).
pub fn run(ctx: &Context, only: Option<AssertionClass>) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for class in ALL_CLASSES {
        let enabled = match only {
            Some(selected) => class == selected,
            None => !ctx.config.check.disable.contains(&class),
        };
        if enabled {
            findings.extend(run_class(ctx, class)?);
        }
    }
    Ok(findings)
}

/// Dispatch one class (§4.1–§4.6). All six are landed; a clean repo yields no findings
/// and exits 0.
fn run_class(ctx: &Context, class: AssertionClass) -> Result<Vec<Finding>> {
    match class {
        AssertionClass::Budget => budget(ctx),
        AssertionClass::Exhaustiveness => exhaustiveness(ctx),
        AssertionClass::Freshness => freshness(ctx),
        AssertionClass::Referential => referential(ctx),
        AssertionClass::Structural => structural(ctx),
        AssertionClass::Liveness => liveness(ctx),
    }
}

// ─── referential (§4.1) ───────────────────────────────────────────────────────

/// The referential class (§4.1): (a) every claim anchor resolves; (b) every
/// referenced-or-declared landmark slug has slug-match cardinality exactly 1; (c)
/// every `decided_by` names an existing, non-superseded ADR; (d) every `enforced_by`
/// names a VCS-tracked file; (e) every `stele:claim` comment resolves to a declared
/// claim.
fn referential(ctx: &Context) -> Result<Vec<Finding>> {
    let graph = ctx.graph;
    let mut findings = Vec::new();

    // The landmark slugs whose cardinality (b) must hold: those a claim anchor
    // REFERENCES plus those a `stele:landmark` comment DECLARES (EXAMPLE 8.3b shows
    // both). A set keeps the union unique and ordered.
    let mut slugs: BTreeSet<String> = graph.anchors.landmarks.keys().cloned().collect();

    for node in &graph.nodes {
        for claim in node.invariants.iter().chain(node.hazards.iter()) {
            if let Some(slug) = claim.anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX) {
                slugs.insert(slug.to_string());
            }

            // (a) the anchor resolves (unresolved = 0, ambiguous = >1; §2.4).
            match claim.resolution {
                Resolution::Unresolved => findings.push(anchor_unresolved(node, claim)),
                Resolution::Ambiguous => findings.push(anchor_ambiguous(node, claim)),
                Resolution::Pending | Resolution::Resolved => {}
            }

            // (d) the enforced_by artifact is a VCS-tracked file.
            if let Some(artifact) = &claim.enforced_by
                && !ctx.tracks(artifact)
            {
                findings.push(Finding::error(
                    AssertionClass::Referential,
                    Some(node.id.clone()),
                    format!("enforced_by {artifact} is not a VCS-tracked file"),
                ));
            }
        }

        // (c) every decided_by names an existing, non-superseded ADR.
        for target in &node.edges.decided_by {
            match graph.adrs.iter().find(|adr| &adr.id == target) {
                None => findings.push(Finding::error(
                    AssertionClass::Referential,
                    Some(node.id.clone()),
                    format!("decided_by target {target} resolves to no ADR in the index"),
                )),
                Some(adr) if adr.status == SUPERSEDED_STATUS => findings.push(Finding::error(
                    AssertionClass::Referential,
                    Some(node.id.clone()),
                    format!("decided_by target {target} names a superseded ADR"),
                )),
                Some(_) => {}
            }
        }
    }

    // (b) slug-match cardinality exactly 1 (§2.5). A referenced slug with 0
    // occurrences is already reported by (a); only >1 is a cardinality finding.
    for slug in &slugs {
        let occurrences = graph
            .anchors
            .landmarks
            .get(slug)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if occurrences.len() > 1 {
            let mut finding = Finding::error(
                AssertionClass::Referential,
                None,
                format!(
                    "landmark {LANDMARK_ANCHOR_PREFIX}{slug} has slug-match cardinality {}",
                    occurrences.len()
                ),
            );
            for occ in occurrences {
                finding = finding.location(occ.file.clone(), occ.line);
            }
            findings.push(finding);
        }
    }

    // (e) every `stele:claim` comment resolves to a declared claim.
    for anchor in &graph.anchors.claims {
        match graph.resolve_claim(&anchor.addr) {
            ClaimLookup::Found(_) => {}
            ClaimLookup::Ambiguous => findings.push(claim_comment_dangling(
                anchor,
                format!(
                    "stele:claim {} is an ambiguous abbreviation (matches multiple nodes)",
                    anchor.addr
                ),
            )),
            ClaimLookup::NotFound => findings.push(claim_comment_dangling(
                anchor,
                format!("stele:claim {} resolves to no declared claim", anchor.addr),
            )),
        }
    }

    Ok(findings)
}

/// A §4.1(a) unresolved-anchor finding (0 occurrences/definitions). The message form
/// matches the EXAMPLE 8.3a gallery for `lm:`; a `<path>#<symbol>` anchor gets the
/// analogous "0 definitions" phrasing.
fn anchor_unresolved(node: &Node, claim: &Claim) -> Finding {
    let message = match claim.anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX) {
        Some(slug) => format!(
            "anchor {} unresolved (0 occurrences of \"{LANDMARK_COMMENT_TOKEN} {slug}\")",
            claim.anchor
        ),
        None => {
            let (path, symbol) = split_symbol(&claim.anchor);
            format!(
                "anchor {} unresolved (0 definitions of \"{symbol}\" in {path})",
                claim.anchor
            )
        }
    };
    Finding::error(AssertionClass::Referential, Some(node.id.clone()), message)
        .detail(unanchored(claim))
}

/// A §4.1(a) ambiguous-anchor finding (>1 definitions; only `<path>#<symbol>` anchors
/// can be ambiguous, §2.4).
fn anchor_ambiguous(node: &Node, claim: &Claim) -> Finding {
    let (path, symbol) = split_symbol(&claim.anchor);
    Finding::error(
        AssertionClass::Referential,
        Some(node.id.clone()),
        format!(
            "anchor {} ambiguous (>1 definitions of \"{symbol}\" in {path})",
            claim.anchor
        ),
    )
    .detail(unanchored(claim))
}

/// A §4.1(e) dangling `stele:claim` finding, anchored at the comment's `file:line`.
fn claim_comment_dangling(anchor: &ClaimAnchor, message: String) -> Finding {
    Finding::error(AssertionClass::Referential, None, message)
        .location(anchor.file.clone(), anchor.line)
}

/// The "claim … is now unanchored" detail line (EXAMPLE 8.3a).
fn unanchored(claim: &Claim) -> String {
    format!(
        "claim \"{}\" is now unanchored — provenance broken",
        claim.text
    )
}

/// Split a `<path>#<symbol>` anchor into its parts for §4.1(a) messages; a `#`-free
/// anchor (never produced for this arm) degrades to an empty symbol.
fn split_symbol(anchor: &str) -> (&str, &str) {
    anchor.rsplit_once('#').unwrap_or((anchor, ""))
}

// ─── structural (§4.2) ─────────────────────────────────────────────────────────

/// The committed structural baseline, relative to the repo root (§4.2 `--freeze`).
const FREEZE_DIR: &str = ".stele";
const FREEZE_FILE: &str = "freeze.json";
const FREEZE_PATH: &str = ".stele/freeze.json";
/// The freeze-file format version this engine reads and writes.
const FREEZE_VERSION: u32 = 1;

/// The two structural directions (§4.2). The string form is the `direction` key in
/// `.stele/freeze.json` and the discriminator in a [`FreezeKey`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    /// A declared `depends` edge with no extracted import backing it (reverse).
    Vestigial,
    /// An extracted cross-node import with no covering `depends` (forward).
    Violation,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vestigial => "vestigial",
            Self::Violation => "violation",
        }
    }
}

/// One structural violation's identity (§4.2 freeze key `{node, direction, target}`).
/// Field order is the canonical baseline sort (node, then direction, then target).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FreezeKey {
    node: String,
    direction: String,
    target: String,
}

/// A current structural violation: its stable [`FreezeKey`] (for baselining) plus the
/// [`Finding`] it renders to.
struct StructuralViolation {
    key: FreezeKey,
    finding: Finding,
}

/// The structural class (§4.2): every current violation in both directions, minus the
/// ones baselined in `.stele/freeze.json` (a NEW violation still surfaces; a baselined
/// one that no longer occurs is simply unused).
fn structural(ctx: &Context) -> Result<Vec<Finding>> {
    let baseline = read_freeze_baseline(ctx.root)?;
    Ok(structural_violations(ctx)
        .into_iter()
        .filter(|violation| !baseline.contains(&violation.key))
        .map(|violation| violation.finding)
        .collect())
}

/// Every CURRENT structural violation (§4.2), both directions, before any freeze
/// filtering — the set `--freeze` baselines and `check` filters against. A node with
/// empty `depends` is airtight (any cross-node import violates); an `allow` entry
/// suppresses BOTH directions for its edge.
fn structural_violations(ctx: &Context) -> Vec<StructuralViolation> {
    let mut violations = Vec::new();
    for node in &ctx.graph.nodes {
        let allowed: BTreeSet<&str> = node.edges.allow.iter().map(|a| a.edge.as_str()).collect();
        let declared: BTreeSet<&str> = node.edges.depends.iter().map(String::as_str).collect();

        // Forward (violation): an extracted cross-node import covered by neither a
        // declared depends edge nor an allow entry.
        for target in &node.extracted_imports {
            if declared.contains(target.as_str()) || allowed.contains(target.as_str()) {
                continue;
            }
            violations.push(forward_violation(ctx, node, target));
        }

        // Reverse (vestigial): a declared depends edge with no extracted import backing
        // it and no allow entry.
        for target in &node.edges.depends {
            if node.extracted_imports.iter().any(|import| import == target)
                || allowed.contains(target.as_str())
            {
                continue;
            }
            violations.push(vestigial_violation(node, target));
        }
    }
    violations
}

/// A forward violation (EXAMPLE 8.1): `<node> imports <target> — edge not declared`,
/// located at every contributing reference occurrence, with the node's declared
/// depends and the remove/declare/allow fix.
fn forward_violation(ctx: &Context, node: &Node, target: &str) -> StructuralViolation {
    let mut finding = Finding::error(
        AssertionClass::Structural,
        Some(node.id.clone()),
        format!("{} imports {target} — edge not declared", node.id),
    );
    if let Some(occurrences) = ctx
        .graph
        .import_edges
        .get(&(node.id.clone(), target.to_string()))
    {
        for occ in occurrences {
            finding = finding.location_text(occ.file.clone(), occ.line, occ.text.clone());
        }
    }
    finding = finding
        .detail(format!(
            "declared depends of {}: [{}]",
            node.id,
            node.edges.depends.join(", ")
        ))
        .fix(format!(
            "remove the import, or declare it in {} (and mean it), or \
             allow: {{edge: {target}, reason: \"...\"}} for dynamic/DI cases",
            node.source.display()
        ));
    StructuralViolation {
        key: freeze_key(node, Direction::Violation, target),
        finding,
    }
}

/// A vestigial violation (EXAMPLE 8.2): `<node> declares depends on <target> — no
/// import found`, with the doc-lied detail and the remove/allow fix.
fn vestigial_violation(node: &Node, target: &str) -> StructuralViolation {
    let finding = Finding::error(
        AssertionClass::Structural,
        Some(node.id.clone()),
        format!("{} declares depends on {target} — no import found", node.id),
    )
    .detail(
        "the signature promises a dependency the code no longer has \
         (doc lied, or dependency died)",
    )
    .fix(format!(
        "remove the edge from {}, or allow: with reason if the dep is runtime-dynamic",
        node.source.display()
    ));
    StructuralViolation {
        key: freeze_key(node, Direction::Vestigial, target),
        finding,
    }
}

fn freeze_key(node: &Node, direction: Direction, target: &str) -> FreezeKey {
    FreezeKey {
        node: node.id.clone(),
        direction: direction.as_str().to_string(),
        target: target.to_string(),
    }
}

// ─── the freeze baseline (§4.2 `--freeze`) ─────────────────────────────────────

/// `.stele/freeze.json` as read: an array of `{direction, node, target}` entries.
#[derive(Deserialize)]
struct FreezeFile {
    #[serde(default)]
    violations: Vec<FreezeEntry>,
}

#[derive(Deserialize)]
struct FreezeEntry {
    direction: String,
    node: String,
    target: String,
}

/// Read the committed baseline (§4.2). An absent file is an empty baseline; malformed
/// content is an input error (exit 2); a genuine IO failure is internal (exit 3).
fn read_freeze_baseline(root: &Path) -> Result<BTreeSet<FreezeKey>> {
    let text = match std::fs::read_to_string(root.join(FREEZE_DIR).join(FREEZE_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(SteleError::internal(format!("read {FREEZE_PATH}: {e}"))),
    };
    let parsed: FreezeFile = serde_json::from_str(&text)
        .map_err(|e| SteleError::input_msg(format!("malformed {FREEZE_PATH}: {e}")))?;
    Ok(parsed
        .violations
        .into_iter()
        .map(|entry| FreezeKey {
            node: entry.node,
            direction: entry.direction,
            target: entry.target,
        })
        .collect())
}

/// `stele check --freeze` (§4.2): baseline every CURRENT structural violation (both
/// directions) into `.stele/freeze.json`, canonically sorted with a trailing LF, and
/// return the count. Every later `check` suppresses exactly these; new violations
/// still fail.
pub fn write_freeze(ctx: &Context) -> Result<usize> {
    let mut keys: Vec<FreezeKey> = structural_violations(ctx)
        .into_iter()
        .map(|violation| violation.key)
        .collect();
    keys.sort();
    keys.dedup();
    let dir = ctx.root.join(FREEZE_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| SteleError::internal(format!("create {FREEZE_DIR}: {e}")))?;
    std::fs::write(dir.join(FREEZE_FILE), freeze_to_canonical_string(&keys))
        .map_err(|e| SteleError::internal(format!("write {FREEZE_PATH}: {e}")))?;
    Ok(keys.len())
}

/// Serialize the baseline to its canonical bytes (§4.2): 2-space pretty-print, keys
/// in Unicode-scalar order, one trailing LF — a byte-stable, committed-friendly file.
fn freeze_to_canonical_string(keys: &[FreezeKey]) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!("  \"version\": {FREEZE_VERSION},\n"));
    if keys.is_empty() {
        out.push_str("  \"violations\": []\n");
    } else {
        out.push_str("  \"violations\": [\n");
        for (i, key) in keys.iter().enumerate() {
            let comma = if i + 1 < keys.len() { "," } else { "" };
            out.push_str("    {\n");
            out.push_str(&format!(
                "      \"direction\": {},\n",
                json_string(&key.direction)
            ));
            out.push_str(&format!("      \"node\": {},\n", json_string(&key.node)));
            out.push_str(&format!("      \"target\": {}\n", json_string(&key.target)));
            out.push_str(&format!("    }}{comma}\n"));
        }
        out.push_str("  ]\n");
    }
    out.push_str("}\n");
    out
}

/// A JSON string literal with the minimal escaping the baseline's content can need
/// (normalized ids and fixed direction words carry no control characters).
fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ─── exhaustiveness (§4.3) ─────────────────────────────────────────────────────

/// The two dir names always dropped from the walk (§4.3): the VCS metadata dir and
/// stele's own committed home. Gitignored paths, untracked dot-dirs, and every path a
/// committed root `.steleignore` matches never appear — the scan is built from the
/// filtered tracked-file list (`tracked_files`, cli.rs), so a `.steleignore`d subtree
/// contributes no files, hence no directory here: an ignored dir is INVISIBLE, never an
/// unmapped recall failure (§2.4). None of the three needs an explicit entry.
pub(crate) const IGNORED_DIRS: [&str; 2] = [".git", ".stele"];

/// The exhaustiveness class (§4.3): every non-ignored directory at depth ≤ D must map
/// into some NON-ROOT node's territory. The system node `/` is a catch-all for
/// import attribution only — it covers repo-root direct files (never a depth-≥1
/// directory), so a subtree reachable through no `container`/`component`/`adr` node is
/// unrouted context and fails. Pass-through dirs (structural ancestors of a node, like
/// `apps/`) are covered but the scan descends through them regardless of D.
fn exhaustiveness(ctx: &Context) -> Result<Vec<Finding>> {
    let scan = DirScan::build(ctx.tracked, &ctx.config.exhaustiveness.exclude);
    let covering = covering_dirs(ctx);
    let max_depth = ctx.config.exhaustiveness.depth;

    // The top-level (depth-1) directories seed the walk. They are NOT pass-through
    // descents, so the depth knob (D) gates them: at D=0 an uncovered top-level dir is
    // below the checked floor and never fires; a pass-through descent ignores D.
    let mut uncovered = Vec::new();
    for top in scan.children_of("") {
        walk_dir(&scan, &covering, &top, 1, max_depth, false, &mut uncovered);
    }
    uncovered.sort();
    uncovered.dedup();
    Ok(uncovered
        .into_iter()
        .map(|dir| uncovered_finding(&dir, scan.count(&dir)))
        .collect())
}

/// One step of the §4.3 descent over directory `dir` (repo-root-relative). `forced`
/// marks a pass-through descent (report an uncovered leaf regardless of D); the
/// top-level seed passes `false` so D gates it.
fn walk_dir(
    scan: &DirScan,
    covering: &BTreeSet<String>,
    dir: &str,
    depth: u32,
    max_depth: u32,
    forced: bool,
    out: &mut Vec<String>,
) {
    // (a) within a non-root node's territory (it is, or nested under a declared
    // location): covered, and every descendant inherits the mapping — stop.
    if within_territory(covering, dir) {
        return;
    }
    // (b) a structural ancestor of such a node (a pass-through like `apps/`): covered
    // itself, but its children do NOT inherit a mapping, so descend to check each one
    // regardless of D (§4.3 bullet 2/4).
    if is_pass_through(covering, dir) {
        for child in scan.children_of(dir) {
            walk_dir(scan, covering, &child, depth + 1, max_depth, true, out);
        }
        return;
    }
    // Neither: an uncovered region. Report its root (the shallowest uncovered dir);
    // its whole subtree rolls into this one finding via the recursive file count.
    if forced || depth <= max_depth {
        out.push(dir.to_string());
    }
}

/// The §8.6a finding for an uncovered directory: `<dir> (<n> files) is covered by no
/// node — unreachable via any router`, with a declare-a-node-or-exclude fix.
fn uncovered_finding(dir: &str, files: usize) -> Finding {
    let unit = if files == 1 { "file" } else { "files" };
    Finding::error(
        AssertionClass::Exhaustiveness,
        None,
        format!("{dir} ({files} {unit}) is covered by no node — unreachable via any router"),
    )
    .fix(format!(
        "declare a node for {dir} (add an AGENTS.md with a stele block), \
         or list it in exhaustiveness.exclude"
    ))
}

/// The directories that cover a subtree for §4.3: the declared location (parent of the
/// AGENTS.md) of every non-root `container`/`component` node, plus the detected ADR
/// dir. SPEC §4.3 bullet 4 lists `adr` among the covering node kinds; the ADR index
/// (§2.6) knows the dir, so a clean repo with a bare `adr/` (no AGENTS.md) passes.
fn covering_dirs(ctx: &Context) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    for node in &ctx.graph.nodes {
        if node.kind == NodeKind::System {
            continue;
        }
        if let Some(dir) = declared_location(&node.source) {
            dirs.insert(dir);
        }
    }
    for adr in &ctx.graph.adrs {
        if let Some((dir, _)) = adr.path.rsplit_once('/') {
            dirs.insert(dir.to_string());
        }
    }
    dirs
}

/// A node's declared location (§4.3): the repo-root-relative parent of its AGENTS.md,
/// or `None` for a root-declared node (the system node, already excluded).
fn declared_location(source: &Path) -> Option<String> {
    match source.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => Some(dir.to_string_lossy().replace('\\', "/")),
        _ => None,
    }
}

/// Whether `dir` lies within a covering node's territory (§4.3(a)): it equals a
/// covering dir, or is nested under one.
fn within_territory(covering: &BTreeSet<String>, dir: &str) -> bool {
    covering.contains(dir) || covering.iter().any(|c| is_strict_ancestor(c, dir))
}

/// Whether `dir` is a pass-through (§4.3(b)): a structural ancestor of a covering dir.
fn is_pass_through(covering: &BTreeSet<String>, dir: &str) -> bool {
    covering.iter().any(|c| is_strict_ancestor(dir, c))
}

/// Whether `ancestor` is a strict path-segment ancestor of `descendant` (`apps`
/// precedes `apps/web` but not `apps` itself nor `appswide`).
fn is_strict_ancestor(ancestor: &str, descendant: &str) -> bool {
    if ancestor.is_empty() {
        return !descendant.is_empty();
    }
    descendant
        .strip_prefix(ancestor)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// The tracked-directory tree for §4.3: which directories exist (a dir exists iff it
/// holds ≥1 tracked file at any depth), each one's immediate children, and its
/// recursive tracked-file count. Built from the VCS-tracked file list with the ignore
/// set (`.git`, `.stele`, `exhaustiveness.exclude` globs) applied by segment.
struct DirScan {
    /// dir → recursive count of tracked files under it (`""` is the repo root).
    counts: BTreeMap<String, usize>,
    /// dir → its immediate child directories, sorted.
    children: BTreeMap<String, BTreeSet<String>>,
}

impl DirScan {
    fn build(tracked: &[PathBuf], excludes: &[String]) -> Self {
        let mut counts = BTreeMap::new();
        let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for path in tracked {
            let rel = path.to_string_lossy().replace('\\', "/");
            let segments: Vec<&str> = rel.split('/').collect();
            // A root-level file (no dir segment) belongs to the system node `/` by
            // construction (§4.3 bullet 2) — never an exhaustiveness subject.
            if segments.len() < 2 {
                continue;
            }
            let dir_segments = &segments[..segments.len() - 1];
            if dir_segments
                .iter()
                .any(|seg| is_ignored_segment(seg, excludes))
            {
                continue;
            }
            let mut prefix = String::new();
            let mut parent = String::new();
            for (i, seg) in dir_segments.iter().enumerate() {
                if i > 0 {
                    prefix.push('/');
                }
                prefix.push_str(seg);
                *counts.entry(prefix.clone()).or_insert(0) += 1;
                children
                    .entry(parent.clone())
                    .or_default()
                    .insert(prefix.clone());
                parent.clone_from(&prefix);
            }
        }
        Self { counts, children }
    }

    fn children_of(&self, dir: &str) -> Vec<String> {
        self.children
            .get(dir)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn count(&self, dir: &str) -> usize {
        self.counts.get(dir).copied().unwrap_or(0)
    }
}

/// Whether a single path segment is dropped from the §4.3 walk: a fixed ignored dir
/// (`.git`/`.stele`) or a match of an `exhaustiveness.exclude` glob.
fn is_ignored_segment(segment: &str, excludes: &[String]) -> bool {
    IGNORED_DIRS.contains(&segment) || excludes.iter().any(|glob| glob_matches(glob, segment))
}

/// A minimal `*`/`?` glob match (anchored, whole-segment) for `exhaustiveness.exclude`
/// (§3.4). The defaults are plain names; wildcards cover a team's custom entries.
fn glob_matches(pattern: &str, text: &str) -> bool {
    fn go(pattern: &[u8], text: &[u8]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some((b'*', rest)) => go(rest, text) || (!text.is_empty() && go(pattern, &text[1..])),
            Some((b'?', rest)) => !text.is_empty() && go(rest, &text[1..]),
            Some((byte, rest)) => text.first() == Some(byte) && go(rest, &text[1..]),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

// ─── budget (§4.4) ─────────────────────────────────────────────────────────────

/// Bytes per KiB, the §4.4 `codex` cap unit.
const BYTES_PER_KIB: usize = 1024;
/// Thousands-grouping width for the §4.4 token render (`1,410 tokens`).
const THOUSANDS_GROUP: usize = 3;

// Markdown structure tokens, reused from the §3.1 scanner (parse.rs owns the authored
// path; the budget segmenter re-derives them locally rather than widen that module).
const STELE_INFO: &str = "stele";
const FENCE_MIN_LEN: usize = 3;
const FENCE_MAX_INDENT: usize = 3;
const BEGIN_MARKER: &str = "stele:begin";
const END_MARKER: &str = "stele:end";

/// The budget class (§4.4): three profiles over materialized AGENTS.md content, each
/// finding tagged with its profile (the class stays `budget` for `--only`/`disable`).
/// `claude` counts the root's always-loaded set in TOKENS against `budget.claude_root`;
/// `codex` counts the worst root→leaf concatenation chain in BYTES against
/// `budget.codex_cap`; `node` counts each declared-budget node's own file in TOKENS.
/// Emission order (claude, codex, node) leaves a clean `codex`+`node` pair when the
/// root fits its `claude_root` ceiling, as in EXAMPLE 8.5.
fn budget(ctx: &Context) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    findings.extend(budget_claude(ctx)?);
    findings.extend(budget_codex(ctx)?);
    findings.extend(budget_node(ctx)?);
    Ok(findings)
}

/// `claude` profile (§4.4): the root node's always-loaded set — the root AGENTS.md
/// alone, since the root has no ancestors — counted in TOKENS against
/// `budget.claude_root` (default 2000). The always-loaded set generalizes to a node's
/// file plus its ancestors' (§4.4), but the ceiling is calibrated for the root.
fn budget_claude(ctx: &Context) -> Result<Vec<Finding>> {
    let Some(root) = ctx.graph.nodes.iter().find(|n| n.id == SYSTEM_ID) else {
        return Ok(Vec::new());
    };
    let tokens = count_tokens(&emit::materialized_content(ctx.root, root)?);
    let ceiling = ctx.config.budget.claude_root as usize;
    if tokens <= ceiling {
        return Ok(Vec::new());
    }
    Ok(vec![
        Finding::error(
            AssertionClass::Budget,
            Some(root.id.clone()),
            format!(
                "always-loaded set for {} renders {} tokens > claude_root {}",
                root.id,
                fmt_thousands(tokens),
                fmt_thousands(ceiling),
            ),
        )
        .profile("claude"),
    ])
}

/// `codex` profile (§4.4): for every node the root→node concatenation chain, counted
/// in BYTES against `budget.codex_cap` (default 32768, key `project_doc_max_bytes` in
/// the message). Reports the single WORST overflowing chain — its leaf id and size in
/// KiB to one decimal (EXAMPLE 8.5 `34.1 KiB`).
fn budget_codex(ctx: &Context) -> Result<Vec<Finding>> {
    let cap = ctx.config.budget.codex_cap as usize;
    let mut worst: Option<(&Node, usize)> = None;
    for node in &ctx.graph.nodes {
        let chain = emit::chain_files(ctx.tracked, node);
        let bytes = emit::chain_content(ctx.root, &chain)?.len();
        if bytes > cap && worst.is_none_or(|(_, w)| bytes > w) {
            worst = Some((node, bytes));
        }
    }
    let Some((leaf, bytes)) = worst else {
        return Ok(Vec::new());
    };
    Ok(vec![
        Finding::error(
            AssertionClass::Budget,
            Some(leaf.id.clone()),
            format!(
                "root chain {} → {} > {} default cap (project_doc_max_bytes) — \
                 Codex truncates the overflow (vendor docs report silent)",
                leaf.id,
                fmt_kib_tenths(bytes),
                fmt_kib_cap(cap),
            ),
        )
        .profile("codex"),
    ])
}

/// `node` profile (§4.4): every node with a declared `budget` whose own materialized
/// file exceeds it (in TOKENS). Names the largest contributing block — the authored
/// `stele` block, a generated region, or a free-prose section (named by its first
/// heading), whichever holds the most tokens (EXAMPLE 8.5).
fn budget_node(ctx: &Context) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for node in &ctx.graph.nodes {
        let Some(budget) = node.budget else {
            continue;
        };
        let content = emit::materialized_content(ctx.root, node)?;
        let tokens = count_tokens(&content);
        if tokens as u64 <= budget {
            continue;
        }
        let mut finding = Finding::error(
            AssertionClass::Budget,
            Some(node.id.clone()),
            format!(
                "{} renders {} tokens > declared budget {}",
                node.id,
                fmt_thousands(tokens),
                fmt_thousands(budget as usize),
            ),
        )
        .profile("node");
        if let Some((descriptor, block_tokens)) = largest_contributor(&content) {
            finding = finding.detail(format!(
                "largest contributor: {descriptor} ({} tokens)",
                fmt_thousands(block_tokens)
            ));
        }
        findings.push(finding);
    }
    Ok(findings)
}

// ─── the bundled tokenizer (§4.4) ──────────────────────────────────────────────

/// The bundled cl100k_base-class tokenizer (§4.4), built once and reused. Its identity
/// is folded into the lock version (`lock::LOCK_VERSION`), so a swap is a visible bump.
fn tokenizer() -> &'static tiktoken_rs::CoreBPE {
    static TOKENIZER: OnceLock<tiktoken_rs::CoreBPE> = OnceLock::new();
    TOKENIZER.get_or_init(|| tiktoken_rs::cl100k_base().expect("bundled cl100k vocab decodes"))
}

/// Token count of `text` under the bundled tokenizer (§4.4). `encode_ordinary` never
/// interprets special-token strings, so arbitrary doc content counts deterministically.
fn count_tokens(text: &str) -> usize {
    tokenizer().encode_ordinary(text).len()
}

// ─── largest-contributor segmentation (§4.4) ───────────────────────────────────

/// The largest §4.4 budget contributor in a materialized AGENTS.md: the block with the
/// most tokens among the authored `stele` block, each generated region, and each
/// free-prose section. `None` for a file with no non-blank content.
fn largest_contributor(content: &str) -> Option<(String, usize)> {
    segments(content)
        .into_iter()
        .map(|(descriptor, text)| (descriptor, count_tokens(&text)))
        .max_by_key(|(_, tokens)| *tokens)
}

/// Split a materialized AGENTS.md into its §4.4 budget segments in document order: the
/// authored `stele` fenced block, each generated `stele:begin…stele:end` region, and
/// each free-prose span between them. Each segment is `(descriptor, text)`.
fn segments(content: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let mut segs: Vec<(String, String)> = Vec::new();
    let mut prose = String::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(end) = stele_block_end(&lines, i) {
            flush_prose(&mut prose, &mut segs);
            segs.push((
                "authored stele block".to_string(),
                join_lines(&lines[i..=end]),
            ));
            i = end + 1;
        } else if let Some((name, end)) = generated_region(&lines, i) {
            flush_prose(&mut prose, &mut segs);
            segs.push((
                format!("generated {name} region"),
                join_lines(&lines[i..=end]),
            ));
            i = end + 1;
        } else {
            prose.push_str(lines[i]);
            prose.push('\n');
            i += 1;
        }
    }
    flush_prose(&mut prose, &mut segs);
    segs
}

/// Emit the accumulated free-prose span as a segment (named by its first heading, §4.4)
/// and reset the buffer; a blank-only span contributes nothing.
fn flush_prose(prose: &mut String, segs: &mut Vec<(String, String)>) {
    if prose.trim().is_empty() {
        prose.clear();
        return;
    }
    let descriptor = match first_heading(prose) {
        Some(heading) => format!("unmanaged prose block \"{heading}\""),
        None => "unmanaged prose block (untitled)".to_string(),
    };
    segs.push((descriptor, std::mem::take(prose)));
}

/// If line `i` opens the authored `stele` fence, the index of its closing fence (or the
/// last line for an unterminated block); else `None`.
fn stele_block_end(lines: &[&str], i: usize) -> Option<usize> {
    let (fence_char, fence_len) = open_stele_fence(lines[i])?;
    let close = (i + 1..lines.len()).find(|&j| is_close_fence(lines[j], fence_char, fence_len));
    Some(close.unwrap_or(lines.len() - 1))
}

/// The `(fence_char, run_length)` of an opening ` ```stele ` fence (§3.1 item 1), or
/// `None` when line is not a stele-info fence.
fn open_stele_fence(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > FENCE_MAX_INDENT {
        return None;
    }
    let rest = &line[indent..];
    let fence_char = rest.chars().next().filter(|&c| c == '`' || c == '~')?;
    let run = rest.chars().take_while(|&c| c == fence_char).count();
    if run < FENCE_MIN_LEN {
        return None;
    }
    let info = rest[run..].split_whitespace().next().unwrap_or("");
    (info == STELE_INFO).then_some((fence_char, run))
}

/// A closing fence: the fence character, run length ≥ the opener's, no trailing info.
fn is_close_fence(line: &str, fence_char: char, open_len: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > FENCE_MAX_INDENT {
        return false;
    }
    let rest = &line[indent..];
    let run = rest.chars().take_while(|&c| c == fence_char).count();
    run >= open_len && rest[run..].trim().is_empty()
}

/// If line `i` opens a generated region (`<!-- stele:begin <name> … -->`), its name and
/// the index of the line carrying `stele:end` (or the last line if unterminated); else
/// `None`. A single-line `begin…end` region closes on its own line.
fn generated_region(lines: &[&str], i: usize) -> Option<(String, usize)> {
    let after = lines[i].split(BEGIN_MARKER).nth(1)?;
    let name = after.split_whitespace().next().unwrap_or("").to_string();
    let end = (i..lines.len()).find(|&j| lines[j].contains(END_MARKER));
    Some((name, end.unwrap_or(lines.len() - 1)))
}

/// The first ATX heading line in `text` (leading `#`), trimmed, for a §4.4 free-prose
/// descriptor (EXAMPLE 8.5 `"## Style notes"`).
fn first_heading(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim_start)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim().to_string())
}

/// Rejoin scanned lines into a segment body (one `\n` per line); exact bytes are
/// immaterial here — this feeds only the relative token sizing of contributors.
fn join_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

// ─── number formatting (§4.4 render) ───────────────────────────────────────────

/// A count with `,` thousands separators (EXAMPLE 8.5 `1,410 tokens`).
fn fmt_thousands(n: usize) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(THOUSANDS_GROUP) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A byte size in KiB to one decimal (EXAMPLE 8.5 `34.1 KiB`) — the overflowing size.
fn fmt_kib_tenths(bytes: usize) -> String {
    format!("{:.1} KiB", bytes as f64 / BYTES_PER_KIB as f64)
}

/// The `codex` cap in KiB (EXAMPLE 8.5 `32 KiB`): whole KiB render as an integer, a
/// non-whole configured cap to one decimal.
fn fmt_kib_cap(bytes: usize) -> String {
    if bytes.is_multiple_of(BYTES_PER_KIB) {
        format!("{} KiB", bytes / BYTES_PER_KIB)
    } else {
        format!("{:.1} KiB", bytes as f64 / BYTES_PER_KIB as f64)
    }
}

// ─── freshness (§4.5) ──────────────────────────────────────────────────────────

/// Short-`sha` width in the freshness render (EXAMPLE 8.4 `e3f19ac`).
const SHORT_SHA_LEN: usize = 7;
/// Short-digest prefix width; rendered with a trailing `…` (EXAMPLE 8.4 `9c41…`).
const SHORT_DIGEST_LEN: usize = 4;
/// The horizontal-ellipsis (U+2026) that terminates a short digest (EXAMPLE 8.4).
const DIGEST_ELLIPSIS: char = '…';

/// The freshness class (§4.5). For every RESOLVED claim carrying a `verified` watermark
/// in the committed lock:
/// - Parser-backed (a stamped `digest`): recompute the bound region's digest from the
///   CURRENT working tree; a drift stales the claim — UNLESS `enforced_by` backs it, in
///   which case its guard (run by the same CI) is the freshness proof and the digest
///   change is exempt (EXAMPLE 8.4 note 2). Unresolved anchors are referential's
///   problem (§4.1), not freshness's, and are skipped.
/// - Parser-less (no digest): fall to a churn count — commits touching the anchored
///   file since `verified.sha`, thresholded by `freshness.churn_threshold` (prose-only)
///   or the longer `freshness.enforced_leash` (`enforced_by`-backed), with per-node
///   overrides (§3.4). An unset threshold disables the fallback for that claim.
fn freshness(ctx: &Context) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for node in &ctx.graph.nodes {
        for claim in node.invariants.iter().chain(node.hazards.iter()) {
            if let Some(finding) = freshness_claim(ctx, node, claim)? {
                findings.push(finding);
            }
        }
    }
    Ok(findings)
}

/// The freshness verdict for one claim (§4.5), or `None` when it is fresh, unresolved,
/// unwatermarked, exempt, or (parser-less) below/without a churn threshold.
fn freshness_claim(ctx: &Context, node: &Node, claim: &Claim) -> Result<Option<Finding>> {
    // Unresolved/ambiguous anchors are §4.1's finding, never freshness's.
    if claim.resolution != Resolution::Resolved {
        return Ok(None);
    }
    let Some(resolved) = &claim.resolved else {
        return Ok(None);
    };
    // The watermark lives only in the committed lock on the check path (§4.5).
    let Some(verified) = committed_verified(ctx.committed, &node.id, &claim.slug) else {
        return Ok(None);
    };
    let file = resolved_file(resolved);

    match &verified.digest {
        // Parser-backed: AST-region digest staling.
        Some(old_digest) => {
            // `enforced_by`-backed claims are exempt from digest-staling (EXAMPLE 8.4).
            if claim.enforced_by.is_some() {
                return Ok(None);
            }
            let Some(current) =
                anchors::region_digest_for_claim(ctx.root, &claim.anchor, resolved)?
            else {
                return Ok(None);
            };
            if &current.digest == old_digest {
                return Ok(None);
            }
            let staling = staling_commit(ctx.root, file, &claim.anchor, &verified.sha, old_digest)?;
            Ok(Some(digest_drift_finding(
                ctx.graph, node, claim, verified, old_digest, &current, staling,
            )))
        }
        // Parser-less: churn-count fallback, region granularity = the anchored file (v1).
        None => {
            let Some(threshold) =
                churn_threshold(ctx.config, &node.id, claim.enforced_by.is_some())
            else {
                return Ok(None);
            };
            // A watermark absent from history (shallow clone) makes churn uncountable —
            // `verified_sha..HEAD` would silently yield 0 and pass a possibly-staled claim
            // (F11). Report the honest failure instead.
            if !sha_reachable(ctx.root, &verified.sha)? {
                return Ok(Some(history_unavailable_finding(
                    ctx.graph, node, claim, verified, file,
                )));
            }
            let count = churn_count(ctx.root, file, &verified.sha)?;
            if count <= threshold as usize {
                return Ok(None);
            }
            Ok(Some(churn_finding(
                ctx.graph, node, claim, verified, file, count, threshold,
            )))
        }
    }
}

/// The committed lock's `verified` watermark for a claim, matched by node id + slug
/// (§4.5); `None` when the claim was never stamped (unresolved at last `build`).
fn committed_verified<'a>(
    committed: &'a Lock,
    node_id: &str,
    slug: &str,
) -> Option<&'a LockVerified> {
    committed
        .nodes
        .get(node_id)?
        .claims
        .iter()
        .find(|c| c.id == slug)?
        .verified
        .as_ref()
}

/// The churn threshold for a parser-less claim (§3.4/§4.5): `enforced_leash` for an
/// `enforced_by`-backed claim (the longer leash — the guard is the proof), else
/// `churn_threshold`. A per-node override (`[freshness.node."<id>"]`) wins over the
/// global; both unset → `None` disables the fallback for that claim.
fn churn_threshold(config: &Config, node_id: &str, enforced: bool) -> Option<u32> {
    let node_cfg = config.freshness.node.get(node_id);
    if enforced {
        node_cfg
            .and_then(|n| n.enforced_leash)
            .or(config.freshness.enforced_leash)
    } else {
        node_cfg
            .and_then(|n| n.churn_threshold)
            .or(config.freshness.churn_threshold)
    }
}

/// The §8.4 digest-drift finding: the `AST digest … changed` message, the verified/now
/// detail, the staling-commit pointer, and the re-affirm fix.
fn digest_drift_finding(
    graph: &Graph,
    node: &Node,
    claim: &Claim,
    verified: &LockVerified,
    old_digest: &str,
    current: &RegionDigest,
    staling: Staling,
) -> Finding {
    let addr = claim_address(graph, &node.id, &claim.slug);
    let staling_line = staling.line(&addr, true);
    Finding::error(
        AssertionClass::Freshness,
        Some(node.id.clone()),
        format!("claim {addr} — AST digest of enclosing region changed"),
    )
    .detail(format!(
        "verified at {} (digest {}), region {} now digests {}",
        short_sha(&verified.sha),
        short_digest(old_digest),
        current.name,
        short_digest(&current.digest),
    ))
    .detail(staling_line)
    .fix("re-read the region, re-affirm or amend the claim, `stele build` re-stamps {sha, digest}")
}

/// The parser-less churn-fallback finding (§4.5): the file crossed its leash. Region
/// granularity is the whole anchored file (a documented v1 limitation).
fn churn_finding(
    graph: &Graph,
    node: &Node,
    claim: &Claim,
    verified: &LockVerified,
    file: &str,
    count: usize,
    threshold: u32,
) -> Finding {
    let addr = claim_address(graph, &node.id, &claim.slug);
    Finding::error(
        AssertionClass::Freshness,
        Some(node.id.clone()),
        format!("claim {addr} — anchored file churned {count} commits past its freshness leash {threshold}"),
    )
    .detail(format!(
        "verified at {}; {file} (parser-less, file-granularity) changed in {count} commit(s) > leash {threshold} — `stele blame {addr}`",
        short_sha(&verified.sha),
    ))
    .fix("re-read the region, re-affirm or amend the claim, `stele build` re-stamps {sha}")
}

/// The shallow-clone freshness finding (§4.5): the watermark commit is not in this repo's
/// history, so churn is uncountable — an honest failure rather than a silent pass (F11).
/// Points at the fetch-depth fix.
fn history_unavailable_finding(
    graph: &Graph,
    node: &Node,
    claim: &Claim,
    verified: &LockVerified,
    file: &str,
) -> Finding {
    let addr = claim_address(graph, &node.id, &claim.slug);
    Finding::error(
        AssertionClass::Freshness,
        Some(node.id.clone()),
        format!("claim {addr} — cannot verify freshness: history unavailable (shallow clone?)"),
    )
    .detail(format!(
        "verified at {}, a commit absent from this repo's history — a shallow / fetch-depth 1 clone cannot compute churn for {file} — `stele blame {addr}`",
        short_sha(&verified.sha),
    ))
    .fix("fetch full history (git fetch --unshallow, or actions/checkout with fetch-depth: 0), then re-run stele check")
}

// ─── the staling-commit walk (§4.5, `stele blame`) ─────────────────────────────

/// A commit that staled a claim: its short sha and subject (EXAMPLE 8.4
/// `b8e02d1 "loosen cap for partial captures"`).
struct StalingCommit {
    short: String,
    subject: String,
}

/// Why a digest-backed claim staled (§4.5), the three honest outcomes of the walk:
///   • `Commit`             — the oldest committed version whose digest diverges;
///   • `Uncommitted`        — no committed version diverges AND the working tree differs
///                            from HEAD, so a local uncommitted edit is the cause;
///   • `HistoryUnavailable` — the watermark commit is not in this repo's history (a
///                            shallow / fetch-depth 1 clone), so the walk cannot run — an
///                            uncommitted-edit claim would be a LIE (F11/F12).
enum Staling {
    Commit(StalingCommit),
    HistoryUnavailable,
    Uncommitted,
}

impl Staling {
    /// The `staling commit: …` attribution line (§4.5). `blame_hint` appends the
    /// `stele blame <addr>` pointer the freshness finding carries (EXAMPLE 8.4); `stele
    /// blame`'s own render omits it.
    fn line(&self, addr: &str, blame_hint: bool) -> String {
        let hint = if blame_hint {
            format!(" — `stele blame {addr}`")
        } else {
            String::new()
        };
        match self {
            Staling::Commit(c) => format!("staling commit: {} {:?}{hint}", c.short, c.subject),
            Staling::HistoryUnavailable => {
                format!("staling commit: history unavailable (shallow clone?){hint}")
            }
            Staling::Uncommitted => {
                format!("staling commit: (uncommitted working-tree change){hint}")
            }
        }
    }

    /// The `--json` `staling_commit` value (§5.3): the commit object, `null` for a local
    /// edit, or a `{status}` marker when history is unavailable.
    fn to_json(&self) -> Value {
        match self {
            Staling::Commit(c) => json!({ "sha": c.short, "subject": c.subject }),
            Staling::HistoryUnavailable => json!({ "status": "history-unavailable" }),
            Staling::Uncommitted => Value::Null,
        }
    }
}

/// The commit that staled a digest-backed claim (§4.5): walk every commit touching the
/// anchored `file` from `verified_sha` forward to `HEAD`, recomputing the bound-def
/// digest at each (`git show <sha>:<file>` → parse → digest), and return the FIRST
/// (oldest) whose digest diverges from `verified_digest`. When no committed version
/// diverges the drift is a working-tree edit — but ONLY when the tree actually differs
/// from HEAD; a `verified_sha` absent from history (shallow clone) yields
/// `HistoryUnavailable`, never a false "uncommitted" attribution (F12).
fn staling_commit(
    root: &Path,
    file: &str,
    anchor: &str,
    verified_sha: &str,
    verified_digest: &str,
) -> Result<Staling> {
    if !sha_reachable(root, verified_sha)? {
        return Ok(Staling::HistoryUnavailable);
    }
    let Some(commits) = commits_touching(root, file, verified_sha)? else {
        return Ok(Staling::HistoryUnavailable);
    };
    // `git rev-list` is newest-first; the staling commit is the OLDEST divergence.
    for sha in commits.iter().rev() {
        let diverges = match git_show(root, sha, file)? {
            // File absent at this commit → the region did not exist → divergent.
            None => true,
            Some(contents) => {
                anchors::region_digest_of_source(anchor, file, &contents)
                    .map(|region| region.digest)
                    .as_deref()
                    != Some(verified_digest)
            }
        };
        if diverges {
            return Ok(Staling::Commit(commit_meta(root, sha)?));
        }
    }
    // No committed version diverges: a genuine uncommitted edit iff the tree differs from
    // HEAD; otherwise history is unavailable (never claim an edit that is not there).
    if working_tree_differs(root, file)? {
        Ok(Staling::Uncommitted)
    } else {
        Ok(Staling::HistoryUnavailable)
    }
}

/// Whether the watermark commit `sha` is present in this repo's object database (§4.5).
/// A shallow / `fetch-depth: 1` clone prunes ancestor commits, so a watermark from before
/// the shallow boundary is absent — freshness must then report "history unavailable"
/// honestly, never silently pass a churn claim (F11) nor misattribute a digest drift to
/// an uncommitted edit (F12). A spawn failure is a §5.3 internal error.
fn sha_reachable(root: &Path, sha: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["cat-file", "-e", sha])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git cat-file -e`: {e}")))?;
    Ok(output.status.success())
}

/// Whether `file`'s working-tree bytes differ from HEAD (§4.5), via `git diff --quiet
/// HEAD -- <file>`: exit 1 = differ, exit 0 = identical. Any other status (e.g. unborn
/// HEAD) is treated as "no diff" so the caller does not assert a local edit it cannot
/// prove. A spawn failure is a §5.3 internal error.
fn working_tree_differs(root: &Path, file: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--", file])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git diff --quiet`: {e}")))?;
    Ok(output.status.code() == Some(1))
}

/// The commits touching `file` in `(verified_sha, HEAD]`, newest first (§4.5); `None`
/// when `verified_sha` is unreachable. Callers now pre-check reachability via
/// [`sha_reachable`] and map both this `None` and an unreachable sha to
/// `Staling::HistoryUnavailable`, so an unreachable watermark is reported honestly rather
/// than silently swallowed.
fn commits_touching(root: &Path, file: &str, verified_sha: &str) -> Result<Option<Vec<String>>> {
    let output = Command::new("git")
        .args(["rev-list", &format!("{verified_sha}..HEAD"), "--", file])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git rev-list`: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .map(str::to_string)
            .collect(),
    ))
}

/// The count of commits touching `file` since `verified_sha` (§4.5 churn fallback),
/// via `git rev-list --count`; an unreachable `verified_sha` counts 0.
fn churn_count(root: &Path, file: &str, verified_sha: &str) -> Result<usize> {
    let output = Command::new("git")
        .args([
            "rev-list",
            "--count",
            &format!("{verified_sha}..HEAD"),
            "--",
            file,
        ])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git rev-list --count`: {e}")))?;
    if !output.status.success() {
        return Ok(0);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0))
}

/// The blob of `file` at commit `sha` (`git show <sha>:<file>`), or `None` when the
/// path did not exist at that commit.
fn git_show(root: &Path, sha: &str, file: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["show", &format!("{sha}:{file}")])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git show`: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// A commit's short sha + subject line (`git show -s --format=%s`) for the §4.5 render.
fn commit_meta(root: &Path, sha: &str) -> Result<StalingCommit> {
    let output = Command::new("git")
        .args(["show", "-s", "--format=%s", sha])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git show -s`: {e}")))?;
    Ok(StalingCommit {
        short: sha.get(..SHORT_SHA_LEN).unwrap_or(sha).to_string(),
        subject: String::from_utf8_lossy(&output.stdout).trim().to_string(),
    })
}

// ─── `stele blame` (§5.1) ──────────────────────────────────────────────────────

/// `stele blame <node-id>/<slug>` (§5.1, §4.5): resolve the claim (full or abbreviated
/// node-id), read its committed `verified` watermark, and report the staling-commit
/// walk — STALE with the staling commit, up-to-date otherwise, or a parser-less churn
/// count. Returns `(human summary, --json data)`; an unresolved/ambiguous address is a
/// §5.3 input error (exit 2).
pub fn blame(ctx: &Context, address: &str) -> Result<(String, Value)> {
    let claim_ref = match ctx.graph.resolve_claim(address) {
        ClaimLookup::Found(found) => found,
        ClaimLookup::Ambiguous => {
            return Err(SteleError::input_msg(format!(
                "claim address {address:?} is ambiguous (its abbreviation matches multiple nodes)"
            )));
        }
        ClaimLookup::NotFound => {
            return Err(SteleError::input_msg(format!(
                "no claim {address:?} in the graph"
            )));
        }
    };
    let node_id = claim_ref.node_id.to_string();
    let claim = claim_ref.claim;
    let addr = claim_address(ctx.graph, &node_id, &claim.slug);

    let Some(verified) = committed_verified(ctx.committed, &node_id, &claim.slug) else {
        return Ok((
            format!(
                "{addr}: not verified in the committed lock (anchor unresolved at last build) — run stele build"
            ),
            json!({ "claim": addr, "node": node_id, "status": "unverified" }),
        ));
    };
    let Some(resolved) = &claim.resolved else {
        return Ok((
            format!("{addr}: anchor no longer resolves — run stele build"),
            json!({ "claim": addr, "node": node_id, "status": "unresolved" }),
        ));
    };
    let file = resolved_file(resolved);

    match &verified.digest {
        Some(old_digest) => {
            let current = anchors::region_digest_for_claim(ctx.root, &claim.anchor, resolved)?;
            match current {
                Some(region) if &region.digest != old_digest => {
                    let staling =
                        staling_commit(ctx.root, file, &claim.anchor, &verified.sha, old_digest)?;
                    let staling_line = staling.line(&addr, false);
                    let summary = format!(
                        "{addr}: STALE — region {} digest drifted\n  verified at {} (digest {}) → now {}\n  {}",
                        region.name,
                        short_sha(&verified.sha),
                        short_digest(old_digest),
                        short_digest(&region.digest),
                        staling_line,
                    );
                    let data = json!({
                        "claim": addr,
                        "node": node_id,
                        "status": "stale",
                        "region": region.name,
                        "verified_sha": verified.sha,
                        "old_digest": old_digest,
                        "new_digest": region.digest,
                        "staling_commit": staling.to_json(),
                    });
                    Ok((summary, data))
                }
                region => {
                    let name = region.map(|r| r.name);
                    let summary = format!(
                        "{addr}: up to date — verified at {} (digest {}){}",
                        short_sha(&verified.sha),
                        short_digest(old_digest),
                        name.as_ref()
                            .map_or(String::new(), |n| format!(", region {n} unchanged")),
                    );
                    Ok((
                        summary,
                        json!({
                            "claim": addr,
                            "node": node_id,
                            "status": "fresh",
                            "region": name,
                            "verified_sha": verified.sha,
                        }),
                    ))
                }
            }
        }
        None => {
            // A watermark absent from history (shallow clone) cannot anchor a churn count
            // — `git rev-list` would silently return 0 and read as "fresh" (F11). Report
            // the honest failure instead.
            if !sha_reachable(ctx.root, &verified.sha)? {
                return Ok((
                    format!(
                        "{addr}: parser-less anchor — history unavailable (shallow clone?); cannot count churn for {file} since {}",
                        short_sha(&verified.sha),
                    ),
                    json!({
                        "claim": addr,
                        "node": node_id,
                        "status": "history-unavailable",
                        "verified_sha": verified.sha,
                    }),
                ));
            }
            let count = churn_count(ctx.root, file, &verified.sha)?;
            Ok((
                format!(
                    "{addr}: parser-less anchor — {file} touched by {count} commit(s) since {} (file-granularity)",
                    short_sha(&verified.sha),
                ),
                json!({
                    "claim": addr,
                    "node": node_id,
                    "status": "churn",
                    "verified_sha": verified.sha,
                    "count": count,
                }),
            ))
        }
    }
}

// ─── shared freshness render helpers ───────────────────────────────────────────

/// The claim's display address (EXAMPLE 8.4 `billing/refund-cap`): the final-segment
/// abbreviation when that segment names exactly one node across the graph (§2.4), else
/// the full `<node-id>/<slug>`.
fn claim_address(graph: &Graph, node_id: &str, slug: &str) -> String {
    let segment = last_segment(node_id);
    let unique = graph
        .nodes
        .iter()
        .filter(|n| last_segment(&n.id) == segment)
        .count()
        == 1;
    if unique {
        format!("{segment}/{slug}")
    } else {
        format!("{node_id}/{slug}")
    }
}

/// The final path segment of a node id (the system id `/` stays itself).
fn last_segment(id: &str) -> &str {
    if id == SYSTEM_ID {
        return id;
    }
    id.rsplit('/').next().unwrap_or(id)
}

/// The file half of a `file:line` `resolved` string (§4.5).
fn resolved_file(resolved: &str) -> &str {
    resolved.rsplit_once(':').map_or(resolved, |(file, _)| file)
}

/// The short-sha prefix for the freshness render (EXAMPLE 8.4 `e3f19ac`).
fn short_sha(sha: &str) -> &str {
    sha.get(..SHORT_SHA_LEN).unwrap_or(sha)
}

/// A digest's short prefix with a trailing ellipsis (EXAMPLE 8.4 `9c41…`).
fn short_digest(digest: &str) -> String {
    let prefix = digest.get(..SHORT_DIGEST_LEN).unwrap_or(digest);
    format!("{prefix}{DIGEST_ELLIPSIS}")
}

// ─── liveness (§4.6) ───────────────────────────────────────────────────────────

/// A pragmatic set of `mix` builtin tasks (§4.6). Not exhaustive — a curated list of
/// the tasks a team actually wires into `commands:`; anything else must be a `mix.exs`
/// alias to resolve. Alpha-sorted.
const MIX_BUILTINS: [&str; 16] = [
    "clean",
    "cmd",
    "compile",
    "deps.clean",
    "deps.compile",
    "deps.get",
    "deps.tree",
    "deps.update",
    "docs",
    "escript.build",
    "format",
    "help",
    "loadpaths",
    "release",
    "run",
    "test",
];

/// A pragmatic set of `cargo` builtin subcommands (§4.6), same curation rationale as
/// [`MIX_BUILTINS`]; anything else must be a `.cargo/config.toml` `[alias]`. Alpha-sorted.
const CARGO_BUILTINS: [&str; 19] = [
    "add", "bench", "build", "check", "clean", "clippy", "doc", "fetch", "fix", "fmt", "init",
    "install", "new", "publish", "remove", "run", "test", "tree", "update",
];

/// The shell builtins the liveness resolver (§4.6) treats as always-resolved — rung (a),
/// ahead of PATH. A builtin is executed directly by the shell and has no executable file
/// to find on `PATH`, so PATH resolution would false-flag it; worse, some hosts ship a
/// PATH shim for a builtin (macOS `/usr/bin/cd`) and others do not, making resolution
/// platform-dependent. This is the 15 POSIX.1-2017 special built-ins (§2.14) plus the
/// ubiquitous regular built-ins standardized in the Shell & Utilities volume — `colon`
/// and `dot` appear as their command tokens `:` and `.`, `test` as both `test` and `[`.
/// Arguments past the first token are not inspected, consistent with the rest of §4.6.
/// Source: pubs.opengroup.org/onlinepubs/9699919799/utilities/V3_chap02.html#tag_18_14
/// (special built-ins) + .../V3_chap04.html (cd, echo, false, printf, pwd, read, test,
/// true, type, ulimit, umask, wait). Alpha-sorted (ASCII).
const SHELL_BUILTINS: [&str; 28] = [
    ".", ":", "[", "break", "cd", "continue", "echo", "eval", "exec", "exit", "export", "false",
    "printf", "pwd", "read", "readonly", "return", "set", "shift", "test", "times", "trap", "true",
    "type", "ulimit", "umask", "unset", "wait",
];

/// The liveness class (§4.6). For every declared command on every node: parse it into
/// shell-operator-separated segments and RESOLVE each segment's executable (always); and
/// when `ctx.run_commands` is set, additionally EXECUTE the whole command from the repo
/// root, reporting a non-zero exit. Resolution reads runner manifests once up front.
fn liveness(ctx: &Context) -> Result<Vec<Finding>> {
    let runners = Runners::detect(ctx.root);
    let path_env = std::env::var_os("PATH");
    let mut findings = Vec::new();
    for node in &ctx.graph.nodes {
        for (name, command) in &node.commands {
            for segment in split_segments(command) {
                if let Some(reason) =
                    resolve_segment(ctx.root, path_env.as_deref(), &runners, &segment)
                {
                    findings.push(unresolved_command(node, name, &segment, reason));
                }
            }
            if ctx.run_commands
                && let Some(failure) = execute_command(ctx.root, command)?
            {
                findings.push(command_exited(node, name, command, &failure));
            }
        }
    }
    Ok(findings)
}

/// The §8.6b unresolved-command finding: `command <node> :<name> → `<segment>` — <reason>`.
/// The node id and command name identify the declaration; the segment is the exact
/// operator-delimited piece that failed to resolve.
fn unresolved_command(node: &Node, name: &str, segment: &str, reason: &str) -> Finding {
    Finding::error(
        AssertionClass::Liveness,
        Some(node.id.clone()),
        format!("command {} :{name} → `{segment}` — {reason}", node.id),
    )
}

/// The `--run-commands` execution finding (§4.6): a declared command exited non-zero
/// when run from the repo root. Scoping is cwd-only — no sandbox (SPEC §4.6 bonfires).
/// The child's captured stdout+stderr (see [`execute_command`]) fold into the finding as
/// a detail so the diagnostic is preserved WITHOUT leaking onto the engine's own stdout —
/// the §5.3 `--json` envelope stays a single object.
fn command_exited(node: &Node, name: &str, command: &str, failure: &CommandFailure) -> Finding {
    let finding = Finding::error(
        AssertionClass::Liveness,
        Some(node.id.clone()),
        format!(
            "command {} :{name} → `{command}` — exited {}",
            node.id, failure.code
        ),
    );
    if failure.output.is_empty() {
        finding
    } else {
        finding.detail(format!("captured output:\n{}", failure.output))
    }
}

/// The maximum bytes of a failed command's captured output folded into its finding
/// (§4.6); a runaway command's output cannot bloat the findings payload unboundedly. The
/// TAIL is kept — the error a command dies on is almost always its last output.
const CAPTURED_OUTPUT_CAP: usize = 4096;

/// A non-zero `--run-commands` execution result (§4.6): the exit code plus the child's
/// captured, interleaved stdout+stderr.
struct CommandFailure {
    code: i32,
    output: String,
}

/// Execute `command` from the repo root via `sh -c`, returning `Some(CommandFailure)` on a
/// non-zero exit (or code `-1` when a signal killed it) and `None` on success. The child's
/// stdout AND stderr are CAPTURED (never inherited) so nothing the command prints reaches
/// the engine's stdout — the §5.3 single-JSON-object contract holds under `--json`; the
/// captured bytes fold into the finding instead. stdin is `/dev/null` so a command that
/// reads input cannot hang the check. Spawn failure is an internal error (§5.3 exit 3). NO
/// sandbox: the command runs with the engine's full environment, scoped only to the repo
/// root as its cwd (§4.6).
fn execute_command(root: &Path, command: &str) -> Result<Option<CommandFailure>> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| SteleError::internal(format!("run `sh -c {command:?}`: {e}")))?;
    if output.status.success() {
        return Ok(None);
    }
    Ok(Some(CommandFailure {
        code: output.status.code().unwrap_or(-1),
        output: fold_captured(&output.stdout, &output.stderr),
    }))
}

/// Interleave a child's captured stdout then stderr into one display string (§4.6),
/// trimmed and capped to the trailing [`CAPTURED_OUTPUT_CAP`] bytes so a noisy command
/// cannot bloat the finding.
fn fold_captured(stdout: &[u8], stderr: &[u8]) -> String {
    let mut out = String::new();
    for stream in [stdout, stderr] {
        let text = String::from_utf8_lossy(stream);
        let text = text.trim();
        if !text.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    if out.len() > CAPTURED_OUTPUT_CAP {
        let start = out.len() - CAPTURED_OUTPUT_CAP;
        // Snap to a char boundary so the truncation never splits a UTF-8 sequence.
        let start = (start..out.len())
            .find(|&i| out.is_char_boundary(i))
            .unwrap_or(out.len());
        out = format!("…{}", &out[start..]);
    }
    out
}

/// Runner manifests detected at the repo root (§4.6). `Some(set)` means the runner is
/// present (its manifest exists) with that set of resolvable task/script/recipe names;
/// `None` means absent — a command naming that runner falls back to PATH resolution.
/// Builtin subcommands (mix, cargo) are static ([`MIX_BUILTINS`]/[`CARGO_BUILTINS`]);
/// the sets carry only the manifest-declared names (aliases, scripts, recipes).
struct Runners {
    /// `mix.exs` alias keys (§4.6), `None` when absent.
    mix: Option<BTreeSet<String>>,
    /// Root `package.json` `scripts` keys (§4.6), `None` when absent.
    node_scripts: Option<BTreeSet<String>>,
    /// `.cargo/config.toml` `[alias]` keys (§4.6); `Some` (possibly empty) iff
    /// `Cargo.toml` exists at the root, `None` otherwise.
    cargo_aliases: Option<BTreeSet<String>>,
    /// `justfile` recipe names (§4.6), `None` when absent.
    just_recipes: Option<BTreeSet<String>>,
}

impl Runners {
    fn detect(root: &Path) -> Self {
        Self {
            mix: read_manifest(root, "mix.exs").map(|s| parse_mix_aliases(&s)),
            node_scripts: read_manifest(root, "package.json").map(|s| parse_npm_scripts(&s)),
            cargo_aliases: read_manifest(root, "Cargo.toml").map(|_| parse_cargo_aliases(root)),
            just_recipes: just_source(root).map(|s| parse_just_recipes(&s)),
        }
    }
}

/// A repo-root manifest's text, or `None` when it does not exist (a read error on an
/// existing file degrades to "present but empty" via the caller's parse, not to absent).
fn read_manifest(root: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(root.join(name)).ok()
}

/// The justfile source, honoring the two common casings (`justfile`, `Justfile`).
fn just_source(root: &Path) -> Option<String> {
    read_manifest(root, "justfile").or_else(|| read_manifest(root, "Justfile"))
}

/// Resolve one command segment's executable (§4.6), returning `None` when it resolves
/// and `Some(reason)` when it does not. Leading `VAR=value` assignments are skipped; the
/// first remaining token is the executable. A DETECTED runner name (mix/cargo/just/
/// npm/pnpm/yarn) resolves through its task list; every other first token resolves on
/// PATH or as a repo-relative executable file. Arguments past the subcommand are NOT
/// resolved (§4.6).
fn resolve_segment(
    root: &Path,
    path_env: Option<&std::ffi::OsStr>,
    runners: &Runners,
    segment: &str,
) -> Option<&'static str> {
    let tokens = tokenize(segment);
    let words = skip_env_assignments(&tokens);
    let first = words.first()?.as_str();
    // Rung (a): a shell builtin resolves unconditionally — it has no executable file to
    // find on PATH, so PATH resolution would false-flag it. `cd` in particular ships as
    // `/usr/bin/cd` on macOS but is builtin-only on Linux, which made resolution
    // platform-dependent (a false liveness finding on CI hosts only) before this rung.
    if SHELL_BUILTINS.contains(&first) {
        return None;
    }
    let sub = words.get(1).map(String::as_str);

    match (first, runners) {
        (
            "mix",
            Runners {
                mix: Some(aliases), ..
            },
        ) => match sub {
            Some(task) if !MIX_BUILTINS.contains(&task) && !aliases.contains(task) => {
                Some("task not found in mix.exs")
            }
            _ => None,
        },
        (
            "cargo",
            Runners {
                cargo_aliases: Some(aliases),
                ..
            },
        ) => match sub {
            Some(cmd) if !CARGO_BUILTINS.contains(&cmd) && !aliases.contains(cmd) => {
                Some("subcommand not found (not a cargo builtin or .cargo/config.toml alias)")
            }
            _ => None,
        },
        (
            "just",
            Runners {
                just_recipes: Some(recipes),
                ..
            },
        ) => match sub {
            Some(recipe) if !recipes.contains(recipe) => Some("recipe not found in justfile"),
            _ => None,
        },
        (
            "npm" | "pnpm" | "yarn",
            Runners {
                node_scripts: Some(scripts),
                ..
            },
        ) => {
            // `<runner> run <script>` checks the scripts table; any other subcommand
            // (`npm ci`, `pnpm install`, a `pnpm <script>` shorthand) is treated as a
            // runner builtin and resolves leniently — the strict check is the `run` form,
            // which is what a stale script key trips (§4.6). This avoids false positives
            // on the many package-manager builtins we do not enumerate.
            match (sub, words.get(2).map(String::as_str)) {
                (Some("run"), Some(script)) if !scripts.contains(script) => {
                    Some("script not found in package.json")
                }
                _ => None,
            }
        }
        // Not a detected runner: PATH, then a repo-relative executable file.
        _ => {
            if resolves_as_executable(root, path_env, first) {
                None
            } else {
                Some("not found on PATH or as a repo-relative executable")
            }
        }
    }
}

/// Whether `first` names an executable reachable from the repo (§4.6): a path (contains
/// `/`) resolves as a repo-relative executable file; a bare name resolves on PATH or, as
/// a fallback, as a repo-relative executable file.
fn resolves_as_executable(root: &Path, path_env: Option<&std::ffi::OsStr>, first: &str) -> bool {
    if first.contains('/') {
        return is_executable_file(&root.join(first));
    }
    on_path(path_env, first) || is_executable_file(&root.join(first))
}

/// Whether `name` is an executable file in some `PATH` directory (§4.6). An unset PATH
/// resolves nothing.
fn on_path(path_env: Option<&std::ffi::OsStr>, name: &str) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| is_executable_file(&dir.join(name)))
}

/// Whether `path` is a regular file with an executable bit set. On non-Unix the bit is
/// unobservable, so any existing regular file counts (§4.6).
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

// ─── command tokenizer (§4.6) ──────────────────────────────────────────────────
//
// A deliberately small POSIX-flavored word splitter — NOT a shell. It honors single
// quotes (fully literal), double quotes (literal with `\` escaping the next char), and
// backslash escapes, and it splits on the four unquoted shell operators `&&`, `||`,
// `|`, `;` into independently-resolved segments. It does NOT expand variables, command
// substitutions, globs, tildes, or braces; it does not interpret redirections, `&`
// backgrounding, or subshell parentheses — those either land as ordinary tokens (never
// the first, which is all resolution reads) or, being unquoted, are simply not special.

/// Shell operators that split a command string into independently-resolved segments.
/// Two-character operators are matched before the one-character `|`.
fn split_segments(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            cur.push(c);
            if q == '"' && c == '\\' && i + 1 < chars.len() {
                cur.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\\' => {
                cur.push(c);
                if i + 1 < chars.len() {
                    cur.push(chars[i + 1]);
                    i += 1;
                }
            }
            '\'' | '"' => {
                cur.push(c);
                quote = Some(c);
            }
            ';' => flush_segment(&mut cur, &mut segments),
            '&' if chars.get(i + 1) == Some(&'&') => {
                flush_segment(&mut cur, &mut segments);
                i += 1;
            }
            '|' if chars.get(i + 1) == Some(&'|') => {
                flush_segment(&mut cur, &mut segments);
                i += 1;
            }
            '|' => flush_segment(&mut cur, &mut segments),
            _ => cur.push(c),
        }
        i += 1;
    }
    flush_segment(&mut cur, &mut segments);
    segments
}

/// Push the trimmed accumulated segment (dropping an empty one, e.g. a trailing `;`) and
/// reset the buffer.
fn flush_segment(cur: &mut String, segments: &mut Vec<String>) {
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    cur.clear();
}

/// Word-split one segment into tokens, resolving quotes and backslash escapes to their
/// literal content (§4.6). A quoted-empty word (`''`) still yields an empty token.
fn tokenize(segment: &str) -> Vec<String> {
    let chars: Vec<char> = segment.chars().collect();
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut cur));
                started = false;
            }
            i += 1;
            continue;
        }
        started = true;
        match c {
            '\\' => {
                if i + 1 < chars.len() {
                    cur.push(chars[i + 1]);
                    i += 1;
                }
            }
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        cur.push(chars[i + 1]);
                        i += 2;
                    } else {
                        cur.push(chars[i]);
                        i += 1;
                    }
                }
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    if started {
        tokens.push(cur);
    }
    tokens
}

/// Drop leading `VAR=value` environment assignments (§4.6); the returned slice begins at
/// the executable token (empty when the segment is assignments only).
fn skip_env_assignments(tokens: &[String]) -> &[String] {
    let skip = tokens
        .iter()
        .take_while(|token| is_env_assignment(token))
        .count();
    &tokens[skip..]
}

/// Whether a token is a leading `NAME=value` assignment (§4.6): a POSIX name
/// (`[A-Za-z_][A-Za-z0-9_]*`) followed by `=`.
fn is_env_assignment(token: &str) -> bool {
    let Some(eq) = token.find('=') else {
        return false;
    };
    let name = &token[..eq];
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ─── runner-manifest parsers (§4.6) ────────────────────────────────────────────

/// Extract `mix.exs` alias keys (§4.6) by a textual scan of the `aliases do … end`
/// block — the aliases are runtime Elixir data, so this reads the keys of the returned
/// keyword list rather than evaluating it. A key is `"quoted.key":` or `bareword:` at the
/// head of a list entry; deleting the `"ecto.reset":` line un-resolves `mix ecto.reset`
/// (EXAMPLE 8.6b).
fn parse_mix_aliases(source: &str) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    let Some(start) = source.find("aliases do") else {
        return aliases;
    };
    for line in source[start..].lines().skip(1) {
        let trimmed = line.trim();
        if trimmed == "end" {
            break;
        }
        if let Some(key) = mix_alias_key(trimmed) {
            aliases.insert(key);
        }
    }
    aliases
}

/// The alias key a `mix.exs` line declares, if any (§4.6): the `<key>` of a leading
/// `"key":` or `bareword:` entry. A line without a leading key (a bare list element,
/// `[`, a closing `]`) yields `None`.
fn mix_alias_key(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix('"') {
        let end = rest.find('"')?;
        rest[end + 1..]
            .trim_start()
            .starts_with(':')
            .then(|| rest[..end].to_string())
    } else {
        let colon = line.find(':')?;
        let key = &line[..colon];
        (!key.is_empty()
            && key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.'))
        .then(|| key.to_string())
    }
}

/// Extract the `scripts` object's keys from a root `package.json` (§4.6). Malformed JSON
/// or an absent/non-object `scripts` field yields the empty set.
fn parse_npm_scripts(source: &str) -> BTreeSet<String> {
    serde_json::from_str::<Value>(source)
        .ok()
        .and_then(|json| {
            json.get("scripts")
                .and_then(Value::as_object)
                .map(|scripts| scripts.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// Extract `[alias]` keys from `.cargo/config.toml` (or the extensionless `.cargo/config`)
/// (§4.6). Called only when `Cargo.toml` exists; a missing/malformed config yields the
/// empty set (builtins still resolve via [`CARGO_BUILTINS`]).
fn parse_cargo_aliases(root: &Path) -> BTreeSet<String> {
    for name in [".cargo/config.toml", ".cargo/config"] {
        let Some(text) = read_manifest(root, name) else {
            continue;
        };
        return toml::from_str::<toml::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("alias")
                    .and_then(toml::Value::as_table)
                    .map(|table| table.keys().cloned().collect())
            })
            .unwrap_or_default();
    }
    BTreeSet::new()
}

/// Extract recipe names from a justfile (§4.6): an unindented, non-comment line whose
/// name (up to its `:`) is a recipe head. `name := value` assignments and `set …`
/// directives are skipped; recipe parameters after the name are ignored.
fn parse_just_recipes(source: &str) -> BTreeSet<String> {
    let mut recipes = BTreeSet::new();
    for line in source.lines() {
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        if line[colon..].starts_with(":=") {
            continue;
        }
        let name = line[..colon].split_whitespace().next().unwrap_or("");
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            recipes.insert(name.to_string());
        }
    }
    recipes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P5 (F11/F12): an unreachable-watermark drift must NEVER be attributed to an
    /// uncommitted edit — the honest attribution is "history unavailable (shallow clone?)".
    #[test]
    fn staling_history_unavailable_line_is_honest() {
        let line = Staling::HistoryUnavailable.line("billing/refund-cap", true);
        assert_eq!(
            line,
            "staling commit: history unavailable (shallow clone?) — `stele blame billing/refund-cap`"
        );
        // Without the blame hint (the `stele blame` render), the pointer is dropped.
        assert_eq!(
            Staling::HistoryUnavailable.line("x/y", false),
            "staling commit: history unavailable (shallow clone?)"
        );
        // The lie the fix removes — the uncommitted phrasing — is reserved for a genuine
        // working-tree edit and must not surface for the history-unavailable case.
        assert!(!line.contains("uncommitted"));
    }

    /// The parser-less churn fallback's shallow-clone finding (F11) names the honest
    /// failure and points at the fetch-depth fix, never a silent pass.
    #[test]
    fn history_unavailable_finding_names_shallow_clone() {
        let graph = Graph::default();
        let node = Node {
            kind: NodeKind::System,
            id: "/".to_string(),
            purpose: None,
            commands: std::collections::BTreeMap::new(),
            invariants: Vec::new(),
            hazards: Vec::new(),
            edges: crate::model::Edges::default(),
            budget: None,
            source: std::path::PathBuf::from("AGENTS.md"),
            extracted_imports: Vec::new(),
            contains: Vec::new(),
        };
        let claim = Claim::authored(
            crate::model::ClaimKind::Invariant,
            "the documented rule holds".to_string(),
            "lm:doc-rule".to_string(),
            None,
            "doc-rule".to_string(),
        );
        let verified = LockVerified {
            digest: None,
            sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
        };
        let finding = history_unavailable_finding(&graph, &node, &claim, &verified, "notes.txt");
        assert!(
            finding
                .message
                .contains("history unavailable (shallow clone?)")
        );
        assert!(finding.details.iter().any(|d| d.contains("notes.txt")));
        assert!(
            finding
                .fix
                .as_deref()
                .is_some_and(|f| f.contains("fetch-depth: 0"))
        );
    }
}
