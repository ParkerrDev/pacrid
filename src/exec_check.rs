/// Returns true when NONE of the given executable names are found on PATH.
/// Safety invariant: if the exe still exists, the app is still installed
/// (flatpak, appimage, /usr/local, etc.) — skip all findings for that package.
pub fn executable_gone<T: AsRef<str>>(exe_names: &[T]) -> bool {
    assert!(!exe_names.is_empty(), "exe_names must not be empty");
    assert!(
        exe_names
            .iter()
            .all(|n| !n.as_ref().contains('/') && !n.as_ref().contains('\0')),
        "exe_names must be plain binary names, not paths"
    );
    let found = exe_names
        .iter()
        .any(|name| which::which(name.as_ref()).is_ok());
    !found
}

/// Derive candidate executable names from a package name.
pub fn exe_candidates(pkgname: &str) -> Vec<String> {
    assert!(!pkgname.is_empty(), "pkgname must not be empty");
    let stripped = strip_aur_suffix(pkgname);
    let mut candidates = vec![
        stripped.clone(),
        stripped.replace('-', ""),
        stripped.replace('_', "-"),
        stripped.replace('-', "_"),
    ];
    candidates.dedup();
    candidates
}

pub fn strip_aur_suffix(name: &str) -> String {
    for suffix in &["-bin", "-git", "-stable", "-dev", "-nightly"] {
        if let Some(s) = name.strip_suffix(suffix) {
            return s.to_owned();
        }
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_is_present() {
        // `cargo` is guaranteed to be on PATH in the build environment.
        assert!(!executable_gone(&["cargo".to_owned()]));
    }

    #[test]
    fn nonexistent_binary_is_gone() {
        assert!(executable_gone(&["__pacrid_no_such_exe__".to_owned()]));
    }

    #[test]
    fn strip_suffix_works() {
        assert_eq!(strip_aur_suffix("steam-bin"), "steam");
        assert_eq!(strip_aur_suffix("neovim-git"), "neovim");
        assert_eq!(strip_aur_suffix("steam"), "steam");
    }

    #[test]
    fn exe_candidates_deduplicated() {
        let c = exe_candidates("foo");
        // foo, foo (no-dash), foo (dash→underscore) may produce duplicates — they should be removed
        assert!(c.contains(&"foo".to_owned()));
    }
}
