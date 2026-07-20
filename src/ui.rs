//! pacman-flavoured terminal output.
//!
//! pacrid's hook prints in the middle of a pacman transaction, sandwiched
//! between pacman's own `:: Running post-transaction hooks...` lines. Anything
//! that doesn't follow pacman's conventions reads as a foreign object wedged
//! into the transaction, so this module mirrors them: `::` in bold blue
//! followed by a bold message, ` ->` for sub-items, `warning:`/`error:` in
//! yellow and red.

use std::os::unix::io::AsRawFd;
use std::path::Path;

const BOLD: &str = "\x1b[1m";
const BLUE: &str = "\x1b[1;34m";
const GREEN: &str = "\x1b[1;32m";
const YELLOW: &str = "\x1b[1;33m";
const RED: &str = "\x1b[1;31m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Fallback when the terminal size can't be read (piped output, no tty).
/// 80 is the conservative POSIX default; pacman assumes the same.
const FALLBACK_WIDTH: usize = 80;

/// Colour is suppressed when output isn't a terminal or `NO_COLOR` is set.
/// Checked per call rather than cached because the hook's stdout is swapped
/// for `/dev/tty` partway through the process's life (see `crate::tty`).
fn colored() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    // SAFETY: isatty(3) on stdout's descriptor, which the process owns for its
    // entire lifetime and which is therefore always valid for this call.
    unsafe { libc::isatty(std::io::stdout().as_raw_fd()) == 1 }
}

fn paint(code: &str, text: &str) -> String {
    if colored() {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

/// A top-level `:: message` line, matching pacman's own transaction output.
pub fn header(message: &str) -> String {
    assert!(!message.is_empty(), "header message must not be empty");
    format!("{} {}", paint(BLUE, "::"), paint(BOLD, message))
}

/// A ` -> item` sub-line, matching pacman's optional-dependency listings.
pub fn item(message: &str) -> String {
    format!("{} {message}", paint(BLUE, " ->"))
}

pub fn success(message: &str) -> String {
    format!("{} {}", paint(GREEN, "::"), paint(BOLD, message))
}

pub fn warning(message: &str) -> String {
    format!("{} {message}", paint(YELLOW, "warning:"))
}

pub fn error(message: &str) -> String {
    format!("{} {message}", paint(RED, "error:"))
}

pub fn dim(text: &str) -> String {
    paint(DIM, text)
}

pub fn bold(text: &str) -> String {
    paint(BOLD, text)
}

pub fn green(text: &str) -> String {
    paint(GREEN, text)
}

pub fn yellow(text: &str) -> String {
    paint(YELLOW, text)
}

pub fn red(text: &str) -> String {
    paint(RED, text)
}

/// `n` followed by `noun`, pluralised with a trailing "s".
/// Exists because "removed 1 paths" is the kind of detail that makes a tool
/// feel unfinished.
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Replaces a leading home directory with `~` for display only.
///
/// Derived from the path itself rather than `$HOME`: the hook runs as root
/// over every user's home, so the environment's idea of home is wrong here.
pub fn abbreviate_home(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("/root") {
        return format!("~{rest}");
    }
    let Some(rest) = s.strip_prefix("/home/") else {
        return s.into_owned();
    };
    match rest.split_once('/') {
        Some((_user, tail)) => format!("~/{tail}"),
        None => s.into_owned(),
    }
}

/// Terminal width in columns, or `FALLBACK_WIDTH` when it can't be determined.
pub fn terminal_width() -> usize {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes a winsize struct through the pointer, which
    // points at a fully initialised local. A non-zero return means the ioctl
    // failed and the struct is ignored.
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &raw mut size) == 0 };
    let width = if ok && size.ws_col > 0 {
        usize::from(size.ws_col)
    } else {
        FALLBACK_WIDTH
    };
    // Column budgets are derived from this by subtraction; a zero width would
    // silently collapse every path to the minimum.
    assert!(width > 0, "terminal width must be positive");
    width
}

/// Shortens `text` to `max` columns, cutting the middle and keeping both ends.
///
/// Middle-out because the informative parts of a path are its root and its
/// leaf; the bytes in between are the ones you can afford to lose. Operates on
/// chars so a multi-byte path can never be split mid-character.
pub fn truncate_middle(text: &str, max: usize) -> String {
    let len = text.chars().count();
    if len <= max || max < 5 {
        return text.to_owned();
    }
    // Reserve one column for the ellipsis, then split the rest so the tail
    // (filename) keeps the larger half when the budget is odd.
    let budget = max.saturating_sub(1);
    let head = budget / 2;
    let tail = budget.saturating_sub(head);
    let prefix: String = text.chars().take(head).collect();
    let suffix: String = text.chars().skip(len.saturating_sub(tail)).collect();
    format!("{prefix}…{suffix}")
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
    use std::path::PathBuf;

    #[test]
    fn count_pluralises() {
        assert_eq!(count(1, "path"), "1 path");
        assert_eq!(count(0, "path"), "0 paths");
        assert_eq!(count(3, "path"), "3 paths");
    }

    #[test]
    fn abbreviates_user_home() {
        assert_eq!(
            abbreviate_home(&PathBuf::from("/home/aphunt/.local/share/lutris")),
            "~/.local/share/lutris"
        );
        assert_eq!(
            abbreviate_home(&PathBuf::from("/root/.cache/x")),
            "~/.cache/x"
        );
    }

    #[test]
    fn leaves_non_home_paths_alone() {
        assert_eq!(
            abbreviate_home(&PathBuf::from("/etc/lutris")),
            "/etc/lutris"
        );
        assert_eq!(abbreviate_home(&PathBuf::from("/home")), "/home");
    }

    #[test]
    fn truncate_keeps_both_ends() {
        let out = truncate_middle("~/.local/share/some-very-long-app-name", 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.starts_with("~/.local"), "{out}");
        assert!(out.ends_with("name"), "{out}");
    }

    #[test]
    fn truncate_is_a_noop_when_it_fits() {
        assert_eq!(truncate_middle("~/.cache/x", 40), "~/.cache/x");
    }

    #[test]
    fn truncate_never_splits_a_multibyte_char() {
        // Every char is 3 bytes; a byte-based slice would panic or corrupt.
        let out = truncate_middle("日本語のとても長いパス名です", 9);
        assert_eq!(out.chars().count(), 9);
    }
}
