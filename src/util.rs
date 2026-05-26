use humansize::{format_size, BINARY};
use std::path::Path;

pub fn format_bytes(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

/// Compute total size of a path (file or directory) without following symlinks.
/// Safety invariant: never follow symlinks.
pub fn compute_size(path: &Path) -> u64 {
    assert!(!path.as_os_str().is_empty(), "path must not be empty");

    // Use symlink_metadata so we never follow symlinks.
    let Ok(meta) = path.symlink_metadata() else {
        return 0;
    };

    if meta.is_symlink() || meta.is_file() {
        return meta.len();
    }

    if meta.is_dir() {
        let mut total: u64 = 0;
        let walker = walkdir::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok);

        for entry in walker {
            // Count every entry including dirs themselves (inode overhead).
            if let Ok(m) = entry.path().symlink_metadata() {
                total = total.saturating_add(m.len());
            }
        }
        return total;
    }

    0
}

/// Expand a leading `~` to the current user's home directory.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

/// Resolve XDG variables using a known home directory (no env-var reads).
/// Always call this instead of `expand_xdg` in scanner code — the hook runs
/// as root so `$HOME` in the environment is `/root`, not the user's actual home.
pub fn expand_xdg_with_home(path: &str, home: &std::path::Path) -> std::path::PathBuf {
    assert!(!path.is_empty(), "path template must not be empty");
    let home_str = home.to_string_lossy();
    let result = path
        .replace("$HOME", &home_str)
        .replace("$XDG_CONFIG_HOME", &format!("{home_str}/.config"))
        .replace("$XDG_CACHE_HOME", &format!("{home_str}/.cache"))
        .replace("$XDG_DATA_HOME", &format!("{home_str}/.local/share"))
        .replace("$XDG_STATE_HOME", &format!("{home_str}/.local/state"));
    std::path::PathBuf::from(result)
}

/// Resolve XDG environment variables using env vars (for non-hook code only).
pub fn expand_xdg(path: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    expand_xdg_with_home(path, std::path::Path::new(&home))
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

    #[test]
    fn format_bytes_human_readable() {
        let s = format_bytes(1_073_741_824);
        assert!(s.contains("GiB"), "expected GiB, got: {s}");
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn compute_size_nonexistent_returns_zero() {
        assert_eq!(compute_size(Path::new("/nonexistent/path/abc")), 0);
    }
}
