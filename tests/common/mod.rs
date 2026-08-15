#![allow(dead_code)]
//! Shared harness for the oracle integration tests (SPEC §5.1 pipeline, §5.3 process
//! contract). Each test owns a fresh temp copy of a fixture, wrapped in a real git
//! repo so scan scope (§2.4, VCS-tracked files only) and freshness (§4.5, verified
//! {sha}) have a history to read. The built binary is invoked via CARGO_BIN_EXE_stele
//! with `current_dir` set to that copy — no test depends on execution order.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The `stele` binary Cargo built for this test run.
pub const BIN: &str = env!("CARGO_BIN_EXE_stele");

/// A process-lifetime global git config that neutralizes the developer's host git config AND
/// the XDG `core.excludesFile` (`~/.config/git/ignore`). This machine's global ignore lists
/// `**/CLAUDE.local.md`, which would MASK an undercover shim leaking into `git status` and make
/// the leak tests pass vacuously. `GIT_CONFIG_GLOBAL=/dev/null` alone does NOT disable the XDG
/// excludes file (its default path is not config-derived, verified 2026-07-21), so the isolation
/// config sets `core.excludesFile = /dev/null` explicitly.
fn git_config_global() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut path = std::env::temp_dir();
        path.push(format!("stele-test-gitconfig-{}", std::process::id()));
        fs::write(&path, "[core]\n\texcludesFile = /dev/null\n").expect("write test git config");
        path
    })
    .as_path()
}

/// Isolate a spawned `git` (or `stele`, whose own `git` children inherit this env) from host and
/// user git config: replace the global config with [`git_config_global`] and void the system
/// config, and drop the repo-locating variables git exports into hook children (under a
/// `pre-push` hook, an inherited `GIT_DIR` points every fixture's git at the host repo). Every
/// `Command` the harness spawns routes through this so an empty `git status` proves the engine's
/// exclude block, never the developer's machine.
pub fn isolate_git(cmd: &mut Command) -> &mut Command {
    for var in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_WORK_TREE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("GIT_CONFIG_GLOBAL", git_config_global())
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
}

const FIXTURE_ACME: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/acme");

/// A captured `stele` invocation: exit code plus its two output streams. Messages may
/// land on either stream, so assertions match against [`RunResult::combined`].
pub struct RunResult {
    pub code: i32,
    pub stderr: String,
    pub stdout: String,
}

impl RunResult {
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// The runner/tool executables the liveness class (§4.6) probes on PATH. Stubbed as
/// exit-0 no-ops so PATH resolution is host-independent — the acme fixture's `setup`
/// command names `mise`, and the runner names resolve to real binaries on a dev box but
/// to nothing on a bare CI host. Alpha-sorted.
const STUB_BINS: [&str; 7] = ["cargo", "just", "mise", "mix", "npm", "pnpm", "yarn"];

/// The system directories appended after `stub-bin` on the child `PATH` (§4.6 harness):
/// enough for the engine's own `git`/`sh` subprocesses, but NOT enough to leak the
/// host's real runners ahead of the stubs. `stub-bin` is always first, so a stub wins
/// over any same-named host binary.
const SYSTEM_PATH_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"];

/// A per-test working tree: a temp directory that is also a git repo. Dropping it
/// deletes the directory. A second temp directory (`_stub_dir`) holds the exit-0 tool
/// stubs kept OUTSIDE the git tree so they never count toward tracked files (§4.3).
pub struct Fixture {
    _dir: tempfile::TempDir,
    _stub_dir: tempfile::TempDir,
    stub_bin: PathBuf,
    pub root: PathBuf,
}

impl Fixture {
    /// A fresh copy of `tests/fixtures/acme`, committed as the clean baseline.
    pub fn acme() -> Self {
        let fixture = Self::new();
        copy_dir(Path::new(FIXTURE_ACME), &fixture.root);
        fixture.commit("import acme fixture");
        fixture
    }

    /// An empty git repo the caller materializes and commits itself (walkthrough §7).
    pub fn bare() -> Self {
        Self::new()
    }

    /// A depth-1 shallow clone of this repo into a fresh temp tree (§4.5 F11/F12 probe):
    /// only HEAD is present, so a watermark commit from before the clone boundary is
    /// unreachable — the exact condition an `actions/checkout` `fetch-depth: 1` produces.
    /// The clone runs the binary with the same child-PATH discipline as [`Self::run`].
    pub fn shallow_clone(&self) -> Self {
        let dir = tempfile::tempdir().expect("create clone dir");
        let root = dir.path().to_path_buf();
        let url = format!("file://{}", self.root.display());
        let out = isolate_git(
            Command::new("git")
                .args(["clone", "--depth", "1", &url, "."])
                .current_dir(&root),
        )
        .output()
        .expect("spawn git clone");
        assert!(
            out.status.success(),
            "git clone --depth 1 failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stub_dir = tempfile::tempdir().expect("create stub-bin dir");
        let stub_bin = stub_dir.path().to_path_buf();
        write_stub_bins(&stub_bin);
        Self {
            _dir: dir,
            _stub_dir: stub_dir,
            stub_bin,
            root,
        }
    }

    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().to_path_buf();
        let stub_dir = tempfile::tempdir().expect("create stub-bin dir");
        let stub_bin = stub_dir.path().to_path_buf();
        write_stub_bins(&stub_bin);
        let fixture = Self {
            _dir: dir,
            _stub_dir: stub_dir,
            stub_bin,
            root,
        };
        fixture.git(&["init", "-b", "main"]);
        fixture
    }

    /// The child `PATH` for [`Self::run`]: `stub-bin` first (so §4.6 runner/tool names
    /// resolve host-independently), then the minimal system dirs the engine's own
    /// `git`/`sh` subprocesses need.
    fn child_path(&self) -> String {
        child_path(&self.stub_bin)
    }

    /// Stage everything WITHOUT committing — leaves HEAD unborn on a fresh repo (the §7
    /// greenfield / F9 build probe: `git ls-files` sees the node, but there is no commit
    /// to anchor a watermark to).
    pub fn stage_all(&self) {
        self.git(&["add", "-A"]);
    }

    /// Stage everything and commit with inline identity (no global git config needed).
    pub fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&[
            "-c",
            "user.email=stele-test@example.com",
            "-c",
            "user.name=stele test",
            "commit",
            "--allow-empty",
            "-m",
            message,
        ]);
    }

    /// Run the built binary in the working tree, capturing status + both streams. The
    /// child `PATH` is forced through `stub-bin` (§4.6) so liveness PATH resolution is
    /// host-independent — identical on a dev box and a bare CI host.
    pub fn run(&self, args: &[&str]) -> RunResult {
        let output = isolate_git(
            Command::new(BIN)
                .args(args)
                .current_dir(&self.root)
                .env("PATH", self.child_path()),
        )
        .output()
        .expect("spawn stele binary");
        RunResult {
            code: output.status.code().expect("stele terminated by signal"),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        }
    }

    /// Spawn the binary feeding `stdin`, then read both streams to completion once the
    /// child exits — the `stele serve` harness (§5.2). The whole input is written up
    /// front and stdin is then closed; the MCP server reads to EOF and exits, so the
    /// captured stdout holds every newline-delimited response in request order. Same
    /// child-PATH discipline as [`Self::run`] so command resolution stays host-independent.
    pub fn run_with_stdin(&self, args: &[&str], stdin: &str) -> RunResult {
        use std::io::Write;
        let mut child = isolate_git(
            Command::new(BIN)
                .args(args)
                .current_dir(&self.root)
                .env("PATH", self.child_path()),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn stele binary");
        child
            .stdin
            .take()
            .expect("child stdin")
            .write_all(stdin.as_bytes())
            .expect("write child stdin");
        let output = child.wait_with_output().expect("wait for stele");
        RunResult {
            code: output.status.code().expect("stele terminated by signal"),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        }
    }

    // ─── mutation helpers ───────────────────────────────────────────────────────

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
    }

    pub fn write(&self, rel: &str, contents: &str) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, contents).unwrap_or_else(|e| panic!("write {rel}: {e}"));
    }

    pub fn append(&self, rel: &str, contents: &str) {
        let mut existing = self.read(rel);
        existing.push_str(contents);
        self.write(rel, &existing);
    }

    pub fn delete_file(&self, rel: &str) {
        fs::remove_file(self.path(rel)).unwrap_or_else(|e| panic!("delete {rel}: {e}"));
    }

    /// Replace the first occurrence of `from`; panics if it is absent (so a fixture
    /// drift that silently no-ops the mutation fails loudly).
    pub fn replace(&self, rel: &str, from: &str, to: &str) {
        let source = self.read(rel);
        assert!(
            source.contains(from),
            "{rel}: substring not found: {from:?}"
        );
        self.write(rel, &source.replacen(from, to, 1));
    }

    /// Delete the single line containing `needle`; panics unless exactly one matches.
    pub fn delete_line_containing(&self, rel: &str, needle: &str) {
        let source = self.read(rel);
        let kept: Vec<&str> = source.lines().filter(|l| !l.contains(needle)).collect();
        let removed = source.lines().count() - kept.len();
        assert_eq!(
            removed, 1,
            "{rel}: expected one line matching {needle:?}, removed {removed}"
        );
        self.write(rel, &rejoin(&kept, &source));
    }

    /// Insert `text` as a new line at 1-based `line_no`, shifting later lines down.
    pub fn insert_line_at(&self, rel: &str, line_no: usize, text: &str) {
        let source = self.read(rel);
        let mut lines: Vec<&str> = source.lines().collect();
        let index = line_no - 1;
        assert!(
            index <= lines.len(),
            "{rel}: line {line_no} beyond end of file"
        );
        lines.insert(index, text);
        self.write(rel, &rejoin(&lines, &source));
    }

    fn git(&self, args: &[&str]) {
        let output = isolate_git(Command::new("git").args(args).current_dir(&self.root))
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Materialize the [`STUB_BINS`] as exit-0 executables in `dir` (§4.6 harness). Each is
/// a `#!/bin/sh` no-op with the Unix executable bit set, so the engine's liveness PATH
/// probe and `--run-commands` execution both see a present, succeeding tool.
fn write_stub_bins(dir: &Path) {
    for name in STUB_BINS {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write stub bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub bin");
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).expect("read fixture dir") {
        let entry = entry.expect("fixture dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            fs::create_dir_all(&to).expect("create fixture subdir");
            copy_dir(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy fixture file");
        }
    }
}

/// Join lines back, preserving the source's trailing-newline convention.
fn rejoin(lines: &[&str], source: &str) -> String {
    let mut out = lines.join("\n");
    if source.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The §4.6 child `PATH`: `stub_bin` first (so runner/tool names resolve to the exit-0
/// stubs, host-independent), then the minimal system dirs the engine's own `git`/`sh`
/// subprocesses need. Shared by [`Fixture`] and [`GroveFixture`].
fn child_path(stub_bin: &Path) -> String {
    let mut dirs = vec![stub_bin.to_path_buf()];
    dirs.extend(SYSTEM_PATH_DIRS.iter().map(PathBuf::from));
    std::env::join_paths(dirs)
        .expect("join child PATH")
        .to_string_lossy()
        .into_owned()
}

// ─── the grove-root (bare-root worktree) harness (§3.5) ──────────────────────────

/// The seed Elixir module a [`GroveFixture`] commits: a landmarked `hello/0` def, so an
/// overlay claim can anchor `lm:app-core` at a tracked code symbol (§4.5). Line 2 is the
/// landmark and line 3 the def, both stable under [`GroveFixture::stale_app_ex`] so the
/// anchor's `resolved` line never moves — only the digested body changes.
const GROVE_APP_EX: &str =
    "defmodule Acme.App do\n  # stele:landmark app-core\n  def hello do\n    :world\n  end\nend\n";

/// The same module with `hello`'s body mutated (`:world` → `:changed`): line 2/3 unchanged,
/// so `resolved` is stable and the rebuilt lock byte-matches, but the def's AST digest
/// diverges — the §4.5 freshness signal a worktree at this commit sees.
const GROVE_APP_EX_STALE: &str = "defmodule Acme.App do\n  # stele:landmark app-core\n  def hello do\n    :changed\n  end\nend\n";

/// A bare-root worktree layout (SPEC §3.5 grove home). A bare repo lives at `root/.git`, so
/// the git common dir is `root/.git` and its parent — the graph home — is `root/`, OUTSIDE
/// every work tree. Two sibling worktrees hang off it: `root/.trunk` (the `main` branch) and
/// `root/feature-x` (a second branch). One private graph at `root/.stele/` serves both and
/// survives worktree churn. Dropping the fixture deletes `root/` and the throwaway seed repo
/// the bare clone was made from.
pub struct GroveFixture {
    _dir: tempfile::TempDir,
    _seed: tempfile::TempDir,
    _stub_dir: tempfile::TempDir,
    stub_bin: PathBuf,
    /// The graph home — `root/`, the parent of the bare common dir.
    pub home: PathBuf,
    /// The `main` worktree, `root/.trunk`.
    pub trunk: PathBuf,
    /// The second worktree, `root/feature-x`.
    pub feature: PathBuf,
}

/// Build a bare-root worktree layout (§3.5): seed a normal repo, `git clone --bare` it to
/// `root/.git`, then add the `.trunk` (main) and `feature-x` sibling worktrees.
pub fn grove_root() -> GroveFixture {
    GroveFixture::new()
}

impl GroveFixture {
    pub fn new() -> Self {
        // A throwaway normal repo carrying the committed two-directory tree the bare clone
        // seeds from — a bare repo cannot `worktree add` without a branch to check out.
        let seed = tempfile::tempdir().expect("create seed dir");
        let seed_root = seed.path();
        run_git(seed_root, &["init", "-b", "main"]);
        write_file(&seed_root.join("apps/web/lib/app.ex"), GROVE_APP_EX);
        write_file(
            &seed_root.join("packages/shared/src/index.ts"),
            "export const version = 1;\n",
        );
        run_git(seed_root, &["add", "-A"]);
        run_git(
            seed_root,
            &[
                "-c",
                "user.email=stele-test@example.com",
                "-c",
                "user.name=stele test",
                "commit",
                "-m",
                "seed grove root",
            ],
        );

        let dir = tempfile::tempdir().expect("create root dir");
        let home = dir.path().to_path_buf();
        let bare = home.join(".git");
        // The bare repo IS `root/.git`, so the common dir's parent is `root/` — the graph home,
        // outside every work tree.
        run_git(
            &home,
            &[
                "clone",
                "--bare",
                seed_root.to_str().expect("utf-8 seed path"),
                bare.to_str().expect("utf-8 bare path"),
            ],
        );
        let trunk = home.join(".trunk");
        let feature = home.join("feature-x");
        run_git(
            &bare,
            &["worktree", "add", trunk.to_str().expect("utf-8"), "main"],
        );
        run_git(
            &bare,
            &[
                "worktree",
                "add",
                "-b",
                "feature-x",
                feature.to_str().expect("utf-8"),
                "main",
            ],
        );

        let stub_dir = tempfile::tempdir().expect("create stub-bin dir");
        let stub_bin = stub_dir.path().to_path_buf();
        write_stub_bins(&stub_bin);
        Self {
            _dir: dir,
            _seed: seed,
            _stub_dir: stub_dir,
            stub_bin,
            home,
            trunk,
            feature,
        }
    }

    /// Run the built binary from `dir` (a worktree the operator is working in), with the same
    /// §4.6 child-PATH discipline as [`Fixture::run`] so command resolution is host-independent.
    pub fn run_from(&self, dir: &Path, args: &[&str]) -> RunResult {
        let output = isolate_git(
            Command::new(BIN)
                .args(args)
                .current_dir(dir)
                .env("PATH", child_path(&self.stub_bin)),
        )
        .output()
        .expect("spawn stele binary");
        RunResult {
            code: output.status.code().expect("stele terminated by signal"),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        }
    }

    /// An absolute path under the graph home (`root/`).
    pub fn home_path(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    /// Write a file under the graph home (`root/`), creating parents — for authoring the
    /// overlay node sources that live off every work tree.
    pub fn write_home(&self, rel: &str, contents: &str) {
        write_file(&self.home_path(rel), contents);
    }

    /// Read a file under the graph home.
    pub fn read_home(&self, rel: &str) -> String {
        fs::read_to_string(self.home_path(rel)).unwrap_or_else(|e| panic!("read home {rel}: {e}"))
    }

    /// Overwrite `feature-x`'s `apps/web/lib/app.ex` with the digest-staled body and commit —
    /// advancing that worktree's HEAD past the watermark while `.trunk` stays put (§4.5).
    pub fn stale_app_ex(&self) {
        write_file(
            &self.feature.join("apps/web/lib/app.ex"),
            GROVE_APP_EX_STALE,
        );
        self.commit(&self.feature, "feature-x: mutate hello body");
    }

    /// `git add -A` + commit with inline identity in the worktree at `dir`.
    pub fn commit(&self, dir: &Path, message: &str) {
        run_git(dir, &["add", "-A"]);
        run_git(
            dir,
            &[
                "-c",
                "user.email=stele-test@example.com",
                "-c",
                "user.name=stele test",
                "commit",
                "-m",
                message,
            ],
        );
    }

    /// `git status --porcelain` in the worktree at `dir`, trimmed — empty means byte-clean.
    pub fn status(&self, dir: &Path) -> String {
        git_stdout_in(dir, &["status", "--porcelain"])
    }

    /// Remove the `feature-x` worktree (the disposable-worktree churn the shared home survives).
    pub fn remove_feature(&self) {
        run_git(
            &self.home.join(".git"),
            &[
                "worktree",
                "remove",
                "--force",
                self.feature.to_str().expect("utf-8"),
            ],
        );
    }
}

impl Default for GroveFixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `git` in `cwd`, asserting success — the grove harness's construction primitive.
fn run_git(cwd: &Path, args: &[&str]) {
    let output = isolate_git(Command::new("git").args(args).current_dir(cwd))
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} in {} failed:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `git` in `cwd`, asserting success and returning trimmed stdout.
fn git_stdout_in(cwd: &Path, args: &[&str]) -> String {
    let output = isolate_git(Command::new("git").args(args).current_dir(cwd))
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} in {} failed:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Write `contents` to `path`, creating parent directories.
fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, contents).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
