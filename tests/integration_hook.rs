/// Integration test: pacman hook stdin parsing.
/// Pipes package names to `pacrid hook --dry-run` and asserts clean exit.
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn hook_processes_packages_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pacrid"))
        .args(["hook", "--dry-run"])
        .env_remove("PACRID_IN_HOOK")
        // The hook scans the *real* user homes on this machine, so "steam" can
        // produce genuine findings on a developer's box. If a controlling
        // terminal is also available (cargo test under a pty), the review would
        // open a prompt and block this test forever. The suppressor is what
        // makes the test independent of both the machine's state and its tty.
        .env("PACRID_NO_PROMPT", "1")
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
        .env("PACRID_NO_PROMPT", "1")
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn pacrid");

    assert!(output.status.success(), "hook with empty stdin must exit 0");
}

/// The prompt suppressor must win even when a package genuinely has leftovers
/// and a terminal is attached — this is the guarantee unattended pacman runs
/// depend on, and the reason the tests above can't hang.
#[test]
fn suppressed_prompt_never_blocks() {
    let start = std::time::Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_pacrid"))
        .args(["hook", "--dry-run"])
        .env_remove("PACRID_IN_HOOK")
        .env("PACRID_NO_PROMPT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn pacrid");

    // "kilo" is arbitrary; whether it has leftovers here is irrelevant, since
    // the assertion is about never waiting for input either way.
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(b"kilo\n").unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "suppressed hook must exit 0");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(30),
        "hook blocked — the prompt suppressor did not take effect"
    );
}
