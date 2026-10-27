//! Exhaustiveness (§4.3): every non-ignored directory to depth D maps into a non-root
//! node's territory.

use super::{Context, Finding};
use crate::config::AssertionClass;
use crate::model::{NodeKind, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The two dir names always dropped from the walk (§4.3): the VCS metadata dir and
/// stele's own committed home. Gitignored paths, untracked dot-dirs, and every path a
/// committed root `.steleignore` matches never appear — the scan is built from the
/// filtered tracked-file list (`tracked_files`, cli/graph.rs), so a `.steleignore`d subtree
/// contributes no files, hence no directory here: an ignored dir is INVISIBLE, never an
/// unmapped recall failure (§2.4). None of the three needs an explicit entry.
pub(crate) const IGNORED_DIRS: [&str; 2] = [".git", ".stele"];

/// The exhaustiveness class (§4.3): every non-ignored directory at depth ≤ D must map
/// into some NON-ROOT node's territory. The system node `/` is a catch-all for
/// import attribution only — it covers repo-root direct files (never a depth-≥1
/// directory), so a subtree reachable through no `container`/`component`/`adr` node is
/// unrouted context and fails. Pass-through dirs (structural ancestors of a node, like
/// `apps/`) are covered but the scan descends through them regardless of D.
pub(super) fn exhaustiveness(ctx: &Context) -> Result<Vec<Finding>> {
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
/// (§2.6) knows the dir, so a clean repo with a bare `adr/` (no AGENTS.md) passes. A
/// node's declared location is the repo-root-relative parent of its AGENTS.md; the root
/// (`""`, whatever its kind) never covers — it would swallow every directory.
fn covering_dirs(ctx: &Context) -> BTreeSet<String> {
    let nodes = ctx
        .graph
        .nodes
        .iter()
        .filter(|n| n.kind != NodeKind::System);
    let node_dirs = nodes
        .filter_map(|n| n.source.parent())
        .map(|dir| dir.to_string_lossy().replace('\\', "/"));
    let adr_dirs = ctx
        .graph
        .adrs
        .iter()
        .filter_map(|adr| adr.path.rsplit_once('/'));
    node_dirs
        .chain(adr_dirs.map(|(dir, _)| dir.to_string()))
        .filter(|dir| !dir.is_empty())
        .collect()
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
/// precedes `apps/web` but not `apps` itself nor `appswide`). Both sides are non-empty:
/// covering dirs filter `""` out and scanned dirs are built from path segments.
fn is_strict_ancestor(ancestor: &str, descendant: &str) -> bool {
    descendant
        .strip_prefix(ancestor)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// The tracked-directory tree for §4.3: which directories exist (a dir exists iff it
/// holds ≥1 tracked file at any depth), each one's immediate children, and its
/// recursive tracked-file count. Built from the VCS-tracked file list with the ignore
/// set (`.git`, `.stele`, `exhaustiveness.exclude` globs) applied per directory.
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
            if (1..=dir_segments.len()).any(|n| is_ignored_dir(&dir_segments[..n], excludes)) {
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

/// Whether the root-relative dir `segments` is dropped from the §4.3 walk: its name is a
/// fixed ignored dir (`.git`/`.stele`) or matches an `exhaustiveness.exclude` glob read as a
/// gitignore line — `./`, a leading/trailing `/` and a trailing `/**` dropped; a glob that
/// had a non-trailing `/` anchored at the root (`apps/legacy`, `/legacy`), one led by `**/`
/// against every path suffix, any other against the name.
fn is_ignored_dir(segments: &[&str], excludes: &[String]) -> bool {
    let name = segments[segments.len() - 1];
    IGNORED_DIRS.contains(&name)
        || excludes.iter().any(|glob| {
            let deep = glob.starts_with("**/");
            let anchored = !deep && glob.trim_end_matches('/').contains('/');
            let glob = glob
                .trim_start_matches("./")
                .trim_start_matches("**/")
                .trim_matches('/');
            let glob = glob.strip_suffix("/**").unwrap_or(glob);
            match (anchored, deep) {
                (true, _) => glob_matches(glob, &segments.join("/")),
                (_, true) => {
                    (0..segments.len()).any(|i| glob_matches(glob, &segments[i..].join("/")))
                }
                _ => glob_matches(glob, name),
            }
        })
}

/// A minimal `*`/`?` glob match (anchored, whole-string; `*` spans `/`) for
/// `exhaustiveness.exclude` (§3.4). The defaults are plain names; wildcards cover a
/// team's custom entries.
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
