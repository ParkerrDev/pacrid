/// Integration test: pacman hook stdin parsing.
/// Pipes package names to `pacrid hook --dry-run` and asserts clean exit.
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn hook_processes_packages_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pacrid"))
        .args(["hook", "--dry-run"])
        .env_remove("PACRID_IN_HOOK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn pacrid");

    // Write package names as the pacman hook would.
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(b"steam\nfoo\n").unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "pacrid hook must exit 0 even in dry-run\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn hook_exits_zero_when_recursion_guard_set() {
    let output = Command::new(env!("CARGO_BIN_EXE_pacrid"))
        .args(["hook"])
        .env("PACRID_IN_HOOK", "1")
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn pacrid");

    assert!(
        output.status.success(),
        "hook with PACRID_IN_HOOK set must exit 0"
    );
}

#[test]
fn hook_exits_zero_with_empty_stdin() {
    let output = Command::new(env!("CARGO_BIN_EXE_pacrid"))
        .args(["hook", "--dry-run"])
        .env_remove("PACRID_IN_HOOK")
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn pacrid");

    assert!(
        output.status.success(),
        "hook with empty stdin must exit 0"
    );
}
