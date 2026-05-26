#![allow(clippy::unreadable_literal)]

use crate::exec_check::{executable_gone, strip_aur_suffix};
use crate::scanners::{Category, Finding, Reason, ScanContext, Scanner};
use crate::util::{compute_size, expand_xdg_with_home};
use std::path::{Path, PathBuf};

// Generated at build time from xdg-unused-data JSON files.
// Falls back to an empty map if build.rs did not produce the file (e.g., no network).
include!(concat!(env!("OUT_DIR"), "/xdg_db.rs"));

pub struct XdgDbScanner;

impl Scanner for XdgDbScanner {
    fn name(&self) -> &'static str {
        "xdg_db"
    }

    fn scan(&self, ctx: &ScanContext) -> anyhow::Result<Vec<Finding>> {
        assert!(
            !ctx.removed_packages.is_empty(),
            "scan called with no removed packages"
        );

        // Capacity heuristic: most XDG entries declare 1-4 locations.
        // Pre-reserve to bound heap churn during the scan.
        let mut findings = Vec::with_capacity(ctx.removed_packages.len().saturating_mul(4));

        for pkg in &ctx.removed_packages {
            if ctx.config.ignore.packages.contains(pkg) {
                continue;
            }

            let Some(entry) = lookup_xdg_entry(pkg) else {
                continue;
            };

            let exe_gone = executable_gone(entry.executables);
            if !exe_gone {
                tracing::debug!(
                    "skipping {pkg}: executable still present ({:?})",
                    entry.executables
                );
                continue;
            }

            for location in entry.locations {
                let path = expand_xdg_with_home(location.path, &ctx.home_dir);
                process_path(
                    path,
                    pkg,
                    entry.executables,
                    exe_gone,
                    &ctx.config,
                    &ctx.home_dir,
                    &mut findings,
                );
            }
        }

        Ok(findings)
    }
}

fn lookup_xdg_entry(pkg: &str) -> Option<&'static XdgEntry> {
    let lower = pkg.to_lowercase();
    let stripped = strip_aur_suffix(&lower);

    // Try exact match first, then stripped suffix.
    XDB_MAP
        .get(lower.as_str())
        .or_else(|| XDB_MAP.get(stripped.as_str()))
}

fn process_path(
    path: PathBuf,
    pkg: &str,
    executables: &[&str],
    exe_gone: bool,
    config: &crate::config::Config,
    home_dir: &Path,
    findings: &mut Vec<Finding>,
) {
    if config.ignore.paths.contains(&path) {
        return;
    }

    // Safety invariant: never follow symlinks for size.
    if path.symlink_metadata().is_err() {
        return; // path does not exist
    }

    let size_bytes = compute_size(&path);
    let category = categorize(&path, home_dir);

    let mut reasons = vec![Reason::InXdgDatabase];
    if exe_gone {
        if let Some(exe) = executables.first() {
            reasons.push(Reason::ExecutableGone((*exe).to_owned()));
        }
    }

    let confidence = crate::confidence::score(&reasons, &path, size_bytes, home_dir);

    findings.push(Finding {
        path,
        size_bytes,
        package: pkg.to_owned(),
        confidence,
        reasons,
        category,
        file_count: 0,
    });
}

fn categorize(path: &Path, home_dir: &Path) -> Category {
    let s = path.to_string_lossy();
    let home = home_dir.to_string_lossy();
    if s.starts_with(&format!("{home}/.config")) || s.starts_with("/etc/") {
        Category::Config
    } else if s.starts_with(&format!("{home}/.cache")) || s.starts_with("/var/cache/") {
        Category::Cache
    } else if s.starts_with(&format!("{home}/.local/share")) || s.starts_with("/usr/share/") {
        Category::Data
    } else if s.starts_with(&format!("{home}/.local/state")) || s.starts_with("/var/lib/") {
        Category::State
    } else if s.starts_with("/etc/") || s.starts_with("/var/") {
        Category::SystemOrphan
    } else {
        Category::Data
    }
}

/// Print xdg-db entry for debugging (`pacrid db check <pkgname>`).
pub fn print_db_entry(pkgname: &str) {
    match XDB_MAP.get(pkgname.to_lowercase().as_str()) {
        Some(entry) => {
            println!("xdg-db entry for '{pkgname}':");
            println!("  name:        {}", entry.name);
            println!("  executables: {:?}", entry.executables);
            println!("  locations:");
            for loc in entry.locations {
                println!("    {}", loc.path);
            }
        }
        None => {
            println!("No xdg-db entry found for '{pkgname}'.");
        }
    }
}
