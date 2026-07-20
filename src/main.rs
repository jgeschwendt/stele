//! stele CLI entry point (SPEC §5.1). Dispatch and the process contract (§5.3)
//! live in `stele::cli`; `main` only forwards argv, catches a would-be panic as the
//! §5.3 internal-error exit (`3`), and forwards the exit code.

use stele::model::ExitCode;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = std::panic::catch_unwind(|| stele::cli::run(&args)).unwrap_or_else(|_| {
        eprintln!("stele: internal error (unexpected panic)");
        ExitCode::Internal as i32
    });
    std::process::exit(code);
}
