//! The one git port: every engine spawn of `git` is built here (§2.4 tracked scope, §4.5
//! history). Pathspecs are literal — stele's paths name files, so `:x` is no magic and
//! `[ab]` no glob — and an inherited glob or icase `GIT_*_PATHSPECS`, which git refuses
//! beside literal mode, is dropped. `check-ignore` reads pathnames and refuses literal
//! magic outright (exit 128), so it runs with the literal switch removed instead.

use crate::model::{Result, SteleError};
use std::io;
use std::path::Path;
use std::process::{Command, Output};

/// `git <cmd> <args>` in `dir`, the spawn's raw outcome — for callers whose failure is a
/// warning or a default, never an error.
pub fn output(dir: &Path, cmd: &str, args: &[&str]) -> io::Result<Output> {
    let mut git = Command::new("git");
    git.args(cmd.split(' ').chain(args.iter().copied()))
        .env_remove("GIT_GLOB_PATHSPECS")
        .env_remove("GIT_ICASE_PATHSPECS")
        .env_remove("GIT_NOGLOB_PATHSPECS")
        .current_dir(dir);
    match cmd {
        "check-ignore" => git.env_remove("GIT_LITERAL_PATHSPECS"),
        _ => git.env("GIT_LITERAL_PATHSPECS", "1"),
    };
    git.output()
}

/// [`output`], a spawn failure a §5.3 internal error naming `git <cmd>`.
pub fn run(dir: &Path, cmd: &str, args: &[&str]) -> Result<Output> {
    output(dir, cmd, args).map_err(|e| SteleError::internal(format!("run `git {cmd}`: {e}")))
}
