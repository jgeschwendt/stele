//! The assertion suite (SPEC §4). `check` runs it over the rebuilt graph after the lock
//! byte-compare (§5.3); any [`Finding`] exits 1, "repo out of spec". This module owns the
//! finding model with its human (EXAMPLE §8 gallery) and `--json` (§5.3) renders, the
//! [`Context`] every class reads, the class dispatch honoring `--only` (§5.1) and
//! `check.disable` (§3.4), the scoped-thread helpers, and the literal-pathspec `git`
//! runner. Each class lives in its own submodule.

mod budget;
mod exhaustiveness;
mod freshness;
mod liveness;
mod referential;
mod structural;

pub use budget::tokenizer;
pub(crate) use exhaustiveness::IGNORED_DIRS;
pub use freshness::blame;
pub use structural::write_freeze;

use crate::config::{AssertionClass, Config};
use crate::lock::Lock;
use crate::model::{Graph, Result, SteleError};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread::{Builder, Scope};

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

/// A finding's severity (§5.3). Every finding is an error; the enum leaves room for a
/// future warn tier without reshaping the envelope.
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

    /// A location; `text` is the reference's own source line (§4.2 structural), printed
    /// after `file:line` in the human render; the JSON envelope still omits it.
    fn location(mut self, file: String, line: usize, text: Option<String>) -> Self {
        self.locations.push(Location { file, line, text });
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
/// (§3.4), the `.steleignore`-filtered tracked file set (§4.3 exhaustiveness scope), and
/// the committed lock — the sole carrier of `verified {sha, digest}` on the check path
/// (§4.5; `build` stamps it, `check` never re-stamps, so the rebuilt graph's claims
/// carry no watermark and freshness reads it from here).
pub struct Context<'a> {
    pub committed: &'a Lock,
    pub config: &'a Config,
    pub graph: &'a Graph,
    /// The graph home (§3.5) holding the engine's artifacts — here the `--freeze`
    /// baseline. Equal to [`Self::root`] in normal mode.
    pub home: &'a Path,
    pub root: &'a Path,
    /// `check --run-commands` (§4.6): additionally EXECUTE each declared command from
    /// the repo root (the off-by-default bonfires tier). Resolution runs regardless.
    pub run_commands: bool,
    /// The directory each node's `source` path (§3.1) resolves against for the budget
    /// class's file reads (§4.4). Equal to [`Self::root`] in normal mode; the overlay
    /// `<home>/.stele/tree` in undercover mode (§3.5), where node sources live off the
    /// work tree. Every other class reads code from [`Self::root`] (the work tree).
    pub source_root: &'a Path,
    pub tracked: &'a [PathBuf],
}

impl Context<'_> {
    /// Whether `rel` (a repo-root-relative POSIX path) is a VCS-tracked file (§4.1
    /// enforced_by: tracked implies committed). Asks git, not [`Self::tracked`]: that is
    /// the `.steleignore`-filtered scan roster, and a referenced guard is not scanned.
    fn tracks(&self, rel: &str) -> Result<bool> {
        let output = git(self.root, "ls-files -z --", &[rel])?;
        // An unmerged path is listed once per stage; a failed git lists nothing.
        Ok(output.status.success()
            && output
                .stdout
                .split(|&b| b == 0)
                .any(|e| e == rel.as_bytes()))
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

/// Dispatch one class (§4.1–§4.6).
fn run_class(ctx: &Context, class: AssertionClass) -> Result<Vec<Finding>> {
    match class {
        AssertionClass::Budget => budget::budget(ctx),
        AssertionClass::Exhaustiveness => exhaustiveness::exhaustiveness(ctx),
        AssertionClass::Freshness => freshness::freshness(ctx),
        AssertionClass::Referential => referential::referential(ctx),
        AssertionClass::Structural => structural::structural(ctx),
        AssertionClass::Liveness => liveness::liveness(ctx),
    }
}

/// `f` over `items` on up to one thread per core, in order — each claim's digest re-reads
/// and re-parses its file, and its churn count spawns git, so the per-claim passes of
/// `build` and freshness are the engine's widest serial loops.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    std::thread::scope(|scope| {
        let chunks: Vec<_> = items
            .chunks(items.len().div_ceil(cores).max(1))
            .map(|chunk| spawn(scope, || chunk.iter().map(&f).collect::<Vec<_>>()))
            .collect();
        chunks.into_iter().flat_map(|join| join()).collect()
    })
}

/// `f` on a thread of `scope`, joined by calling the result — or, when the OS refuses
/// the thread, run here and now: a parallel pass degrades to the serial one, never a panic.
pub fn spawn<'s, R: Send + 's>(
    scope: &'s Scope<'s, '_>,
    f: impl FnOnce() -> R + Send + 's,
) -> Box<dyn FnOnce() -> R + 's> {
    spawn_on(Builder::new(), scope, f)
}

/// [`spawn`] on `builder` (a test seam: an impossible stack makes the OS refuse).
fn spawn_on<'s, R: Send + 's>(
    builder: Builder,
    scope: &'s Scope<'s, '_>,
    f: impl FnOnce() -> R + Send + 's,
) -> Box<dyn FnOnce() -> R + 's> {
    // A refused spawn drops its closure, so the job is shared to survive it.
    let job = Arc::new(Mutex::new(Some(f)));
    let theirs = Arc::clone(&job);
    match builder.spawn_scoped(scope, move || theirs.lock().unwrap().take().unwrap()()) {
        Ok(thread) => Box::new(move || thread.join().unwrap()),
        Err(_) => {
            let done = job.lock().unwrap().take().unwrap()();
            Box::new(move || done)
        }
    }
}

/// Run `git <cmd> <args>` from the repo root (§4.5). A spawn failure is a §5.3 internal
/// error naming `git <cmd>`.
/// Pathspecs are literal — stele's paths name files, so `:x` is no magic and `[ab]` no
/// glob — and an inherited glob or icase `GIT_*_PATHSPECS`, which git refuses beside
/// literal mode, is dropped.
fn git(root: &Path, cmd: &str, args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .args(cmd.split(' ').chain(args.iter().copied()))
        .env_remove("GIT_GLOB_PATHSPECS")
        .env_remove("GIT_ICASE_PATHSPECS")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env_remove("GIT_NOGLOB_PATHSPECS")
        .current_dir(root)
        .output();
    output.map_err(|e| SteleError::internal(format!("run `git {cmd}`: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thread the OS refuses (here an impossible stack) runs its work inline on the
    /// calling thread, never panicking the command.
    #[test]
    fn refused_thread_runs_inline() {
        let refused = std::thread::Builder::new().stack_size(usize::MAX);
        let ran =
            std::thread::scope(|scope| spawn_on(refused, scope, || std::thread::current().id())());
        assert_eq!(ran, std::thread::current().id());
    }
}
