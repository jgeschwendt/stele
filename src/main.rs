//! stele CLI entry point (SPEC §5.1). Dispatch and the process contract (§5.3)
//! live in `stele::cli`; `main` only forwards argv, catches a would-be panic as the
//! §5.3 internal-error exit (`3`), and forwards the exit code.

use stele::model::ExitCode;

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

fn main() {
    // Rust ignores SIGPIPE, so a reader closing early (`stele root | head`) panics in
    // println! → exit 3. Restore SIG_DFL (SIGPIPE is 13 on every shipped target): the
    // process ends quietly by SIGPIPE, like any Unix filter (§5.3).
    #[cfg(unix)]
    unsafe {
        signal(13, 0);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = std::panic::catch_unwind(|| stele::cli::run(&args)).unwrap_or_else(|_| {
        eprintln!("stele: internal error (unexpected panic)");
        ExitCode::Internal as i32
    });
    std::process::exit(code);
}
