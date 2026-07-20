use std::os::unix::io::RawFd;

const STDOUT_FD: RawFd = 1;
const STDERR_FD: RawFd = 2;

/// Points stdout and stderr at the controlling terminal for as long as the
/// guard lives, restoring the originals on drop.
///
/// Why this exists: pacman captures a hook's stdout and stderr so it can fold
/// them into its own log, which means neither is a terminal while the
/// `PostTransaction` hook runs. The interactive review renders to stderr
/// (inquire) and stdout (the per-package headers), so without this redirect the
/// prompt is drawn into pacman's log pipe — invisible — while still blocking on
/// keystrokes, because crossterm reads keys from `/dev/tty` regardless of where
/// stdin points. Redirecting the two output streams is what makes the prompt
/// visible to the human sitting at the terminal.
pub struct TtyRedirect {
    saved_stdout: RawFd,
    saved_stderr: RawFd,
    tty: RawFd,
}

impl TtyRedirect {
    /// Returns a guard when a controlling terminal exists and both streams were
    /// successfully redirected, `None` otherwise. `None` means the caller must
    /// fall back to non-interactive behaviour — there is no human to ask.
    pub fn acquire() -> Option<Self> {
        // Flush anything already buffered before the descriptors move, so
        // pending log lines land in pacman's log rather than on the terminal.
        flush_std_streams();

        let tty = open_tty()?;

        let (saved_stdout, saved_stderr) = dup_std_streams();
        if saved_stdout < 0 || saved_stderr < 0 {
            close_fd(tty);
            close_if_valid(saved_stdout);
            close_if_valid(saved_stderr);
            return None;
        }

        if !point_std_streams_at(tty) {
            restore(saved_stdout, saved_stderr);
            close_fd(tty);
            return None;
        }

        Some(Self {
            saved_stdout,
            saved_stderr,
            tty,
        })
    }
}

impl Drop for TtyRedirect {
    fn drop(&mut self) {
        flush_std_streams();
        restore(self.saved_stdout, self.saved_stderr);
        close_fd(self.tty);
    }
}

/// Duplicates stdout and stderr so they can be restored later. A negative
/// value in either slot means the duplication failed; the caller checks.
fn dup_std_streams() -> (RawFd, RawFd) {
    // SAFETY: dup(2) on the two standard descriptors, which this process owns
    // for its entire lifetime. Both returns are checked by the caller before
    // any use, and closed by the guard's Drop.
    unsafe { (libc::dup(STDOUT_FD), libc::dup(STDERR_FD)) }
}

/// Points stdout and stderr at `tty`. Returns false if either failed.
fn point_std_streams_at(tty: RawFd) -> bool {
    // SAFETY: dup2(2) with a source descriptor already validated by open_tty
    // onto the two standard output descriptors. Both targets are restored in
    // Drop from the copies dup_std_streams took beforehand.
    unsafe { libc::dup2(tty, STDOUT_FD) >= 0 && libc::dup2(tty, STDERR_FD) >= 0 }
}

/// Opens `/dev/tty` read-write and confirms it is a terminal. Returns `None`
/// when the process has no controlling terminal — a cron job, a GUI package
/// manager, or a hook run from a detached script.
fn open_tty() -> Option<RawFd> {
    // SAFETY: open(2) with a NUL-terminated literal path and no O_CREAT, so no
    // mode argument is required. The returned descriptor is checked for
    // validity before use and closed by the guard's Drop.
    let fd = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if fd < 0 {
        return None;
    }
    // SAFETY: isatty(3) on the descriptor just returned by open(2).
    let is_tty = unsafe { libc::isatty(fd) == 1 };
    if !is_tty {
        close_fd(fd);
        return None;
    }
    Some(fd)
}

fn restore(saved_stdout: RawFd, saved_stderr: RawFd) {
    // SAFETY: dup2(2) restoring the saved duplicates onto the standard
    // descriptors, then closing the now-redundant copies. Failures here are
    // unrecoverable and deliberately ignored — the process is about to exit.
    unsafe {
        libc::dup2(saved_stdout, STDOUT_FD);
        libc::dup2(saved_stderr, STDERR_FD);
    }
    close_fd(saved_stdout);
    close_fd(saved_stderr);
}

fn close_fd(fd: RawFd) {
    // SAFETY: close(2) on a descriptor this module opened or duplicated.
    unsafe { libc::close(fd) };
}

fn close_if_valid(fd: RawFd) {
    if fd >= 0 {
        close_fd(fd);
    }
}

fn flush_std_streams() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    // Runs under `cargo test`, where stdout is a pipe and there is usually no
    // controlling terminal. Either outcome is correct; what must not happen is
    // a panic or a corrupted stdout for the rest of the test binary.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn acquire_is_safe_without_a_terminal() {
        {
            let _guard = TtyRedirect::acquire();
        }
        println!("stdout still works after the guard is dropped");
    }
}
