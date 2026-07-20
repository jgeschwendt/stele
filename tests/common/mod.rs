#![allow(dead_code)]
//! Shared harness for the oracle integration tests (SPEC §5.1 pipeline, §5.3 process
//! contract). Each test owns a fresh temp copy of a fixture, wrapped in a real git
//! repo so scan scope (§2.4, VCS-tracked files only) and freshness (§4.5, verified
//! {sha}) have a history to read. The built binary is invoked via CARGO_BIN_EXE_stele
//! with `current_dir` set to that copy — no test depends on execution order.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `stele` binary Cargo built for this test run.
pub const BIN: &str = env!("CARGO_BIN_EXE_stele");

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

/// A per-test working tree: a temp directory that is also a git repo. Dropping it
/// deletes the directory.
pub struct Fixture {
    _dir: tempfile::TempDir,
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

    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().to_path_buf();
        let fixture = Self { _dir: dir, root };
        fixture.git(&["init", "-b", "main"]);
        fixture
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

    /// Run the built binary in the working tree, capturing status + both streams.
    pub fn run(&self, args: &[&str]) -> RunResult {
        let output = Command::new(BIN)
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("spawn stele binary");
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
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
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
