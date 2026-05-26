use crate::exec_check::{exe_candidates, executable_gone, strip_aur_suffix};
use crate::scanners::{Category, Finding, Reason, ScanContext, Scanner};
use crate::util::compute_size;
use std::path::{Path, PathBuf};

pub struct NameHeuristicScanner;

impl Scanner for NameHeuristicScanner {
    fn name(&self) -> &'static str {
        "name_heuristic"
    }

    fn scan(&self, ctx: &ScanContext) -> anyhow::Result<Vec<Finding>> {
        assert!(
            !ctx.removed_packages.is_empty(),
            "scan called with no removed packages"
        );

        // Use ctx.home_dir — DO NOT use std::env::var("HOME") here.
        // The hook runs as root; $HOME is /root, not the user's actual home.
        let home = ctx.home_dir.to_string_lossy().into_owned();
        let xdg_config = format!("{home}/.config");
        let xdg_cache = format!("{home}/.cache");
        let xdg_data = format!("{home}/.local/share");
        let xdg_state = format!("{home}/.local/state");

        // Capacity heuristic: ~8 candidate paths per package per name variant.
        // Pre-reserving avoids reallocation as findings accumulate (Rule 3
        // partial compliance — bound the heap churn).
        let mut findings = Vec::with_capacity(ctx.removed_packages.len().saturating_mul(8));

        for pkg in &ctx.removed_packages {
            if ctx.config.ignore.packages.contains(pkg) {
                continue;
            }

            let exe_names = exe_candidates(pkg);
            let exe_gone = executable_gone(&exe_names);

            // If the exe is still alive, skip all findings for this package.
            if !exe_gone {
                tracing::debug!("skipping {pkg}: executable still present");
                continue;
            }

            // Use the package name as the ExecutableGone label (most readable).
            let exe_gone_name = pkg.to_owned();

            let stripped = strip_aur_suffix(pkg);
            let names_to_try = name_variants(&stripped);

            for name in &names_to_try {
                check_home_paths(
                    name,
                    pkg,
                    &exe_gone_name,
                    &home,
                    &xdg_config,
                    &xdg_cache,
                    &xdg_data,
                    &xdg_state,
                    &ctx.config,
                    &ctx.home_dir,
                    &mut findings,
                );
            }

            // Extra roots from config.
            for extra_root in &ctx.config.scan_paths_extra {
                for name in &names_to_try {
                    let candidate = extra_root.join(name);
                    let reasons = vec![
                        Reason::ExactNameMatch,
                        Reason::ExecutableGone(exe_gone_name.clone()),
                    ];
                    if let Some(f) = probe(
                        &candidate,
                        pkg,
                        reasons,
                        Category::Data,
                        &ctx.config,
                        &ctx.home_dir,
                    ) {
                        findings.push(f);
                    }
                }
            }
        }

        Ok(findings)
    }
}

fn name_variants(base: &str) -> Vec<String> {
    // Normalize separators to '-' so the Title-Case helpers see a single shape.
    let normalized = base.replace('_', "-");
    let mut v = vec![
        base.to_owned(),
        base.replace('-', "_"),
        normalized.clone(),
        base.replace('-', ""),
        // Vendor-style capitalizations: "brave-origin-nightly" →
        // "Brave-Origin-Nightly", "Brave_Origin_Nightly", "BraveOriginNightly".
        title_case_join(&normalized, "-"),
        title_case_join(&normalized, "_"),
        title_case_join(&normalized, ""),
    ];
    v.sort();
    v.dedup();
    v
}

/// Split `s` on '-', uppercase the first character of each word, then rejoin
/// with `joiner`. Used to produce CamelCase / Title-Case package-name variants.
fn title_case_join(s: &str, joiner: &str) -> String {
    s.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(joiner)
}

#[allow(clippy::too_many_arguments)]
fn check_home_paths(
    name: &str,
    pkg: &str,
    exe_gone_name: &str,
    home: &str,
    xdg_config: &str,
    xdg_cache: &str,
    xdg_data: &str,
    xdg_state: &str,
    config: &crate::config::Config,
    home_dir: &Path,
    findings: &mut Vec<Finding>,
) {
    let home_candidates: &[(&str, Category)] = &[
        (xdg_config, Category::Config),
        (xdg_cache, Category::Cache),
        (xdg_data, Category::Data),
        (xdg_state, Category::State),
    ];

    check_xdg_direct(
        name,
        pkg,
        exe_gone_name,
        home_candidates,
        config,
        home_dir,
        findings,
    );
    check_xdg_vendor_nested(
        name,
        pkg,
        exe_gone_name,
        home_candidates,
        config,
        home_dir,
        findings,
    );
    check_dotfiles(name, pkg, exe_gone_name, home, config, home_dir, findings);
    check_system_paths(name, pkg, config, home_dir, findings);
}

/// Direct XDG check: ~/.config/<name>, ~/.cache/<name>, etc.
fn check_xdg_direct(
    name: &str,
    pkg: &str,
    exe_gone_name: &str,
    candidates: &[(&str, Category)],
    config: &crate::config::Config,
    home_dir: &Path,
    findings: &mut Vec<Finding>,
) {
    for (base, category) in candidates {
        let path = PathBuf::from(base).join(name);
        let reasons = vec![
            Reason::ExactNameMatch,
            Reason::ExecutableGone(exe_gone_name.to_owned()),
        ];
        if let Some(f) = probe(&path, pkg, reasons, *category, config, home_dir) {
            findings.push(f);
        }
    }
}

/// One level deeper than XDG: catches `~/.config/BraveSoftware/Brave-Origin-Nightly`
/// where the immediate XDG child is a vendor name and the app data lives nested.
fn check_xdg_vendor_nested(
    name: &str,
    pkg: &str,
    exe_gone_name: &str,
    candidates: &[(&str, Category)],
    config: &crate::config::Config,
    home_dir: &Path,
    findings: &mut Vec<Finding>,
) {
    for (base, category) in candidates {
        let xdg_root = Path::new(base);
        let Ok(entries) = std::fs::read_dir(xdg_root) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            // file_type from DirEntry is cheap and reflects the symlink itself
            // on Linux, matching the never-follow invariant.
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let nested = entry.path().join(name);
            let reasons = vec![
                Reason::ExactNameMatch,
                Reason::ExecutableGone(exe_gone_name.to_owned()),
            ];
            if let Some(f) = probe(&nested, pkg, reasons, *category, config, home_dir) {
                findings.push(f);
            }
        }
    }
}

/// Legacy ~/.pkgname and ~/.pkgnamerc dotfiles.
fn check_dotfiles(
    name: &str,
    pkg: &str,
    exe_gone_name: &str,
    home: &str,
    config: &crate::config::Config,
    home_dir: &Path,
    findings: &mut Vec<Finding>,
) {
    let dot_name = format!("{home}/.{name}");
    let dot_rc = format!("{home}/.{name}rc");
    for (path_str, cat) in &[(dot_name, Category::Config), (dot_rc, Category::Config)] {
        let path = Path::new(path_str);
        let reasons = vec![
            Reason::ExactNameMatch,
            Reason::ExecutableGone(exe_gone_name.to_owned()),
        ];
        if let Some(f) = probe(path, pkg, reasons, *cat, config, home_dir) {
            findings.push(f);
        }
    }
}

/// System paths: /var/cache, /var/log, /etc — capped at Medium.
/// /var/lib — capped at Low (live state data, dangerous to auto-remove).
fn check_system_paths(
    name: &str,
    pkg: &str,
    config: &crate::config::Config,
    home_dir: &Path,
    findings: &mut Vec<Finding>,
) {
    let sys_medium: &[(String, Category)] = &[
        (format!("/var/cache/{name}"), Category::Cache),
        (format!("/var/log/{name}"), Category::Cache),
        (format!("/etc/{name}"), Category::Config),
        (format!("/etc/{name}.conf"), Category::Config),
        (format!("/etc/{name}.d"), Category::Config),
    ];
    for (path_str, cat) in sys_medium {
        let path = Path::new(path_str);
        if let Some(mut f) = probe(
            path,
            pkg,
            vec![Reason::ExactNameMatch],
            *cat,
            config,
            home_dir,
        ) {
            if f.confidence > crate::scanners::Confidence::Medium {
                f.confidence = crate::scanners::Confidence::Medium;
            }
            findings.push(f);
        }
    }

    let var_lib = format!("/var/lib/{name}");
    let path = Path::new(&var_lib);
    if let Some(mut f) = probe(
        path,
        pkg,
        vec![Reason::ExactNameMatch],
        Category::State,
        config,
        home_dir,
    ) {
        f.confidence = crate::scanners::Confidence::Low;
        findings.push(f);
    }
}

fn probe(
    path: &Path,
    pkg: &str,
    reasons: Vec<Reason>,
    category: Category,
    config: &crate::config::Config,
    home_dir: &Path,
) -> Option<Finding> {
    assert!(!reasons.is_empty(), "probe requires at least one reason");

    if config.ignore.paths.contains(&path.to_path_buf()) {
        return None;
    }

    // Safety invariant: never follow symlinks.
    path.symlink_metadata().ok()?;

    let size_bytes = compute_size(path);
    let confidence = crate::confidence::score(&reasons, path, size_bytes, home_dir);

    Some(Finding {
        path: path.to_path_buf(),
        size_bytes,
        package: pkg.to_owned(),
        confidence,
        reasons,
        category,
        file_count: 0,
    })
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
    use crate::config::Config;
    use crate::pacman::db::PacmanDb;
    use crate::scanners::ScanContext;
    use tempfile::TempDir;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn finds_config_cache_dirs() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_str().unwrap().to_owned();

        std::fs::create_dir_all(format!("{home}/.config/fakepkg")).unwrap();
        std::fs::create_dir_all(format!("{home}/.cache/fakepkg")).unwrap();

        let prev_home = std::env::var("HOME").unwrap_or_default();
        // SAFETY: setting env vars in tests is only safe in single-threaded test.
        unsafe { std::env::set_var("HOME", &home) };
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        unsafe { std::env::remove_var("XDG_CACHE_HOME") };
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        unsafe { std::env::remove_var("XDG_STATE_HOME") };

        let ctx = ScanContext {
            removed_packages: vec!["fakepkg".to_owned()],
            pacman_db: PacmanDb::default(),
            config: Config::default(),
            home_dir: std::path::PathBuf::from(&home),
        };

        let scanner = NameHeuristicScanner;
        // executable_gone("fakepkg") should return true since fakepkg is not on PATH.
        let findings = scanner.scan(&ctx).unwrap();

        // Restore HOME.
        unsafe { std::env::set_var("HOME", &prev_home) };

        let paths: Vec<_> = findings.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert!(
            paths.iter().any(|p| p.contains(".config/fakepkg")),
            "expected .config/fakepkg in findings, got: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains(".cache/fakepkg")),
            "expected .cache/fakepkg in findings, got: {paths:?}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn finds_vendor_namespaced_dir() {
        // Mirrors the brave case shape but uses a fake vendor/app combo so the
        // executable_gone guard succeeds regardless of what's on PATH.
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_str().unwrap().to_owned();
        let leaf = format!("{home}/.config/FakeVendorCo/Fake-Origin-Nightly");
        std::fs::create_dir_all(&leaf).unwrap();

        let prev_home = std::env::var("HOME").unwrap_or_default();
        // SAFETY: env mutation is single-threaded inside this test binary.
        unsafe { std::env::set_var("HOME", &home) };
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        unsafe { std::env::remove_var("XDG_CACHE_HOME") };
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        unsafe { std::env::remove_var("XDG_STATE_HOME") };

        let ctx = ScanContext {
            removed_packages: vec!["fake-origin-nightly-bin".to_owned()],
            pacman_db: PacmanDb::default(),
            config: Config::default(),
            home_dir: std::path::PathBuf::from(&home),
        };

        let findings = NameHeuristicScanner.scan(&ctx).unwrap();
        unsafe { std::env::set_var("HOME", &prev_home) };

        let paths: Vec<_> = findings.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert!(
            paths
                .iter()
                .any(|p| p.contains("FakeVendorCo/Fake-Origin-Nightly")),
            "expected vendor-nested Fake dir in findings, got: {paths:?}"
        );
    }

    #[test]
    fn name_variants_include_camelcase() {
        let v = name_variants("brave-origin-nightly");
        assert!(v.iter().any(|s| s == "Brave-Origin-Nightly"), "{v:?}");
        assert!(v.iter().any(|s| s == "BraveOriginNightly"), "{v:?}");
        assert!(v.iter().any(|s| s == "brave-origin-nightly"), "{v:?}");
    }
}
