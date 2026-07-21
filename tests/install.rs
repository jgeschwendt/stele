//! Distribution-scripts smoke test (scripts/{release,install,uninstall}.sh). Runs the
//! hermetic `tests/install_smoke.sh` — assemble a local artifact, install it from that
//! local dir (STELE_ARTIFACT_DIR, no network), assert `--version` runs, then uninstall.
//! The shell script owns the assertions; this wrapper just makes `cargo test` run it and
//! fails with the captured output on a non-zero exit.

use std::path::Path;
use std::process::Command;

#[test]
fn install_scripts_round_trip() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/install_smoke.sh");
    let root = env!("CARGO_MANIFEST_DIR");
    let output = Command::new("sh")
        .arg(script)
        .current_dir(Path::new(root))
        .output()
        .expect("spawn install_smoke.sh");
    assert!(
        output.status.success(),
        "install_smoke.sh failed (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
