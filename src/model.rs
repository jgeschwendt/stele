//! Graph model: node kinds, schema, edge vocabulary (SPEC §2), plus the shared
//! error type and exit-code discipline (SPEC §5.3) that every phase reuses.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// SPEC §2.2: `purpose` is capped at 200 characters (hard, §10 item 4).
pub const PURPOSE_MAX_CHARS: usize = 200;

/// SPEC §2.4/§2.5: the `lm:` anchor namespace whose remainder is the landmark slug verbatim.
pub const LANDMARK_ANCHOR_PREFIX: &str = "lm:";
/// SPEC §2.1: the system node's id — the sole non-relative id.
pub const SYSTEM_ID: &str = "/";

// ─── errors & exit discipline (§5.3) ────────────────────────────────────────

/// SPEC §5.3 non-zero terminations. `0` (success) is the absence of an error, so
/// it has no variant here. Discriminants ARE the process exit codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitCode {
    /// `1` — assertion failure ("repo out of spec"); Phase D populates it.
    Assertion = 1,
    /// `2` — input error: malformed block, duplicate id, unknown lock version, bad flags.
    Input = 2,
    /// `3` — internal error: IO, tree-sitter/extractor crash.
    Internal = 3,
}

/// A terminating error carrying the §5.3 exit class plus, where known, the
/// offending `file:line` the process contract requires build to print.
#[derive(Clone, Debug)]
pub struct SteleError {
    pub exit: ExitCode,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub message: String,
}

impl SteleError {
    /// An input error (exit 2) anchored at a concrete `file:line`.
    pub fn input(file: impl AsRef<Path>, line: usize, message: impl Into<String>) -> Self {
        Self {
            exit: ExitCode::Input,
            file: Some(file.as_ref().to_path_buf()),
            line: Some(line),
            message: message.into(),
        }
    }

    /// An input error (exit 2) with no line context (bad flags, missing subcommand).
    pub fn input_msg(message: impl Into<String>) -> Self {
        Self {
            exit: ExitCode::Input,
            file: None,
            line: None,
            message: message.into(),
        }
    }

    /// An internal error (exit 3): IO failure or a would-be panic.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            exit: ExitCode::Internal,
            file: None,
            line: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for SteleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.file, self.line) {
            (Some(p), Some(l)) => write!(f, "{}:{}: {}", p.display(), l, self.message),
            (Some(p), None) => write!(f, "{}: {}", p.display(), self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for SteleError {}

/// Result specialized to [`SteleError`]; the shared return type across phases.
pub type Result<T> = std::result::Result<T, SteleError>;

// ─── the typed graph (§2) ───────────────────────────────────────────────────

/// Node kinds. §2.1 primary altitudes plus the auxiliary kinds later phases
/// compile (never authored in a `stele` block).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Adr,
    Anchor,
    Component,
    Container,
    System,
}

impl NodeKind {
    /// Lowercase wire form (the lock's `kind` string, §3.2).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adr => "adr",
            Self::Anchor => "anchor",
            Self::Component => "component",
            Self::Container => "container",
            Self::System => "system",
        }
    }
}

/// Whether a claim is an invariant or a hazard — the lock's required discriminator
/// so the two round-trip through one `claims[]` array (§3.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimKind {
    Hazard,
    Invariant,
}

impl ClaimKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hazard => "hazard",
            Self::Invariant => "invariant",
        }
    }
}

/// A claim (§2.4): an authored invariant/hazard plus the slots later phases fill.
#[derive(Clone, Debug)]
pub struct Claim {
    pub kind: ClaimKind,
    /// The authored `claim:` prose; serialized as `text` in the lock (§3.2).
    pub text: String,
    pub anchor: String,
    pub enforced_by: Option<String>,
    /// The slug derived from `anchor` (§2.4): the claim's lock `id` and the tail of
    /// its `<node-id>/<slug>` address. Derived at build, never authored.
    pub slug: String,
    // ── compiled-only (§2.4/§4.5); filled by Phase C/D ──
    /// `file:line` recomputed every build (§2.4).
    pub resolved: Option<String>,
    /// `{sha, digest}` stamped by build (§4.5).
    pub verified: Option<Verified>,
}

impl Claim {
    /// Assemble an authored claim; compiled slots start empty. `slug` is the
    /// [`derive_slug`] output for `anchor` (§2.4).
    pub fn authored(
        kind: ClaimKind,
        text: String,
        anchor: String,
        enforced_by: Option<String>,
        slug: String,
    ) -> Self {
        Self {
            kind,
            text,
            anchor,
            enforced_by,
            slug,
            resolved: None,
            verified: None,
        }
    }
}

/// Freshness watermark (§4.5). `digest` is `None` only where the anchored file's
/// language has no bundled parser (§2.4).
#[derive(Clone, Debug)]
pub struct Verified {
    pub sha: String,
    pub digest: Option<String>,
}

/// A tolerated cross-boundary edge (§4.2). `reason` is mandatory and surfaced
/// verbatim in `check --report`.
#[derive(Clone, Debug)]
pub struct Allow {
    pub edge: String,
    pub reason: String,
}

/// Authored edges (§2.2/§2.3). `imports`/`contains` are derived elsewhere.
#[derive(Clone, Debug, Default)]
pub struct Edges {
    pub depends: Vec<String>,
    pub decided_by: Vec<String>,
    pub allow: Vec<Allow>,
}

/// A compiled node: authored fields (§2.2) plus derived slots later phases fill.
#[derive(Clone, Debug)]
pub struct Node {
    // ── authored (§2.2) ──
    pub kind: NodeKind,
    pub id: String,
    pub purpose: Option<String>,
    pub commands: BTreeMap<String, String>,
    pub invariants: Vec<Claim>,
    pub hazards: Vec<Claim>,
    pub edges: Edges,
    pub budget: Option<u64>,
    /// The AGENTS.md that declared this node (repo-root-relative); provenance for
    /// duplicate-id reporting (§2.1, Phase B2) and error context.
    pub source: PathBuf,
    // ── compiled-only (§2.2); filled by Phase C ──
    pub extracted_imports: Vec<String>,
    pub contains: Vec<String>,
}

/// The aggregated graph. B1 collects nodes; later phases add adrs/landmarks and
/// key nodes by id with duplicate detection.
#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
}

impl Graph {
    /// Add a node, rejecting an id already held (§2.1 duplicate-id gate). Ids are
    /// assumed already normalized ([`normalize_id`]); the error names BOTH declaring
    /// files and is a §5.3 input error (exit 2).
    pub fn insert(&mut self, node: Node) -> Result<()> {
        if let Some(existing) = self.nodes.iter().find(|n| n.id == node.id) {
            return Err(SteleError::input_msg(format!(
                "duplicate node id {:?}: declared by both {} and {}",
                node.id,
                existing.source.display(),
                node.source.display(),
            )));
        }
        self.nodes.push(node);
        Ok(())
    }

    /// The territory index (§4.2): a queryable owner-lookup over the node set.
    pub fn territory(&self) -> Territory {
        Territory::from_nodes(&self.nodes)
    }

    /// Resolve a claim address (§2.4). Accepts the full `<node-id>/<slug>` and the
    /// last-path-segment abbreviation `<segment>/<slug>` (resolved only when the
    /// segment names exactly one node across the graph).
    pub fn resolve_claim(&self, address: &str) -> ClaimLookup<'_> {
        let Some((node_part, slug)) = address.rsplit_once('/') else {
            return ClaimLookup::NotFound;
        };
        // The system id `/` splits to an empty node_part with the leading slash
        // consumed; restore it so `//<slug>` addresses the system node.
        let node_part = if node_part.is_empty() && address.starts_with('/') {
            SYSTEM_ID
        } else {
            node_part
        };

        // An exact id match wins outright (ids are unique after dedup).
        if let Some(node) = self.nodes.iter().find(|n| n.id == node_part) {
            return match find_claim(node, slug) {
                Some(claim) => ClaimLookup::Found(ClaimRef {
                    node_id: &node.id,
                    claim,
                }),
                None => ClaimLookup::NotFound,
            };
        }

        // Otherwise treat node_part as a final-segment abbreviation; it resolves only
        // when it names exactly one node (§2.4 "unambiguous across the graph").
        let mut matches = self
            .nodes
            .iter()
            .filter(|n| last_segment(&n.id) == node_part);
        let (Some(node), None) = (matches.next(), matches.next()) else {
            // Zero matches → NotFound; two or more → the abbreviation is ambiguous.
            return if self.nodes.iter().any(|n| last_segment(&n.id) == node_part) {
                ClaimLookup::Ambiguous
            } else {
                ClaimLookup::NotFound
            };
        };
        match find_claim(node, slug) {
            Some(claim) => ClaimLookup::Found(ClaimRef {
                node_id: &node.id,
                claim,
            }),
            None => ClaimLookup::NotFound,
        }
    }
}

/// A resolved claim plus the id of the node that declares it.
#[derive(Clone, Copy, Debug)]
pub struct ClaimRef<'a> {
    pub node_id: &'a str,
    pub claim: &'a Claim,
}

/// The outcome of [`Graph::resolve_claim`]. `Ambiguous` is distinct from `NotFound`
/// so callers (blame, `stele:claim` back-references) can report a bare abbreviation
/// that matches multiple nodes.
#[derive(Clone, Copy, Debug)]
pub enum ClaimLookup<'a> {
    Ambiguous,
    Found(ClaimRef<'a>),
    NotFound,
}

/// The territory attribution index (§4.2): maps each declaring directory (the parent
/// of an AGENTS.md, repo-root-relative; `""` for the repo root) to its node id. The
/// owner of a file is the node whose territory directory is its DEEPEST ancestor —
/// deepest-prefix wins, which is exactly "declared directory MINUS nested territories"
/// (a file inside a child's directory is owned by the child, never the parent).
#[derive(Debug, Default)]
pub struct Territory {
    // Sorted (declaring-dir, node-id); `""` is the repo root (system node).
    dirs: BTreeMap<String, String>,
}

impl Territory {
    fn from_nodes(nodes: &[Node]) -> Self {
        let mut dirs = BTreeMap::new();
        for node in nodes {
            dirs.insert(declaring_dir(node), node.id.clone());
        }
        Self { dirs }
    }

    /// The id of the node owning `file` (repo-root-relative), or `None` when no node's
    /// territory contains it (only possible when no system node was declared; a system
    /// node's `""` directory is an ancestor of every path). Non-inheriting: the
    /// deepest containing directory wins.
    pub fn owner(&self, file: &str) -> Option<&str> {
        let file = normalize_id(file).ok()?;
        let file = if file == SYSTEM_ID { "" } else { &file };
        self.dirs
            .iter()
            .filter(|(dir, _)| dir_contains(dir, file))
            .max_by_key(|(dir, _)| dir.len())
            .map(|(_, id)| id.as_str())
    }
}

/// A node's declaring directory (§4.2): the repo-root-relative parent of its source
/// AGENTS.md, `""` for the repo root.
fn declaring_dir(node: &Node) -> String {
    match node.source.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_string_lossy().replace('\\', "/"),
        _ => String::new(),
    }
}

/// Whether territory directory `dir` (`""` = root) contains repo-relative `file`
/// (equal, or a path-segment ancestor — `apps/web` contains `apps/web/x` but not
/// `apps/website`).
fn dir_contains(dir: &str, file: &str) -> bool {
    dir.is_empty() || file == dir || file.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// The final path segment of a node id, used for the §2.4 abbreviated claim address.
fn last_segment(id: &str) -> &str {
    if id == SYSTEM_ID {
        return id;
    }
    id.rsplit('/').next().unwrap_or(id)
}

/// The claim in `node` whose derived slug equals `slug` (§2.4 addressing).
fn find_claim<'a>(node: &'a Node, slug: &str) -> Option<&'a Claim> {
    node.invariants
        .iter()
        .chain(node.hazards.iter())
        .find(|c| c.slug == slug)
}

/// Id normalization (§2.1), applied before any comparison/dedup/lock write: strip a
/// leading `./`, normalize `\` → `/`, collapse repeated `/`, strip a trailing `/`,
/// reject `..` and OS-absolute paths. The system node's id is the single `/`.
pub fn normalize_id(raw: &str) -> std::result::Result<String, String> {
    let slashed = raw.replace('\\', "/");
    if slashed.starts_with('/') && slashed != "/" {
        return Err(format!(
            "id must be repo-root-relative, not absolute: {raw:?}"
        ));
    }
    let mut segments = Vec::new();
    for segment in slashed.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err(format!("id must not contain '..': {raw:?}")),
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        // Empty, "/", "./", "." all denote the system node.
        return Ok(SYSTEM_ID.to_string());
    }
    Ok(segments.join("/"))
}

/// Derive a claim's slug from its `anchor` (§2.4). An `lm:<slug>` anchor yields the
/// remainder verbatim; a `<path>#<symbol>` anchor yields `<symbol>` lowercased with
/// each maximal run of non-`[a-z0-9]` collapsed to a single `-` and leading/trailing
/// `-` stripped. Either way the result must satisfy the §2.5 slug lexeme
/// `[a-z0-9]+(-[a-z0-9]+)*`; a malformed slug is a §5.3 input error (exit 2).
pub fn derive_slug(anchor: &str) -> std::result::Result<String, String> {
    let slug = match anchor.strip_prefix(LANDMARK_ANCHOR_PREFIX) {
        Some(rest) => rest.to_string(),
        None => {
            // `<path>#<symbol>` — the symbol is the tail past the last `#`.
            let symbol = anchor.rsplit('#').next().unwrap_or(anchor);
            collapse_symbol(symbol)
        }
    };
    if is_valid_slug(&slug) {
        Ok(slug)
    } else {
        Err(format!(
            "anchor {anchor:?} derives the slug {slug:?}, which is not a valid slug \
             (§2.5: {SLUG_LEXEME})"
        ))
    }
}

/// The §2.5 slug lexeme, quoted in error messages.
const SLUG_LEXEME: &str = "[a-z0-9]+(-[a-z0-9]+)*";

/// Lowercase `symbol` and collapse every maximal run of non-`[a-z0-9]` to a single
/// `-`, stripping leading/trailing `-` (§2.4 `<path>#<symbol>` derivation).
fn collapse_symbol(symbol: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in symbol.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            pending_dash = false;
        } else {
            pending_dash = true;
        }
    }
    out
}

/// The §2.5 slug lexeme check: one or more `[a-z0-9]` groups joined by single `-`,
/// with no leading, trailing, or doubled hyphen.
fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.starts_with('-') || slug.ends_with('-') {
        return false;
    }
    let mut prev_dash = false;
    for c in slug.chars() {
        match c {
            'a'..='z' | '0'..='9' => prev_dash = false,
            '-' if !prev_dash => prev_dash = true,
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(anchor: &str) -> Claim {
        let slug = derive_slug(anchor).expect("valid anchor in fixture");
        Claim::authored(
            ClaimKind::Invariant,
            "prose".to_string(),
            anchor.to_string(),
            None,
            slug,
        )
    }

    fn node(id: &str, source: &str, claims: &[&str]) -> Node {
        Node {
            kind: NodeKind::Component,
            id: id.to_string(),
            purpose: None,
            commands: BTreeMap::new(),
            invariants: claims.iter().map(|a| claim(a)).collect(),
            hazards: Vec::new(),
            edges: Edges::default(),
            budget: None,
            source: PathBuf::from(source),
            extracted_imports: Vec::new(),
            contains: Vec::new(),
        }
    }

    /// The acme fixture's node shape: (id, declaring AGENTS.md).
    fn acme_graph() -> Graph {
        let shape = [
            ("/", "AGENTS.md"),
            ("apps/web", "apps/web/AGENTS.md"),
            ("apps/web/lib/billing", "apps/web/lib/billing/AGENTS.md"),
            ("apps/web/lib/store", "apps/web/lib/store/AGENTS.md"),
            ("apps/worker", "apps/worker/AGENTS.md"),
            ("packages/shared", "packages/shared/AGENTS.md"),
        ];
        let mut graph = Graph::default();
        for (id, source) in shape {
            graph.insert(node(id, source, &[])).unwrap();
        }
        graph
    }

    // ─── id normalization (§2.1) ─────────────────────────────────────────────

    #[test]
    fn normalize_id_strips_leading_dot_slash_and_trailing_slash() {
        assert_eq!(normalize_id("./apps/web/").unwrap(), "apps/web");
    }

    #[test]
    fn normalize_id_collapses_repeated_slashes() {
        assert_eq!(normalize_id("apps//web///lib").unwrap(), "apps/web/lib");
    }

    #[test]
    fn normalize_id_normalizes_backslashes() {
        assert_eq!(normalize_id("apps\\web\\lib").unwrap(), "apps/web/lib");
    }

    #[test]
    fn normalize_id_maps_root_forms_to_system_id() {
        for raw in ["", "/", ".", "./"] {
            assert_eq!(normalize_id(raw).unwrap(), SYSTEM_ID, "{raw:?}");
        }
    }

    #[test]
    fn normalize_id_rejects_parent_segments() {
        assert!(normalize_id("apps/../etc").is_err());
    }

    #[test]
    fn normalize_id_rejects_absolute_paths() {
        assert!(normalize_id("/etc/passwd").is_err());
    }

    // ─── claim slugs (§2.4/§2.5) ──────────────────────────────────────────────

    #[test]
    fn derive_slug_takes_landmark_remainder_verbatim() {
        assert_eq!(derive_slug("lm:refund-cap").unwrap(), "refund-cap");
    }

    #[test]
    fn derive_slug_collapses_path_symbol_anchor() {
        assert_eq!(derive_slug("refund.ex#changeset").unwrap(), "changeset");
        assert_eq!(derive_slug("money.ts#MoneyType").unwrap(), "moneytype");
    }

    #[test]
    fn derive_slug_collapses_non_alphanumeric_runs_to_single_hyphen() {
        assert_eq!(
            derive_slug("m.ex#handle_webhook!").unwrap(),
            "handle-webhook"
        );
        assert_eq!(derive_slug("m.ex#__MODULE__").unwrap(), "module");
    }

    #[test]
    fn derive_slug_rejects_malformed_landmark_slug() {
        for anchor in [
            "lm:Refund-Cap",
            "lm:refund--cap",
            "lm:-cap",
            "lm:cap-",
            "lm:",
        ] {
            assert!(derive_slug(anchor).is_err(), "{anchor:?}");
        }
    }

    #[test]
    fn derive_slug_rejects_symbol_with_no_alphanumerics() {
        assert!(derive_slug("m.ex#___").is_err());
    }

    // ─── duplicate-id gate (§2.1) ─────────────────────────────────────────────

    #[test]
    fn insert_rejects_duplicate_id_naming_both_files() {
        let mut graph = Graph::default();
        graph
            .insert(node("apps/web", "apps/web/AGENTS.md", &[]))
            .unwrap();
        let err = graph
            .insert(node("apps/web", "packages/shared/AGENTS.md", &[]))
            .unwrap_err();
        assert_eq!(err.exit, ExitCode::Input);
        assert!(
            err.message.contains("apps/web/AGENTS.md"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("packages/shared/AGENTS.md"),
            "{}",
            err.message
        );
    }

    // ─── territory attribution (§4.2), acme shape ─────────────────────────────

    #[test]
    fn territory_attributes_root_files_to_system_node() {
        let territory = acme_graph().territory();
        assert_eq!(territory.owner("README.md"), Some("/"));
        assert_eq!(territory.owner("mix.exs"), Some("/"));
    }

    #[test]
    fn territory_is_non_inheriting_child_owns_its_files() {
        let territory = acme_graph().territory();
        // A file inside billing belongs to billing, never the enclosing apps/web.
        assert_eq!(
            territory.owner("apps/web/lib/billing/refund.ex"),
            Some("apps/web/lib/billing")
        );
        assert_eq!(
            territory.owner("apps/web/lib/store/cart.ex"),
            Some("apps/web/lib/store")
        );
        // A file directly under apps/web, outside every child, belongs to apps/web.
        assert_eq!(territory.owner("apps/web/lib/router.ex"), Some("apps/web"));
    }

    #[test]
    fn territory_does_not_prefix_match_sibling_directories() {
        // `apps/worker` must not claim files under a hypothetical `apps/workerx`.
        let territory = acme_graph().territory();
        assert_eq!(territory.owner("apps/workerx/lib/x.ex"), Some("/"));
        assert_eq!(
            territory.owner("packages/shared/src/money.ts"),
            Some("packages/shared")
        );
    }

    // ─── claim address resolution (§2.4) ──────────────────────────────────────

    #[test]
    fn resolve_claim_by_full_node_id() {
        let mut graph = acme_graph();
        graph.nodes[2] = node(
            "apps/web/lib/billing",
            "apps/web/lib/billing/AGENTS.md",
            &["lm:refund-cap"],
        );
        let found = graph.resolve_claim("apps/web/lib/billing/refund-cap");
        assert!(matches!(
            found,
            ClaimLookup::Found(ClaimRef { node_id, claim })
                if node_id == "apps/web/lib/billing" && claim.slug == "refund-cap"
        ));
    }

    #[test]
    fn resolve_claim_by_unambiguous_abbreviation() {
        let mut graph = acme_graph();
        graph.nodes[2] = node(
            "apps/web/lib/billing",
            "apps/web/lib/billing/AGENTS.md",
            &["lm:refund-cap"],
        );
        // `billing/refund-cap` abbreviates the full id to its final segment.
        assert!(matches!(
            graph.resolve_claim("billing/refund-cap"),
            ClaimLookup::Found(ClaimRef { node_id, .. }) if node_id == "apps/web/lib/billing"
        ));
    }

    #[test]
    fn resolve_claim_abbreviation_is_ambiguous_across_collision() {
        // Two nodes share the final segment `core`; the abbreviation cannot resolve.
        let mut graph = Graph::default();
        graph
            .insert(node("apps/web/core", "apps/web/core/AGENTS.md", &["lm:x"]))
            .unwrap();
        graph
            .insert(node(
                "apps/worker/core",
                "apps/worker/core/AGENTS.md",
                &["lm:x"],
            ))
            .unwrap();
        assert!(matches!(
            graph.resolve_claim("core/x"),
            ClaimLookup::Ambiguous
        ));
        // The full id disambiguates.
        assert!(matches!(
            graph.resolve_claim("apps/web/core/x"),
            ClaimLookup::Found(_)
        ));
    }

    #[test]
    fn resolve_claim_missing_slug_is_not_found() {
        let graph = acme_graph();
        assert!(matches!(
            graph.resolve_claim("apps/web/nonexistent"),
            ClaimLookup::NotFound
        ));
    }

    // ─── intra-node duplicate slug (§2.4) via the parse layer is covered in parse.rs;
    //     here we assert the resolver never sees a duplicate because slugs are unique.
}
