//! The source pipeline (§5.1): node sources → in-memory graph with resolved anchors,
//! import edges, the ADR index (§2.6) and `verified` stamps — shared by `build` and `check`.

use super::{Mode, TREE_DIR, Workspace};
use crate::anchors::{self, SymbolResolution};
use crate::assert;
use crate::extract;
use crate::model::{AdrEntry, Graph, Node, Resolution, Result, SteleError, Verified};
use crate::parse;
use crate::steleignore::Steleignore;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One node source (§2.4/§3.5): `rel` is the repo-root-relative path fed to
/// [`parse::parse_agents_file`] (so node id and territory come out identical in both
/// modes), `source` is the absolute path the bytes are read from. In normal mode the two
/// name the same tracked file; in undercover mode `rel` is the mirror path (the
/// `.stele/tree/` prefix stripped) and `source` is the overlay file at the graph home.
pub(super) struct NodeSource {
    pub(super) rel: PathBuf,
    pub(super) source: PathBuf,
}

/// The node-source roster for one build (§2.4/§3.5), mode-aware. Normal: every tracked
/// `AGENTS.md`, `rel` the tracked path and `source` under `ws.work` — today's behavior.
/// Undercover: a sorted recursive filesystem walk of `<home>/.stele/tree/**/AGENTS.md`
/// (never `git ls-files`, never `.steleignore`-filtered — these are authored sources, not
/// scanned code, §3.5), `rel` the mirror path with the `.stele/tree/` prefix stripped and
/// `source` the overlay absolute path. Every other build input (anchors, extraction, ADRs,
/// freshness) stays on `ws.work` + tracked, unchanged.
fn node_sources(ws: &Workspace, tracked: &[PathBuf]) -> Result<Vec<NodeSource>> {
    match ws.mode {
        Mode::Normal => Ok(tracked
            .iter()
            .filter(|rel| rel.file_name().is_some_and(|name| name == "AGENTS.md"))
            .map(|rel| NodeSource {
                rel: rel.clone(),
                source: ws.work.join(rel),
            })
            .collect()),
        Mode::Undercover => overlay_sources(&ws.home.join(TREE_DIR), Path::new("")),
    }
}

/// Every `AGENTS.md` under `<tree_root>/<rel>` (the overlay `<home>/.stele/tree/`, §3.5) as a
/// [`NodeSource`] whose `rel` mirrors the tree path (`<tree_root>/apps/api/AGENTS.md` →
/// `apps/api/AGENTS.md`), sorted by `rel`. A missing overlay root is empty, not an error (an
/// undercover repo with no scaffold yet builds an empty graph).
pub(super) fn overlay_sources(tree_root: &Path, rel: &Path) -> Result<Vec<NodeSource>> {
    // Collected through its components so the root itself (`rel` empty) has no trailing `/`.
    let dir: PathBuf = tree_root.join(rel).components().collect();
    let entries = match std::fs::read_dir(&dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries
            .map_err(|e| SteleError::internal(format!("read overlay {}: {e}", dir.display())))?,
    };
    let mut sources = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| SteleError::internal(format!("read overlay entry: {e}")))?;
        let (rel, source) = (rel.join(entry.file_name()), entry.path());
        let file_type = entry
            .file_type()
            .map_err(|e| SteleError::internal(format!("stat {}: {e}", source.display())))?;
        if file_type.is_dir() {
            sources.extend(overlay_sources(tree_root, &rel)?);
        } else if rel.file_name().is_some_and(|name| name == "AGENTS.md") {
            sources.push(NodeSource { rel, source });
        }
    }
    sources.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(sources)
}

/// Sources → in-memory graph (§5.1): parse every node source (§3.5, [`node_sources`]) into
/// nodes, scan the tracked tree for comment anchors (§2.4), resolve each claim's anchor to
/// a `file:line` (§2.4), index the repo's ADRs (§2.6), and derive import edges
/// (§2.3/§4.2). Shared by `build` and `check`, so every derived slot is recomputed
/// identically on both paths. Node discovery is mode-aware (normal: tracked `AGENTS.md`;
/// undercover: the overlay walk); every other scan runs over `ws.work` + tracked
/// unchanged. `verified` is stamped only by `build` afterward (§4.5).
pub(super) fn build_graph(ws: &Workspace, tracked: &[PathBuf]) -> Result<Graph> {
    let root = &ws.work;
    let mut graph = Graph::default();
    for source in node_sources(ws, tracked)? {
        let contents = read_node_source(&source.source, &source.rel)?;
        if let Some(node) = parse::parse_agents_file(&source.rel, &contents)? {
            graph.insert(node)?;
        }
    }

    // Import extraction (§2.3/§4.2): attribute each cross-boundary reference to its
    // owning node via the territory index, then fill each node's `extracted.imports`
    // and retain the per-edge reference occurrences for the structural class (§4.2).
    // Territory reads only the node roster, so extraction runs beside the anchor scan —
    // the two tree-sitter passes dominate a build — and its error still surfaces last.
    let territory = graph.territory();
    let (anchors, extraction) = std::thread::scope(|scope| {
        let extraction = assert::spawn(scope, || {
            extract::extract_imports(root, tracked, &territory)
        });
        (anchors::scan(root, tracked), extraction())
    });
    graph.anchors = anchors?;
    resolve_claim_anchors(root, &mut graph.nodes, &graph.anchors)?;
    graph.adrs = build_adr_index(root, tracked)?;
    let extraction = extraction?;
    for node in &mut graph.nodes {
        node.extracted_imports = extraction
            .per_node
            .get(&node.id)
            .cloned()
            .unwrap_or_default();
    }
    graph.import_edges = extraction.edges;
    Ok(graph)
}

/// [`build_graph`], then [`warn_untracked_node_files`] over the untracked-but-not-gitignored
/// files. Their `git ls-files --others` walk shares nothing with the build, so it runs beside
/// the tree-sitter passes; a build error still surfaces first and suppresses every warning.
pub(super) fn build_graph_and_warn(ws: &Workspace, tracked: &[PathBuf]) -> Result<Graph> {
    let (graph, others) = std::thread::scope(|scope| {
        let others = assert::spawn(scope, || {
            git_ls_files(&ws.work, &["--others", "--exclude-standard"])
        });
        (build_graph(ws, tracked), others())
    });
    let graph = graph?;
    warn_untracked_node_files(ws, others?)?;
    Ok(graph)
}

/// Read a node source (AGENTS.md, §3.1) from its absolute `source` path, naming the
/// repo-root-relative `rel` in any error (identical text in both modes, §3.5). Unlike the
/// scan's binary-skip (§2.4), a declared node source that is not valid UTF-8 is a hard
/// §5.3 input error naming the file — a node whose own bytes cannot be read is a
/// repo-out-of-spec condition, never silently dropped; so is one gone from the work tree
/// (§5.3 "a lock node whose file is gone"). A genuine IO failure stays a §5.3 internal error.
pub(super) fn read_node_source(source: &Path, rel: &Path) -> Result<String> {
    let bytes = crate::model::read_tracked(source, &rel.display().to_string())?
        .ok_or_else(|| SteleError::input_msg(format!("{}: node source is gone", rel.display())))?;
    String::from_utf8(bytes).map_err(|_| {
        SteleError::input_msg(format!(
            "{}: node source is not valid UTF-8 (§3.1)",
            rel.display()
        ))
    })
}

/// Resolve every claim's anchor to a `file:line` (§2.4), recomputed every build. A
/// `※ <slug>` landmark anchor resolves to the winning landmark occurrence (or stays `null`
/// when the slug has zero occurrences — build stays 0, §4.1 fails it later). A
/// `<path>#<symbol>` anchor resolves via tree-sitter: exactly one definition →
/// its line; zero or many → `null` plus the §4.1 unresolved-vs-ambiguous marker.
fn resolve_claim_anchors(
    root: &Path,
    nodes: &mut [Node],
    anchors: &crate::model::AnchorData,
) -> Result<()> {
    for node in nodes.iter_mut() {
        for claim in node.invariants.iter_mut().chain(node.hazards.iter_mut()) {
            if let Some(slug) = claim
                .anchor
                .strip_prefix(crate::model::LANDMARK_ANCHOR_PREFIX)
            {
                match anchors.winner(slug) {
                    Some(occ) => {
                        claim.resolved = Some(format!("{}:{}", occ.file, occ.line));
                        claim.resolution = Resolution::Resolved;
                    }
                    None => claim.resolution = Resolution::Unresolved,
                }
            } else if let Some((path, symbol)) = claim.anchor.rsplit_once('#') {
                match anchors::resolve_symbol(root, path, symbol)? {
                    SymbolResolution::Resolved(line) => {
                        claim.resolved = Some(format!("{path}:{line}"));
                        claim.resolution = Resolution::Resolved;
                    }
                    SymbolResolution::Ambiguous => claim.resolution = Resolution::Ambiguous,
                    SymbolResolution::Unresolved => claim.resolution = Resolution::Unresolved,
                }
            } else {
                // In neither namespace (no `※`, no `#`): derive_slug accepts it as a bare
                // symbol, but it binds nothing — §4.1 reports it unresolved.
                claim.resolution = Resolution::Unresolved;
            }
        }
    }
    Ok(())
}

/// Stamp `verified = {sha, digest}` on every claim whose anchor RESOLVES (§4.5),
/// using the full 40-hex `HEAD` sha (deterministic within a commit, so two builds
/// byte-match) and the tree-sitter structural digest of the claim's bound definition
/// (§4.5; `null` only where the anchored file's language has no bundled parser).
/// Unresolved claims stay `verified:null`. `build` is the sole stamper — `check`
/// carries `verified` over instead (§4.5). Returns `false` when HEAD is unborn (no
/// commit yet): every claim is left `verified:null` and the caller prints one notice
/// — a greenfield `stele build` still succeeds (§7), it simply has no watermark to
/// anchor to until the first commit.
pub(super) fn stamp_verified(root: &Path, graph: &mut Graph) -> Result<bool> {
    let Some(sha) = head_sha(root)? else {
        return Ok(false);
    };
    let claims: Vec<_> = graph
        .nodes
        .iter_mut()
        .flat_map(|node| node.invariants.iter_mut().chain(&mut node.hazards))
        .filter(|claim| claim.resolved.is_some())
        .collect();
    let digests = assert::par_map(&claims, |claim| {
        anchors::digest_for_claim(root, &claim.anchor, claim.resolved.as_deref().unwrap())
    });
    for (claim, digest) in claims.into_iter().zip(digests) {
        claim.verified = Some(Verified {
            sha: sha.clone(),
            digest: digest?,
        });
    }
    Ok(true)
}

/// The full `HEAD` commit sha (§4.5 watermark), or `None` when HEAD is unborn — a repo
/// with no commit yet (§7 greenfield). `git rev-parse --verify HEAD` fails on an unborn
/// HEAD; since `build_graph` already ran `git ls-files` successfully, that failure means
/// "no commit", never "not a git repo". A genuine spawn failure stays a §5.3 internal
/// error.
fn head_sha(root: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `git rev-parse --verify HEAD`: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

/// The source-scan file set (§2.4 scan scope): VCS-tracked files (so `.gitignore` is
/// honored by construction) MINUS every path a committed root `.steleignore` matches.
/// The single choke point every scan reads — node discovery, anchors, extraction, and
/// the §4.3 directory walk all consume this list — so filtering here makes an ignored
/// subtree invisible to all of them at once (`steleignore` module). Repo-root-relative
/// and sorted for deterministic iteration.
pub(super) fn tracked_files(root: &Path) -> Result<Vec<PathBuf>> {
    let ignore = Steleignore::load(root)?;
    Ok(git_ls_files(root, &[])?
        .into_iter()
        .filter(|rel| !ignore.is_ignored(&rel.to_string_lossy().replace('\\', "/")))
        .collect())
}

/// Warn (stderr only) about work-tree AGENTS.md files that declare a stele node the build's
/// roster will never read — a real adoption trap, mode-aware (§2.4/§3.5). For each untracked,
/// non-gitignored, non-steleignored work-tree AGENTS.md whose content carries a stele block,
/// print ONE warning line. This is purely diagnostic: stdout, the process exit code, and the
/// `--json` envelope are all untouched. Excluded (each for a reason): a gitignored/excluded
/// path (`--exclude-standard` drops it — including the undercover overlay under `.stele/`,
/// hidden by the `info/exclude` block, §3.5), a steleignored path (already invisible to every
/// scan, §2.4), and a blockless AGENTS.md (`Ok(None)` = plain-markdown degradation, not a
/// node). The message differs by mode: Normal names the untracked node the tracked scan misses
/// (run `git add`); Undercover names a node authored outside the overlay (build reads only
/// `.stele/tree/`), which is the confusion signal, not an adoption gap.
fn warn_untracked_node_files(ws: &Workspace, others: Vec<PathBuf>) -> Result<()> {
    let root = &ws.work;
    let ignore = Steleignore::load(root)?;
    for rel in others {
        if rel.file_name().is_none_or(|name| name != "AGENTS.md") {
            continue;
        }
        let posix = rel.to_string_lossy().replace('\\', "/");
        if ignore.is_ignored(&posix) {
            continue;
        }
        // Non-UTF-8 bytes cannot carry a parseable stele block, so an unreadable file reads
        // as empty — blockless, a silent skip (never a node). `Ok(None)` = blockless;
        // `Ok(Some)`/`Err` both mean a block is present (a malformed block still declares a
        // node the scan omits).
        let contents = std::fs::read_to_string(root.join(&rel)).unwrap_or_default();
        if matches!(parse::parse_agents_file(&rel, &contents), Ok(None)) {
            continue;
        }
        match ws.mode {
            Mode::Normal => eprintln!(
                "warning: {posix} declares a stele node but is not tracked — run git add \
                 (build scans tracked files only, §2.4)"
            ),
            Mode::Undercover => eprintln!(
                "warning: {posix} declares a stele node but undercover mode reads node sources \
                 from .stele/tree/ — move it into the overlay"
            ),
        }
    }
    Ok(())
}

/// `git ls-files <args> -z` (§2.4): with no args the raw VCS-tracked file list.
/// Repo-root-relative and sorted, NUL-delimited to survive unusual filenames.
pub(super) fn git_ls_files(root: &Path, args: &[&str]) -> Result<Vec<PathBuf>> {
    // Messages name the command by its first flag: `git ls-files`, `git ls-files --others`.
    let cmd = ["git ls-files"].into_iter().chain(args.first().copied());
    let cmd = cmd.collect::<Vec<_>>().join(" ");
    let output = Command::new("git")
        .arg("ls-files")
        .args(args)
        .arg("-z")
        .current_dir(root)
        .output()
        .map_err(|e| SteleError::internal(format!("run `{cmd}`: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Outside a work tree git prints "fatal: not a git repository …" — a user
        // mistake (§5.3 input, exit 2), not an internal fault. Surface a one-liner
        // instead of the raw git text so `stele <verb>` outside a repo fails cleanly. Only the
        // tracked scan (no args) is that probe; a later listing failing is internal.
        if args.is_empty() && stderr.contains("not a git repository") {
            return Err(SteleError::input_msg(
                "not a git repository (stele scans VCS-tracked files; run inside a git repo)",
            ));
        }
        return Err(SteleError::internal(format!(
            "`{cmd}` failed: {}",
            stderr.trim()
        )));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<PathBuf> = listing
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect();
    files.sort();
    Ok(files)
}

// ─── the ADR index (§2.6) ─────────────────────────────────────────────────────

/// The ADR directories stele probes, in precedence order (§10 item 5); the first
/// that holds a tracked `NNNN-*.md` file is the repo's ADR dir.
pub(super) const ADR_DIRS: [&str; 3] = ["adr", "doc/adr", "docs/adr"];

/// Index the repo's ADRs (§2.6): detect the ADR dir (the first of [`ADR_DIRS`] with
/// a tracked `NNNN-*.md` file), then parse each such file into an [`AdrEntry`] —
/// number and zero-padded id from the filename, status from its `Status:` line.
fn build_adr_index(root: &Path, tracked: &[PathBuf]) -> Result<Vec<AdrEntry>> {
    let paths: Vec<String> = tracked
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    let Some(dir) = ADR_DIRS
        .iter()
        .find(|dir| paths.iter().any(|p| adr_number(dir, p).is_some()))
    else {
        return Ok(Vec::new());
    };

    let mut entries = Vec::new();
    for path in &paths {
        let Some(nnnn) = adr_number(dir, path) else {
            continue;
        };
        let number: i64 = nnnn.parse().map_err(|_| {
            SteleError::input(path, 1, format!("ADR number {nnnn:?} is not an integer"))
        })?;
        // Gone (deleted, dangling) is skipped like any scanned file; bytes decode lossily,
        // as only the status token matters.
        let Some(bytes) = crate::model::read_tracked(&root.join(path), path)? else {
            continue;
        };
        entries.push(AdrEntry {
            id: format!("{dir}/{nnnn}"),
            number,
            status: adr_status(&String::from_utf8_lossy(&bytes)),
            path: path.clone(),
        });
    }
    Ok(entries)
}

/// The zero-padded number of a tracked ADR file `<dir>/NNNN-*.md` (e.g. `"0007"`),
/// or `None` when the path is not an ADR file directly under `dir`.
pub(super) fn adr_number<'a>(dir: &str, path: &'a str) -> Option<&'a str> {
    let name = (path.strip_prefix(dir)?.strip_prefix('/'))
        .filter(|name| !name.contains('/') && name.ends_with(".md"))?;
    let digits = name.split('-').next().filter(|d| !d.is_empty())?;
    digits.bytes().all(|b| b.is_ascii_digit()).then_some(digits)
}

/// The ADR status (§2.6): the lowercased status token, drawn from whichever of the three
/// conventions in the wild the file uses, or `"unknown"` when none is present. §4.1
/// checks `≠ superseded`.
///   • MADR YAML frontmatter — a `status:` key inside a leading `---`-fenced block;
///   • adr-tools/Nygard inline — a `Status: X` on any line (the original form);
///   • adr-tools/Nygard heading — a `## Status` ATX heading, the value on the first
///     non-empty line after it.
/// Each value reduces to its first word ([`status_word`]): quotes, emphasis and links drop.
/// Frontmatter outranks the inline form, which outranks the heading form.
fn adr_status(contents: &str) -> String {
    const UNKNOWN: &str = "unknown";
    frontmatter_status(contents)
        .or_else(|| inline_status(contents))
        .or_else(|| heading_status(contents))
        .unwrap_or_else(|| UNKNOWN.to_string())
}

/// The MADR YAML-frontmatter status: the `status:` key inside a leading `---` fence,
/// reduced to its status word (so a quoted scalar reads as its first word). `None` when
/// the file opens with no frontmatter or the block carries no `status:` key.
fn frontmatter_status(contents: &str) -> Option<String> {
    const FENCE: &str = "---";
    let mut lines = contents.lines();
    (lines.next()?.trim_end() == FENCE).then_some(())?;
    lines
        .map(str::trim)
        .take_while(|line| *line != FENCE)
        .find_map(|line| line.strip_prefix("status:"))
        .and_then(status_word)
}

/// The inline `Status: X` form (adr-tools/Nygard, the original): the status word after
/// the first `Status:` on any line, markdown emphasis on either side dropped. `None` when
/// no line carries a `Status:` with a word after it.
fn inline_status(contents: &str) -> Option<String> {
    contents
        .lines()
        .find_map(|line| line.split("Status:").nth(1))
        .and_then(status_word)
}

/// The `## Status` heading form (adr-tools/Nygard): the first non-empty line after a
/// `## Status` ATX heading, reduced to its status word. `None` when the file has no such
/// heading or nothing follows it.
fn heading_status(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    while let Some(line) = lines.next() {
        if is_status_heading(line) {
            return lines
                .by_ref()
                .find(|body| !body.trim().is_empty())
                .and_then(status_word);
        }
    }
    None
}

/// Whether `line` is a `## Status` ATX heading (any heading depth; case-insensitive) —
/// one or more leading `#`, then only the word `Status`.
fn is_status_heading(line: &str) -> bool {
    let trimmed = line.trim();
    let body = trimmed.trim_start_matches('#');
    body.len() < trimmed.len() && body.trim().eq_ignore_ascii_case("status")
}

/// The status word of a status value: quoting, emphasis and markdown-link syntax reduced
/// to its text by taking the first maximal run of ASCII letters and inner hyphens, lowercased (so
/// `[Accepted](0009.md)` → `accepted`, `Superseded by [ADR-9](…)` → `superseded`).
/// `None` for a line with no letters.
fn status_word(line: &str) -> Option<String> {
    let word: String = line
        .chars()
        .skip_while(|c| !c.is_ascii_alphabetic())
        .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
        .collect();
    (!word.is_empty()).then(|| word.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adr_status_reads_all_three_conventions() {
        // MADR YAML frontmatter (F5); a frontmatter block without `status:` defers.
        assert_eq!(
            adr_status("---\nstatus: superseded\ndate: 2026-01-01\n---\n# t\n"),
            "superseded"
        );
        assert_eq!(
            adr_status("---\ndate: 2026-01-01\n---\nStatus: Proposed\n"),
            "proposed"
        );
        // Inline `Status: X` — the original form still works, unchanged.
        assert_eq!(
            adr_status("# 7\n\nDate: 2026-03-02 · Status: Accepted\n"),
            "accepted"
        );
        // Nygard/adr-tools `## Status` heading: value on the first non-empty line after,
        // markdown links stripped, first word token decides.
        assert_eq!(adr_status("# 1\n\n## Status\n\nAccepted\n"), "accepted");
        assert_eq!(
            adr_status("# 1\n\n## Status\n\n[Superseded](0009-x.md) by ADR 9\n"),
            "superseded"
        );
        // A quoted YAML scalar (the MADR 4.0 template) and markdown emphasis reduce to the
        // status word, as the heading form does.
        for contents in [
            "---\nstatus: \"superseded by ADR-0002\"\n---\n",
            "**Status:** Superseded\n",
            "Status: **Superseded**\n",
        ] {
            assert_eq!(adr_status(contents), "superseded", "{contents:?}");
        }
        // A hyphenated status stays whole: `in-progress` is not `in`, nor is a
        // `superseded-candidate` superseded.
        assert_eq!(adr_status("---\nstatus: in-progress\n---\n"), "in-progress");
        assert_eq!(
            adr_status("Status: Superseded-candidate\n"),
            "superseded-candidate"
        );
        // No status anywhere → unknown.
        assert_eq!(adr_status("# t\n\n## Context\n\nnothing here\n"), "unknown");
    }
}
