use crate::scanners::{Confidence, Reason};
use std::path::Path;

const LARGE_BYTES: u64 = 1_000_000_000; // 1 GB

/// Compute the confidence for a set of reasons on a given path and size.
/// `home_dir` is the real user's home (not necessarily `$HOME` — the hook runs as root).
/// Safety invariant: items > 1 GB always force Low regardless of other signals,
/// and /var/lib paths are always Low regardless of other signals.
pub fn score(reasons: &[Reason], path: &Path, size_bytes: u64, home_dir: &Path) -> Confidence {
    assert!(
        !reasons.is_empty(),
        "reasons must not be empty to score confidence"
    );

    // Invariant: force Low for huge items — always require human review.
    if size_bytes > LARGE_BYTES {
        return Confidence::Low;
    }

    // Invariant: state paths are always Low.
    if is_state_path(path) {
        return Confidence::Low;
    }

    let has_xdg_db = reasons.iter().any(|r| matches!(r, Reason::InXdgDatabase));
    let has_exe_gone = reasons
        .iter()
        .any(|r| matches!(r, Reason::ExecutableGone(_)));
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
        assert_eq!(
            score(&reasons, &p("/home/u/.steam"), 100, &home_dir()),
            Confidence::High
        );
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
        assert_eq!(
            score(&reasons, &p("/etc/steam"), 100, &home_dir()),
            Confidence::Medium
        );
    }

    #[test]
    fn low_substring_match_alone() {
        let reasons = vec![Reason::PkgnameSubstringMatch];
        assert_eq!(
            score(
                &reasons,
                &p("/home/u/.config/com.valve.steam"),
                100,
                &home_dir()
            ),
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
        assert_eq!(
            score(&reasons, &p("/home/u/.steam"), size, &home_dir()),
            Confidence::Low
        );
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

    // ─ Property tests ─
    //
    // These tests assert invariants that must hold across the entire input
    // space of `score`, not just hand-picked cases. proptest generates
    // hundreds of pseudo-random inputs per run and shrinks failures down to
    // a minimal counter-example.
    //
    // The whole block is gated out under Miri: proptest spawns subprocesses
    // (rusty-fork) to isolate each case, and Miri cannot simulate fork().
    #[cfg(not(miri))]
    use proptest::prelude::*;

    #[cfg(not(miri))]
    fn any_reason() -> impl Strategy<Value = Reason> {
        prop_oneof![
            Just(Reason::InXdgDatabase),
            Just(Reason::ExecutableGone("x".to_owned())),
            Just(Reason::ExactNameMatch),
            Just(Reason::PkgnameSubstringMatch),
            Just(Reason::PacmanOrphan),
            Just(Reason::UserAllowlist),
        ]
    }

    #[cfg(not(miri))]
    fn any_path() -> impl Strategy<Value = String> {
        // Generate a path with at least one segment so it parses as absolute.
        // Mix in known-special prefixes so the invariants are exercised.
        prop_oneof![
            Just("/home/u/.config/foo".to_owned()),
            Just("/home/u/.cache/bar".to_owned()),
            Just("/var/lib/baz".to_owned()),
            Just("/opt/x".to_owned()),
            Just("/srv/y".to_owned()),
            Just("/etc/z".to_owned()),
            Just("/usr/share/w".to_owned()),
            "[a-z]{1,8}".prop_map(|s| format!("/{s}/{s}")),
        ]
    }

    #[cfg(not(miri))]
    proptest! {
        /// Files larger than 1 GB must ALWAYS score Low — that is a hard
        /// safety floor independent of any other signal.
        #[test]
        fn property_huge_files_always_low(
            reasons in proptest::collection::vec(any_reason(), 1..6),
            path in any_path(),
            extra_size in 1u64..1_000_000_u64,
        ) {
            let size = LARGE_BYTES.saturating_add(extra_size);
            let c = score(&reasons, Path::new(&path), size, &home_dir());
            prop_assert_eq!(c, Confidence::Low);
        }

        /// Paths under /var/lib, /opt, /srv must ALWAYS score Low.
        #[test]
        fn property_state_paths_always_low(
            reasons in proptest::collection::vec(any_reason(), 1..6),
            size in 0u64..LARGE_BYTES,
            suffix in "[a-z]{1,16}",
        ) {
            for prefix in &["/var/lib/", "/opt/", "/srv/"] {
                let path = format!("{prefix}{suffix}");
                let c = score(&reasons, Path::new(&path), size, &home_dir());
                prop_assert_eq!(c, Confidence::Low, "path: {}", path);
            }
        }

        /// Confidence never exceeds Medium for an empty-of-strong-signals
        /// reason set (UserAllowlist + PkgnameSubstringMatch only).
        #[test]
        fn property_weak_reasons_never_high(
            path in any_path(),
            size in 0u64..LARGE_BYTES,
        ) {
            let weak = vec![Reason::PkgnameSubstringMatch, Reason::UserAllowlist];
            let c = score(&weak, Path::new(&path), size, &home_dir());
            prop_assert!(c <= Confidence::Medium);
        }
    }
}
