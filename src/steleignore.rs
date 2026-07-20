//! `.steleignore` — gitignore-syntax scan exclusion (SPEC §2.4).
//!
//! A committed root `.steleignore` hides paths from ALL build/check source scanning:
//! node discovery (§3.1), the anchor scan (§2.4), import extraction (§4.2), AND the
//! §4.3 exhaustiveness directory walk. Every one of those consumes the same tracked-
//! file list, so filtering that single list at its source (`tracked_files`, cli.rs) is
//! the whole integration: an ignored subtree contributes no files, hence no nodes, no
//! anchors, no imports, and — the reading the task pins down — no directory the
//! exhaustiveness scan can call a recall failure. An ignored dir is INVISIBLE, not
//! unmapped; "invisible to all scanning" is exactly why it cannot fire §4.3.
//!
//! `.steleignore` is a SOURCE input to `build` (§2.4), never check-time config (§3.4):
//! it feeds the tracked-file list every `build` reads, so the lock is
//! `.steleignore`-dependent by construction and `.stele/config.toml` stays graph-free.
//!
//! Syntax is a faithful subset of gitignore(5): blank and `#` lines are skipped; `!`
//! negates (last matching pattern wins); a leading or internal `/` anchors the pattern
//! to the repo root; a trailing `/` matches directories only; `*` and `?` match within
//! one path segment; `**` matches across segments; a slash-free pattern matches at any
//! depth. Two documented divergences from full gitignore, neither reachable by a
//! source-scan exclude list: character classes (`[a-z]`) and backslash escapes are not
//! parsed, and gitignore's "a file under an excluded parent cannot be re-included by a
//! later `!` negation" rule is not modeled — negation is evaluated per path.

use crate::model::{Result, SteleError};
use std::path::Path;

/// The committed root exclude file (§2.4).
const STELEIGNORE_FILE: &str = ".steleignore";

/// The compiled root `.steleignore`: an ordered pattern list matched last-wins.
#[derive(Default)]
pub struct Steleignore {
    patterns: Vec<Pattern>,
}

/// One compiled ignore pattern. `segments` is the `/`-split glob (a leading `**` is
/// synthesized for a slash-free pattern so it matches at any depth); `dir_only` marks a
/// trailing-slash pattern that matches directories only; `negated` marks a `!` pattern.
struct Pattern {
    dir_only: bool,
    negated: bool,
    segments: Vec<Segment>,
}

/// A single path segment of a pattern: `**` (spanning) or a within-segment glob.
enum Segment {
    DoubleStar,
    Glob(String),
}

impl Steleignore {
    /// Load the root `.steleignore` (§2.4). Absent → an empty matcher (ignore nothing),
    /// which is why a repo without one behaves exactly as before this feature.
    pub fn load(root: &Path) -> Result<Self> {
        match std::fs::read_to_string(root.join(STELEIGNORE_FILE)) {
            Ok(contents) => Ok(Self::parse(&contents)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(SteleError::internal(format!(
                "read {STELEIGNORE_FILE}: {e}"
            ))),
        }
    }

    /// Compile every non-comment line into a [`Pattern`] (§2.4 gitignore subset).
    pub fn parse(contents: &str) -> Self {
        Self {
            patterns: contents.lines().filter_map(Pattern::parse).collect(),
        }
    }

    /// Whether a repo-root-relative POSIX `path` is excluded (§2.4). Last matching
    /// pattern wins; a `!` pattern re-includes. A path is matched by a pattern when the
    /// pattern matches the whole path or any ancestor directory of it (everything under
    /// an excluded directory is excluded too).
    pub fn is_ignored(&self, path: &str) -> bool {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.matches(&segments) {
                ignored = !pattern.negated;
            }
        }
        ignored
    }
}

impl Pattern {
    /// Compile one `.steleignore` line, or `None` for a blank/comment line.
    fn parse(raw: &str) -> Option<Self> {
        // A CRLF file leaves a trailing CR; trailing spaces are insignificant in the
        // gitignore subset we support (escaped trailing spaces are not modeled).
        let line = raw.strip_suffix('\r').unwrap_or(raw).trim_end();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        let (negated, line) = match line.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let (dir_only, line) = match line.strip_suffix('/') {
            Some(rest) => (true, rest),
            None => (false, line),
        };

        // Any remaining `/` (leading or internal) anchors the pattern to the repo root;
        // a slash-free pattern matches at any depth, expressed as a leading `**`.
        let anchored = line.contains('/');
        let body = line.strip_prefix('/').unwrap_or(line);

        let mut segments = Vec::new();
        if !anchored {
            segments.push(Segment::DoubleStar);
        }
        for seg in body.split('/').filter(|s| !s.is_empty()) {
            segments.push(if seg == "**" {
                Segment::DoubleStar
            } else {
                Segment::Glob(seg.to_string())
            });
        }
        if segments.is_empty() {
            return None;
        }
        Some(Pattern {
            dir_only,
            negated,
            segments,
        })
    }

    /// Whether this pattern excludes `path` (given as its non-empty segment list). The
    /// pattern must fully match either the whole path or one of its ancestor-directory
    /// prefixes; a `dir_only` pattern never matches the final (file) segment, so it
    /// tests strict prefixes only.
    fn matches(&self, path: &[&str]) -> bool {
        let last_prefix = if self.dir_only {
            path.len().saturating_sub(1)
        } else {
            path.len()
        };
        (1..=last_prefix).any(|len| seg_match(&self.segments, &path[..len]))
    }
}

/// Match a pattern's segment list against a candidate segment list, `**` spanning zero
/// or more segments. A full (both-exhausted) match is required.
fn seg_match(pattern: &[Segment], candidate: &[&str]) -> bool {
    match pattern.split_first() {
        None => candidate.is_empty(),
        Some((Segment::DoubleStar, rest)) => {
            (0..=candidate.len()).any(|skip| seg_match(rest, &candidate[skip..]))
        }
        Some((Segment::Glob(glob), rest)) => match candidate.split_first() {
            Some((head, tail)) if glob_match(glob, head) => seg_match(rest, tail),
            _ => false,
        },
    }
}

/// Match a single-segment glob (`*` any run, `?` one char, else literal) against `text`.
/// Iterative wildcard matching with `*`-backtracking; `/` never appears in a segment.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut resume) = (None, 0);
    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            resume = ti;
            pi += 1;
        } else if let Some(star) = star {
            pi = star + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ig(patterns: &str, path: &str) -> bool {
        Steleignore::parse(patterns).is_ignored(path)
    }

    #[test]
    fn dir_pattern_excludes_the_whole_subtree() {
        let s = "tests/fixtures/\n";
        assert!(ig(s, "tests/fixtures/acme/AGENTS.md"));
        assert!(ig(s, "tests/fixtures/acme/apps/web/refund.ex"));
        // The directory itself and sibling files stay visible.
        assert!(!ig(s, "tests/cli.rs"));
        assert!(!ig(s, "tests/fixtures.md"));
    }

    #[test]
    fn blank_and_comment_lines_are_skipped() {
        let s = "# a comment\n\n  \ntests/fixtures/\n";
        assert!(ig(s, "tests/fixtures/x"));
        assert_eq!(Steleignore::parse("# only comments\n\n").patterns.len(), 0);
    }

    #[test]
    fn slash_free_pattern_matches_at_any_depth() {
        let s = "node_modules\n";
        assert!(ig(s, "node_modules/pkg/index.js"));
        assert!(ig(s, "apps/web/node_modules/pkg/index.js"));
        assert!(!ig(s, "apps/web/src/node_modules_helper.rs"));
    }

    #[test]
    fn anchored_pattern_binds_to_the_root() {
        let s = "/target\n";
        assert!(ig(s, "target/debug/stele"));
        // A deeper `target` is untouched by the root-anchored pattern.
        assert!(!ig(s, "crates/inner/target/x"));
    }

    #[test]
    fn glob_and_doublestar() {
        assert!(ig("*.tmp\n", "a/b/c.tmp"));
        assert!(!ig("*.tmp\n", "a/b/c.tmpx"));
        assert!(ig("build/**/gen.rs\n", "build/a/b/gen.rs"));
        assert!(ig("build/**/gen.rs\n", "build/gen.rs"));
        assert!(!ig("build/**/gen.rs\n", "build/a/gen.rsx"));
    }

    #[test]
    fn negation_reincludes_last_wins() {
        let s = "*.log\n!keep.log\n";
        assert!(ig(s, "a/debug.log"));
        assert!(!ig(s, "a/keep.log"));
    }
}
