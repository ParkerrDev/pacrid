use crate::scanners::{Confidence, Reason};
use std::path::Path;

const LARGE_BYTES: u64 = 1_000_000_000; // 1 GB

/// Compute the confidence for a set of reasons on a given path and size.
/// `home_dir` is the real user's home (not necessarily `$HOME` — the hook runs as root).
/// Safety invariant: items > 1 GB always force Low regardless of other signals,
/// and /var/lib paths are always Low regardless of other signals.
pub fn score(reasons: &[Reason], path: &Path, size_bytes: u64, home_dir: &Path) -> Confidence {
    assert!(!reasons.is_empty(), "reasons must not be empty to score confidence");

    // Invariant: force Low for huge items — always require human review.
    if size_bytes > LARGE_BYTES {
        return Confidence::Low;
    }

    // Invariant: state paths are always Low.
    if is_state_path(path) {
        return Confidence::Low;
    }

    let has_xdg_db = reasons.iter().any(|r| matches!(r, Reason::InXdgDatabase));
    let has_exe_gone = reasons.iter().any(|r| matches!(r, Reason::ExecutableGone(_)));
    let has_exact = reasons.iter().any(|r| matches!(r, Reason::ExactNameMatch));
    let has_orphan = reasons.iter().any(|r| matches!(r, Reason::PacmanOrphan));
    let is_home_subpath = is_home_config_path(path, home_dir);

    // High: (InXdgDatabase AND ExecutableGone) OR (ExactNameMatch AND ExecutableGone AND home config path)
    if (has_xdg_db && has_exe_gone) || (has_exact && has_exe_gone && is_home_subpath) {
        return Confidence::High;
    }

    // Medium: ExactNameMatch alone, PacmanOrphan in /etc, or InXdgDatabase without exe check
    if has_exact || (has_orphan && is_etc_path(path)) || has_xdg_db {
        return Confidence::Medium;
    }

    Confidence::Low
}

fn is_home_config_path(path: &Path, home_dir: &Path) -> bool {
    // DO NOT read $HOME from env — caller must pass the real home.
    let s = path.to_string_lossy();
    let home = home_dir.to_string_lossy();
    if home.is_empty() {
        return false;
    }
    s.starts_with(&format!("{home}/.config"))
        || s.starts_with(&format!("{home}/.cache"))
        || s.starts_with(&format!("{home}/.local/share"))
        || s.starts_with(&format!("{home}/.local/state"))
}

fn is_state_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.starts_with("/var/lib/") || s.starts_with("/srv/") || s.starts_with("/opt/")
}

fn is_etc_path(path: &Path) -> bool {
    path.starts_with("/etc/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn home_dir() -> PathBuf {
        p("/home/u")
    }

    #[test]
    fn high_xdg_db_plus_exe_gone() {
        let reasons = vec![
            Reason::InXdgDatabase,
            Reason::ExecutableGone("steam".to_owned()),
        ];
        // xdg_db + exe_gone is always High regardless of path.
        assert_eq!(score(&reasons, &p("/home/u/.steam"), 100, &home_dir()), Confidence::High);
    }

    #[test]
    fn high_exact_plus_exe_gone_home_config() {
        let reasons = vec![
            Reason::ExactNameMatch,
            Reason::ExecutableGone("steam".to_owned()),
        ];
        let path = p("/home/u/.config/steam");
        assert_eq!(score(&reasons, &path, 100, &home_dir()), Confidence::High);
    }

    #[test]
    fn medium_exact_name_alone() {
        let reasons = vec![Reason::ExactNameMatch];
        assert_eq!(
            score(&reasons, &p("/home/u/.config/foo"), 100, &home_dir()),
            Confidence::Medium
        );
    }

    #[test]
    fn medium_pacman_orphan_in_etc() {
        let reasons = vec![Reason::PacmanOrphan];
        assert_eq!(score(&reasons, &p("/etc/steam"), 100, &home_dir()), Confidence::Medium);
    }

    #[test]
    fn low_substring_match_alone() {
        let reasons = vec![Reason::PkgnameSubstringMatch];
        assert_eq!(
            score(&reasons, &p("/home/u/.config/com.valve.steam"), 100, &home_dir()),
            Confidence::Low
        );
    }

    #[test]
    fn large_item_always_low() {
        let reasons = vec![
            Reason::InXdgDatabase,
            Reason::ExecutableGone("steam".to_owned()),
        ];
        let size = LARGE_BYTES + 1;
        assert_eq!(score(&reasons, &p("/home/u/.steam"), size, &home_dir()), Confidence::Low);
    }

    #[test]
    fn var_lib_always_low() {
        let reasons = vec![
            Reason::InXdgDatabase,
            Reason::ExecutableGone("steam".to_owned()),
        ];
        assert_eq!(
            score(&reasons, &p("/var/lib/steam"), 100, &home_dir()),
            Confidence::Low
        );
    }
}
