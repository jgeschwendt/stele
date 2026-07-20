//! Code-truth extraction and import-edge derivation (SPEC §2.3 `imports`, §4.2
//! territory attribution, §9 implementation notes).
//!
//! One [`Extractor`] per language, dispatched by file extension, turns a
//! VCS-tracked source tree into raw file→file references, then a single driver
//! ([`extract_imports`]) maps each endpoint to its owning node via the §4.2
//! territory index and records a node→node edge only when the two owners differ
//! (a same-node reference is not a boundary crossing). Resolution binds INTERNAL
//! boundaries only: a reference that resolves to no tracked file (stdlib, an
//! external hex/npm/cargo/pypi package) is silently ignored — the signature
//! governs the repo's own nodes, and `allow:` covers the honest remainder (§4.2).
//!
//! Each language resolves references its own way: Elixir binds a module reference
//! to the file whose `defmodule` declares it; TS/JS resolves relative import
//! specifiers by path-probing and bare specifiers against tracked `package.json`
//! `name` fields; Rust maps a `use`/`extern crate` head segment to the workspace
//! member (a tracked `Cargo.toml` `[package].name`) that owns it; Python maps a
//! dotted module to the tracked file that would define it.

use crate::model::{ImportRef, Result, SteleError, Territory};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Parser, Tree};

/// A raw reference: the importing file drew a dependency on the defining file,
/// both repo-root-relative POSIX paths, plus the 1-based line the reference sits on
/// and that line's trimmed source text (§4.2 structural locations). Node attribution
/// happens in the driver.
struct FileRef {
    from: String,
    to: String,
    line: usize,
    text: String,
}

/// One language's extraction: tracked sources of its kind → raw file→file
/// references. Registry dispatch is by file extension, internal to each impl.
trait Extractor {
    /// Every `(importing_file, defining_file)` reference this language contributes,
    /// resolved to tracked files (unresolvable references dropped, §4.2).
    fn extract(&self, ctx: &Ctx) -> Result<Vec<FileRef>>;
}

/// The tracked-file view every extractor reads: repo root, the sorted POSIX path
/// list, and a membership set for O(1) resolution probes.
struct Ctx<'a> {
    root: &'a Path,
    files: Vec<String>,
    tracked: HashSet<String>,
}

impl<'a> Ctx<'a> {
    fn new(root: &'a Path, tracked: &[PathBuf]) -> Self {
        let files: Vec<String> = tracked.iter().map(|p| posix(p)).collect();
        let set = files.iter().cloned().collect();
        Self {
            root,
            files,
            tracked: set,
        }
    }

    /// Tracked files whose lowercased extension is one of `exts` (no dot), in the
    /// sorted order `tracked` arrived — deterministic iteration for every build.
    fn with_ext<'b>(&'b self, exts: &'b [&'b str]) -> impl Iterator<Item = &'b String> {
        self.files
            .iter()
            .filter(move |f| ext_of(f).is_some_and(|e| exts.contains(&e.as_str())))
    }

    /// Tracked files whose basename equals `name` (e.g. `Cargo.toml`,
    /// `package.json`), sorted.
    fn with_name<'b>(&'b self, name: &'b str) -> impl Iterator<Item = &'b String> {
        self.files.iter().filter(move |f| basename(f) == name)
    }

    fn read(&self, rel: &str) -> Result<String> {
        std::fs::read_to_string(self.root.join(rel))
            .map_err(|e| SteleError::internal(format!("read {rel}: {e}")))
    }

    fn is_tracked(&self, rel: &str) -> bool {
        self.tracked.contains(rel)
    }
}

/// The result of import extraction (§2.3/§4.2): the per-node de-duplicated target
/// ids that feed `extracted.imports` in the lock, plus the per-edge reference
/// occurrences the structural class (§4.2) prints as violation locations. The
/// occurrences are in-memory only — the lock never carries them.
pub struct Extraction {
    /// Each node id → the sorted, de-duplicated ids of the OTHER nodes it imports.
    pub per_node: BTreeMap<String, Vec<String>>,
    /// Each `(from node id, to node id)` cross-node edge → its contributing
    /// reference occurrences, sorted and de-duplicated.
    pub edges: BTreeMap<(String, String), Vec<ImportRef>>,
}

/// Extract `imports` per node (§2.2): each node id → the sorted, de-duplicated ids
/// of the OTHER nodes it imports across a territory boundary (§4.2), plus the
/// per-edge reference occurrences (§4.2 structural locations). Runs on both `build`
/// and `check`, and only `per_node` reaches the lock, so the lock byte-matches on
/// both paths. A reference whose endpoints share an owner, or whose target resolves
/// outside every node's territory, is dropped.
pub fn extract_imports(
    root: &Path,
    tracked: &[PathBuf],
    territory: &Territory,
) -> Result<Extraction> {
    let ctx = Ctx::new(root, tracked);
    let extractors: [&dyn Extractor; 4] = [&Elixir, &TsJs, &RustLang, &Python];
    let mut per_node: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut edges: BTreeMap<(String, String), BTreeSet<ImportRef>> = BTreeMap::new();
    for extractor in extractors {
        for reference in extractor.extract(&ctx)? {
            let (Some(from), Some(to)) = (
                territory.owner(&reference.from),
                territory.owner(&reference.to),
            ) else {
                continue;
            };
            if from != to {
                let (from, to) = (from.to_string(), to.to_string());
                per_node.entry(from.clone()).or_default().insert(to.clone());
                edges.entry((from, to)).or_default().insert(ImportRef {
                    file: reference.from,
                    line: reference.line,
                    text: reference.text,
                });
            }
        }
    }
    Ok(Extraction {
        per_node: per_node
            .into_iter()
            .map(|(id, imports)| (id, imports.into_iter().collect()))
            .collect(),
        edges: edges
            .into_iter()
            .map(|(edge, occurrences)| (edge, occurrences.into_iter().collect()))
            .collect(),
    })
}

// ─── Elixir ───────────────────────────────────────────────────────────────────

/// Elixir extraction (§9): a module reference — an `alias` node anywhere in the
/// tree, covering `alias`/`import`/`use`/`require` directive targets and every
/// fully-qualified call receiver — resolves against the project's
/// `defmodule`-name → file map. `def`/`defmodule` are macro calls, so this is
/// tree-sitter over `call`/`alias` nodes, not a definition-kind scan.
struct Elixir;

impl Extractor for Elixir {
    fn extract(&self, ctx: &Ctx) -> Result<Vec<FileRef>> {
        const EXTS: [&str; 2] = ["ex", "exs"];
        let mut parser = Parser::new();
        let language: Language = tree_sitter_elixir::LANGUAGE.into();

        // First pass: parse every file once, and build the module→file map from
        // each `defmodule`. First declarer wins (files arrive sorted).
        let mut modules: BTreeMap<String, String> = BTreeMap::new();
        let mut parsed: Vec<(&String, String, Tree)> = Vec::new();
        for file in ctx.with_ext(&EXTS) {
            let src = ctx.read(file)?;
            let tree = parse(&mut parser, &language, &src, file)?;
            collect_defmodules(tree.root_node(), src.as_bytes(), &mut |name| {
                modules.entry(name).or_insert_with(|| file.clone());
            });
            parsed.push((file, src, tree));
        }

        // Second pass: resolve every module reference against the map.
        let mut refs = Vec::new();
        for (file, src, tree) in &parsed {
            let mut aliases = Vec::new();
            collect_kind_occurrences(tree.root_node(), src.as_bytes(), "alias", &mut aliases);
            for (alias, row) in aliases {
                if let Some(defining) = modules.get(&alias) {
                    refs.push(FileRef {
                        from: (*file).clone(),
                        to: defining.clone(),
                        line: row + 1,
                        text: source_line(src, row),
                    });
                }
            }
        }
        Ok(refs)
    }
}

/// Invoke `sink` with the module name of every `defmodule NAME do` in the tree:
/// a `call` whose leading identifier is `defmodule` and whose `arguments` hold an
/// `alias`.
fn collect_defmodules(node: Node, src: &[u8], sink: &mut impl FnMut(String)) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call"
            && let Some(name) = defmodule_name(child, src)
        {
            sink(name);
        }
        collect_defmodules(child, src, sink);
    }
}

/// The module name of a `defmodule` call, or `None` for any other call.
fn defmodule_name(call: Node, src: &[u8]) -> Option<String> {
    let mut cursor = call.walk();
    let children: Vec<Node> = call.children(&mut cursor).collect();
    let leads_defmodule = children
        .iter()
        .find(|c| c.kind() == "identifier")
        .and_then(|c| c.utf8_text(src).ok())
        == Some("defmodule");
    if !leads_defmodule {
        return None;
    }
    let args = children.iter().find(|c| c.kind() == "arguments")?;
    let mut arg_cursor = args.walk();
    let alias = args
        .children(&mut arg_cursor)
        .find(|c| c.kind() == "alias")?;
    alias.utf8_text(src).ok().map(str::to_string)
}

// ─── TypeScript / JavaScript ──────────────────────────────────────────────────

/// TS/JS extraction (§9): `import`/`export … from` sources and `require(…)` calls
/// are resolved either by relative path-probing (`./x`, `../x` against
/// `.ts/.tsx/.js/.jsx/.cjs/.mjs` and `index.*`) or, for a bare specifier, against
/// the workspace package-name map built from tracked `package.json` `name` fields.
struct TsJs;

/// A tracked `package.json` `name` field and its resolved main file (§9 workspace
/// map). Unknown fields are ignored — only `name`/`main` steer resolution.
#[derive(Deserialize)]
struct PackageJson {
    main: Option<String>,
    name: Option<String>,
}

/// A workspace package: its declared name, its directory, and its resolved main
/// file (when `package.json` names one that a probe binds to a tracked file).
struct Package {
    dir: String,
    main: Option<String>,
    name: String,
}

impl Extractor for TsJs {
    fn extract(&self, ctx: &Ctx) -> Result<Vec<FileRef>> {
        const EXTS: [&str; 6] = ["cjs", "js", "jsx", "mjs", "ts", "tsx"];
        let packages = workspace_packages(ctx)?;

        let mut parser = Parser::new();
        let mut refs = Vec::new();
        for file in ctx.with_ext(&EXTS) {
            let src = ctx.read(file)?;
            let language = ts_language(&ext_of(file).unwrap_or_default());
            let tree = parse(&mut parser, &language, &src, file)?;
            let mut sources = Vec::new();
            collect_ts_sources(tree.root_node(), src.as_bytes(), &mut sources);
            for (source, row) in sources {
                if let Some(to) = resolve_ts(ctx, file, &source, &packages) {
                    refs.push(FileRef {
                        from: file.clone(),
                        to,
                        line: row + 1,
                        text: source_line(&src, row),
                    });
                }
            }
        }
        Ok(refs)
    }
}

/// Build the workspace package map from every tracked `package.json` (§9). A
/// package with no `name` is skipped; `main` is kept only when it path-probes to
/// a tracked file.
fn workspace_packages(ctx: &Ctx) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for file in ctx.with_name("package.json") {
        let src = ctx.read(file)?;
        let Ok(parsed) = serde_json::from_str::<PackageJson>(&src) else {
            continue;
        };
        let Some(name) = parsed.name else {
            continue;
        };
        let dir = parent_dir(file).to_string();
        let main = parsed.main.and_then(|m| probe_ts(ctx, &join(&dir, &m)));
        packages.push(Package { dir, main, name });
    }
    Ok(packages)
}

/// Collect every module specifier paired with its 0-based line: the `string` source
/// of an `import`/`export … from` statement and the first string argument of a
/// `require(…)` call.
fn collect_ts_sources(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    match node.kind() {
        "export_statement" | "import_statement" => {
            if let Some(source) = string_child(node, src) {
                out.push((source, node.start_position().row));
            }
        }
        "call_expression" => {
            if let Some(source) = require_source(node, src) {
                out.push((source, node.start_position().row));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ts_sources(child, src, out);
    }
}

/// The specifier of a `require("…")` call, or `None` for any other call.
fn require_source(call: Node, src: &[u8]) -> Option<String> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "identifier" || function.utf8_text(src).ok() != Some("require") {
        return None;
    }
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    let string = args.children(&mut cursor).find(|c| c.kind() == "string")?;
    string_text(string, src)
}

/// The first `string` child's literal value (a direct `from "…"` source).
fn string_child(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let string = node.children(&mut cursor).find(|c| c.kind() == "string")?;
    string_text(string, src)
}

/// A `string` node's value: its `string_fragment` text, or the empty string when
/// the literal has no fragment (`""`).
fn string_text(string: Node, src: &[u8]) -> Option<String> {
    let mut cursor = string.walk();
    match string
        .children(&mut cursor)
        .find(|c| c.kind() == "string_fragment")
    {
        Some(fragment) => fragment.utf8_text(src).ok().map(str::to_string),
        None => Some(String::new()),
    }
}

/// Resolve a TS/JS specifier to a tracked file (§9), or `None` when it is external
/// (a bare specifier naming no workspace package) or unresolvable.
fn resolve_ts(ctx: &Ctx, from: &str, source: &str, packages: &[Package]) -> Option<String> {
    if source.starts_with('.') {
        return probe_ts(ctx, &join(parent_dir(from), source));
    }
    for package in packages {
        if source == package.name {
            return package
                .main
                .clone()
                .or_else(|| probe_ts(ctx, &join(&package.dir, "index")));
        }
        if let Some(subpath) = source.strip_prefix(&format!("{}/", package.name)) {
            return probe_ts(ctx, &join(&package.dir, subpath));
        }
    }
    None
}

/// Path-probe a module target (an extensionless or exact path) against the tracked
/// set: the exact path, then `target.<ext>`, then `target/index.<ext>` (§9).
fn probe_ts(ctx: &Ctx, target: &str) -> Option<String> {
    const EXTS: [&str; 6] = ["ts", "tsx", "js", "jsx", "cjs", "mjs"];
    if ctx.is_tracked(target) {
        return Some(target.to_string());
    }
    for ext in EXTS {
        let candidate = format!("{target}.{ext}");
        if ctx.is_tracked(&candidate) {
            return Some(candidate);
        }
    }
    for ext in EXTS {
        let candidate = format!("{target}/index.{ext}");
        if ctx.is_tracked(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// The tree-sitter grammar for a TS/JS extension: `.tsx` needs the TSX dialect,
/// `.ts` the plain TypeScript one, every other extension the JavaScript grammar.
fn ts_language(ext: &str) -> Language {
    match ext {
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    }
}

// ─── Rust ─────────────────────────────────────────────────────────────────────

/// Rust extraction (§9): the head segment of a `use` path or an `extern crate`
/// name is resolved against the workspace-member map (tracked `Cargo.toml`
/// `[package].name`, hyphens folded to underscores to match the code identifier).
/// `crate`/`self`/`super` are intra-crate and never cross a node boundary.
struct RustLang;

/// A tracked `Cargo.toml`'s `[package]` (§9). Unknown fields are ignored.
#[derive(Deserialize)]
struct CargoToml {
    package: Option<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
}

impl Extractor for RustLang {
    fn extract(&self, ctx: &Ctx) -> Result<Vec<FileRef>> {
        // Crate identifier (underscored) → the member directory that defines it.
        let mut crates: BTreeMap<String, String> = BTreeMap::new();
        for file in ctx.with_name("Cargo.toml") {
            let src = ctx.read(file)?;
            if let Ok(CargoToml {
                package: Some(package),
            }) = toml::from_str::<CargoToml>(&src)
            {
                crates
                    .entry(package.name.replace('-', "_"))
                    .or_insert_with(|| parent_dir(file).to_string());
            }
        }

        let mut parser = Parser::new();
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut refs = Vec::new();
        for file in ctx.with_ext(&["rs"]) {
            let src = ctx.read(file)?;
            let tree = parse(&mut parser, &language, &src, file)?;
            let mut heads = Vec::new();
            collect_rust_crate_heads(tree.root_node(), src.as_bytes(), &mut heads);
            for (head, row) in heads {
                if let Some(dir) = crates.get(&head) {
                    refs.push(FileRef {
                        from: file.clone(),
                        to: dir.clone(),
                        line: row + 1,
                        text: source_line(&src, row),
                    });
                }
            }
        }
        Ok(refs)
    }
}

/// Collect the head crate segment of every `use` declaration and `extern crate`
/// paired with its 0-based line, dropping the intra-crate roots
/// `crate`/`self`/`super`.
fn collect_rust_crate_heads(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    match node.kind() {
        "use_declaration" => {
            if let Some(head) = use_head_segment(node, src) {
                out.push((head, node.start_position().row));
            }
        }
        "extern_crate_declaration" => {
            if let Some(head) = extern_crate_name(node, src) {
                out.push((head, node.start_position().row));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_crate_heads(child, src, out);
    }
}

/// The head segment of a `use` path (`foo` in `use foo::bar::Baz`), or `None` for
/// an intra-crate root.
fn use_head_segment(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let path = node.children(&mut cursor).find(|c| {
        !matches!(
            c.kind(),
            "use" | ";" | "attribute_item" | "visibility_modifier"
        )
    })?;
    let head = first_path_segment(path.utf8_text(src).ok()?);
    if matches!(head, "crate" | "self" | "super" | "") {
        None
    } else {
        Some(head.to_string())
    }
}

/// The crate name of an `extern crate NAME [as ALIAS]` — the first identifier.
fn extern_crate_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| c.kind() == "identifier")
        .and_then(|c| c.utf8_text(src).ok())
        .map(str::to_string)
}

/// The leading path segment of a Rust use-path text (`crate::model::Node` →
/// `crate`, `a::{b, c}` → `a`), leading `::` stripped.
fn first_path_segment(text: &str) -> &str {
    text.trim_start_matches(':')
        .split(|c: char| c == ':' || c == '{' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim()
}

// ─── Python ───────────────────────────────────────────────────────────────────

/// Python extraction (§9): a dotted module from an `import a.b.c` or
/// `from a.b import x` resolves against the tracked-file map (`a/b/c.py` →
/// `a.b.c`, `a/b/__init__.py` → `a.b`). Relative imports (`from . import x`) carry
/// no dotted module and are left to `allow:`.
struct Python;

impl Extractor for Python {
    fn extract(&self, ctx: &Ctx) -> Result<Vec<FileRef>> {
        let files: Vec<&String> = ctx.with_ext(&["py"]).collect();
        // Dotted module → the tracked file that defines it (first declarer wins).
        let mut modules: BTreeMap<String, String> = BTreeMap::new();
        for file in &files {
            modules
                .entry(python_module_name(file))
                .or_insert_with(|| (*file).clone());
        }

        let mut parser = Parser::new();
        let language: Language = tree_sitter_python::LANGUAGE.into();
        let mut refs = Vec::new();
        for file in &files {
            let src = ctx.read(file)?;
            let tree = parse(&mut parser, &language, &src, file)?;
            let mut dotted = Vec::new();
            collect_python_modules(tree.root_node(), src.as_bytes(), &mut dotted);
            for (module, row) in dotted {
                if let Some(defining) = modules.get(&module) {
                    refs.push(FileRef {
                        from: (*file).clone(),
                        to: defining.clone(),
                        line: row + 1,
                        text: source_line(&src, row),
                    });
                }
            }
        }
        Ok(refs)
    }
}

/// The dotted module a tracked `.py` file would define: `a/b/c.py` → `a.b.c`,
/// `a/b/__init__.py` → `a.b`.
fn python_module_name(path: &str) -> String {
    let stem = path.strip_suffix(".py").unwrap_or(path);
    let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
    stem.replace('/', ".")
}

/// Collect the dotted module of every `import` and `from`-`import` paired with its
/// 0-based line: each `dotted_name` (or `aliased_import`) under an
/// `import_statement`, and the `module_name` of a `from`-import when it is dotted
/// (relative imports skipped).
fn collect_python_modules(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        if let Ok(text) = child.utf8_text(src) {
                            out.push((text.to_string(), child.start_position().row));
                        }
                    }
                    "aliased_import" => {
                        if let Some(name) = child.child_by_field_name("name")
                            && let Ok(text) = name.utf8_text(src)
                        {
                            out.push((text.to_string(), child.start_position().row));
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" => {
            if let Some(module) = node.child_by_field_name("module_name")
                && module.kind() == "dotted_name"
                && let Ok(text) = module.utf8_text(src)
            {
                out.push((text.to_string(), module.start_position().row));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_modules(child, src, out);
    }
}

// ─── shared helpers ───────────────────────────────────────────────────────────

/// Parse `src` with `language`, mapping a grammar-load or parse failure to a §5.3
/// internal error (exit 3) — the extractor crashing, never a repo-out-of-spec case.
fn parse(parser: &mut Parser, language: &Language, src: &str, file: &str) -> Result<Tree> {
    parser
        .set_language(language)
        .map_err(|e| SteleError::internal(format!("load grammar for {file}: {e}")))?;
    parser
        .parse(src, None)
        .ok_or_else(|| SteleError::internal(format!("tree-sitter failed to parse {file}")))
}

/// Collect the text of every node of kind `kind` in the tree paired with its 0-based
/// line — one entry per occurrence, so the structural class can locate each reference.
fn collect_kind_occurrences(node: Node, src: &[u8], kind: &str, out: &mut Vec<(String, usize)>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind
            && let Ok(text) = child.utf8_text(src)
        {
            out.push((text.to_string(), child.start_position().row));
        }
        collect_kind_occurrences(child, src, kind, out);
    }
}

/// The trimmed source text of the line at 0-based `row` — a reference's own line
/// (e.g. `alias AcmeWeb.Billing.Charge`), used verbatim in structural locations (§4.2).
fn source_line(src: &str, row: usize) -> String {
    src.lines().nth(row).unwrap_or("").trim().to_string()
}

/// A repo-root-relative path as a POSIX string (forward slashes on every platform).
fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A POSIX path's lowercased final extension without the dot, or `None`.
fn ext_of(path: &str) -> Option<String> {
    let name = basename(path);
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some(name[dot + 1..].to_ascii_lowercase())
}

/// A POSIX path's final segment.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A POSIX path's parent directory (`""` for a root-level path).
fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// Lexically join a relative path onto a base directory, collapsing `.`/`..`
/// segments (no filesystem access): `join("packages/shared/test", "../src/money")`
/// → `packages/shared/src/money`.
fn join(base: &str, rel: &str) -> String {
    let mut stack: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for segment in rel.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_collapses_dot_and_parent_segments() {
        assert_eq!(
            join("packages/shared/test", "../src/money"),
            "packages/shared/src/money"
        );
        assert_eq!(join("a/b", "./c"), "a/b/c");
        assert_eq!(join("", "pkg/index"), "pkg/index");
    }

    #[test]
    fn ext_and_basename_ignore_dotfiles_and_dirs() {
        assert_eq!(ext_of("a/b/money.test.ts").as_deref(), Some("ts"));
        assert_eq!(ext_of("a/b/Makefile"), None);
        assert_eq!(ext_of("a/.gitignore"), None);
        assert_eq!(basename("a/b/c.rs"), "c.rs");
        assert_eq!(parent_dir("a/b/c.rs"), "a/b");
        assert_eq!(parent_dir("top.rs"), "");
    }

    #[test]
    fn python_module_name_maps_files_and_packages() {
        assert_eq!(python_module_name("a/b/c.py"), "a.b.c");
        assert_eq!(python_module_name("a/b/__init__.py"), "a.b");
        assert_eq!(python_module_name("top.py"), "top");
    }

    #[test]
    fn first_path_segment_takes_the_crate_head() {
        assert_eq!(first_path_segment("crate::model::Node"), "crate");
        assert_eq!(first_path_segment("foo::bar::Baz"), "foo");
        assert_eq!(first_path_segment("a::{b, c}"), "a");
        assert_eq!(first_path_segment("::external::thing"), "external");
    }
}
