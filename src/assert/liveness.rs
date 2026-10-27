//! Liveness (§4.6): every declared command resolves to a runner task, builtin or
//! executable, and with `--run-commands` exits zero.

use super::{Context, Finding};
use crate::config::AssertionClass;
use crate::model::{Node, Result, SteleError};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// The `mix` builtin tasks (§4.6): the stock Mix tasks `mix help` lists (Elixir 1.20.2,
/// archive tasks such as `hex.*`/`phx.*` excluded) plus `docs` and `loadpaths`; anything
/// else must be a `mix.exs` alias to resolve. Alpha-sorted.
const MIX_BUILTINS: [&str; 43] = [
    "app.config",
    "app.start",
    "app.tree",
    "archive",
    "archive.build",
    "archive.install",
    "archive.uninstall",
    "clean",
    "cmd",
    "compile",
    "deps",
    "deps.clean",
    "deps.compile",
    "deps.get",
    "deps.tree",
    "deps.unlock",
    "deps.update",
    "do",
    "docs",
    "escript",
    "escript.build",
    "escript.install",
    "escript.uninstall",
    "eval",
    "format",
    "help",
    "loadconfig",
    "loadpaths",
    "local",
    "local.hex",
    "local.rebar",
    "new",
    "profile.cprof",
    "profile.eprof",
    "profile.fprof",
    "profile.tprof",
    "release",
    "release.init",
    "run",
    "source",
    "test",
    "test.coverage",
    "xref",
];

/// The `cargo` builtin subcommands (§4.6): those with a man page in the toolchain's
/// share/man/man1, the built-in aliases `cargo --list` names (b, c, d, r, rm, t), and the
/// rustup components `clippy`/`fmt`; anything else must be a `.cargo/config.toml`
/// `[alias]`. Alpha-sorted.
const CARGO_BUILTINS: [&str; 43] = [
    "add",
    "b",
    "bench",
    "build",
    "c",
    "check",
    "clean",
    "clippy",
    "d",
    "doc",
    "fetch",
    "fix",
    "fmt",
    "generate-lockfile",
    "help",
    "info",
    "init",
    "install",
    "locate-project",
    "login",
    "logout",
    "metadata",
    "new",
    "owner",
    "package",
    "pkgid",
    "publish",
    "r",
    "remove",
    "report",
    "rm",
    "run",
    "rustc",
    "rustdoc",
    "search",
    "t",
    "test",
    "tree",
    "uninstall",
    "update",
    "vendor",
    "version",
    "yank",
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
/// (special built-ins) + .../V3_chap04.html (alias, bg, cd, command, echo, false, fg,
/// getopts, hash, jobs, printf, pwd, read, test, true, type, ulimit, umask, unalias,
/// wait). Alpha-sorted (ASCII).
const SHELL_BUILTINS: [&str; 36] = [
    ".", ":", "[", "alias", "bg", "break", "cd", "command", "continue", "echo", "eval", "exec",
    "exit", "export", "false", "fg", "getopts", "hash", "jobs", "printf", "pwd", "read",
    "readonly", "return", "set", "shift", "test", "times", "trap", "true", "type", "ulimit",
    "umask", "unalias", "unset", "wait",
];

/// The liveness class (§4.6). For every declared command on every node: parse it into
/// shell-operator-separated segments and RESOLVE each segment's executable (always); and
/// when `ctx.run_commands` is set, additionally EXECUTE the whole command from the repo
/// root, reporting a non-zero exit. Resolution reads runner manifests once up front.
pub(super) fn liveness(ctx: &Context) -> Result<Vec<Finding>> {
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
    let streams = [stdout, stderr].map(String::from_utf8_lossy);
    let texts: Vec<&str> = streams
        .iter()
        .map(|s| s.trim())
        .filter(|t| !t.is_empty())
        .collect();
    let out = texts.join("\n");
    if out.len() <= CAPTURED_OUTPUT_CAP {
        return out;
    }
    // Snap up to a char boundary so the truncation never splits a UTF-8 sequence.
    format!(
        "…{}",
        &out[out.ceil_char_boundary(out.len() - CAPTURED_OUTPUT_CAP)..]
    )
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
    path_env
        .into_iter()
        .flat_map(std::env::split_paths)
        .any(|dir| is_executable_file(&dir.join(name)))
}

/// Whether `path` is a regular file with an executable bit set. On non-Unix the bit is
/// unobservable, so any existing regular file counts (§4.6).
fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|meta| {
        #[cfg(unix)]
        let executable = meta.permissions().mode() & 0o111 != 0;
        #[cfg(not(unix))]
        let executable = true;
        meta.is_file() && executable
    })
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
    let block = source
        .find("aliases do")
        .map_or("", |start| &source[start..]);
    block
        .lines()
        .skip(1)
        .map(str::trim)
        .take_while(|line| *line != "end")
        .filter_map(mix_alias_key)
        .collect()
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
    [".cargo/config.toml", ".cargo/config"]
        .into_iter()
        .find_map(|name| read_manifest(root, name))
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
        .and_then(|value| {
            let table = value.get("alias")?.as_table()?;
            Some(table.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// Extract recipe names from a justfile (§4.6): an unindented line whose name (up to its
/// first `:`, a quiet `@` dropped) is a recipe head. Of the `name := value` lines only
/// `alias name := recipe` names one; recipe parameters after the name are ignored. Blank
/// lines, `set …` directives and `#` comments need no rule: they carry no `:`, or a head
/// that is not a recipe name.
fn parse_just_recipes(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter(|line| !line.starts_with([' ', '\t']))
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(head, rest)| {
            let mut words = head.split_whitespace();
            match (words.next()?, rest.starts_with('=')) {
                ("alias", true) => words.next(),
                (name, false) => Some(name.strip_prefix('@').unwrap_or(name)),
                _ => None,
            }
        })
        .filter(|name| {
            name.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
        .map(str::to_string)
        .collect()
}
