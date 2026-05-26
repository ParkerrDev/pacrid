use crate::config::AutoConfirmLevel;
use crate::scanners::{Confidence, Finding, Reason};
use crate::util::format_bytes;
use dialoguer::MultiSelect;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// Directories we never collapse to even if fully orphaned — too dangerous.
const COLLAPSE_NEVER: &[&str] = &[
    "/",
    "/home",
    "/root",
    "/etc",
    "/usr",
    "/var",
    "/boot",
    "/proc",
    "/sys",
    "/run",
    "/dev",
    "/tmp",
    "/usr/lib",
    "/usr/share",
    "/usr/bin",
    "/usr/include",
    "/var/lib",
    "/var/log",
    "/var/cache",
];

// A path must have at least this many components before we'll collapse to it.
// Prevents collapsing to shallow roots like /usr/lib (3 components).
const COLLAPSE_MIN_COMPONENTS: usize = 5;

/// Group findings by package, sort, then present an interactive review.
/// Returns the paths the user confirmed for deletion.
pub fn interactive_review(
    mut findings: Vec<Finding>,
    auto_confirm_level: &AutoConfirmLevel,
    non_interactive: bool,
) -> anyhow::Result<Vec<PathBuf>> {
    assert!(!findings.is_empty(), "interactive_review called with no findings");

    // Collapse individual files under fully-orphaned directories into one entry.
    findings = collapse_to_dirs(findings);

    // Sort: confidence DESC, size DESC.
    findings.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| b.size_bytes.cmp(&a.size_bytes))
    });

    if non_interactive || !is_interactive() {
        return Ok(auto_select(&findings, auto_confirm_level));
    }

    let by_package = group_by_package(&findings);
    let mut confirmed = Vec::new();

    for (pkg, pkg_findings) in &by_package {
        let selected = review_package(pkg, pkg_findings, auto_confirm_level)?;
        confirmed.extend(selected);
    }

    Ok(confirmed)
}

fn review_package(
    pkg: &str,
    findings: &[Finding],
    auto_confirm_level: &AutoConfirmLevel,
) -> anyhow::Result<Vec<PathBuf>> {
    assert!(!findings.is_empty(), "review_package called with no findings for {pkg}");

    println!("\nPackage: {pkg} (removed)");

    let labels: Vec<String> = findings.iter().map(format_finding).collect();

    let defaults: Vec<bool> = findings
        .iter()
        .map(|f| should_default_checked(f.confidence, auto_confirm_level))
        .collect();

    let total_bytes: u64 = findings
        .iter()
        .filter(|f| should_default_checked(f.confidence, auto_confirm_level))
        .map(|f| f.size_bytes)
        .sum();

    let default_count = defaults.iter().filter(|&&b| b).count();
    println!(
        "  Total pre-selected: {} across {} paths",
        format_bytes(total_bytes),
        default_count
    );

    let selection = MultiSelect::new()
        .with_prompt("Select items to remove (space to toggle, enter to confirm)")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;

    Ok(selection
        .into_iter()
        .map(|i| findings[i].path.clone())
        .collect())
}

fn format_finding(f: &Finding) -> String {
    let conf = match f.confidence {
        Confidence::High => "High  ",
        Confidence::Medium => "Medium",
        Confidence::Low => "Low   ",
    };
    let reasons = format_reasons(&f.reasons);

    if f.file_count > 0 {
        format!(
            "  {}/  ({}, {} files)  {}  [{}]",
            f.path.display(),
            format_bytes(f.size_bytes),
            f.file_count,
            conf,
            reasons
        )
    } else {
        format!(
            "  {}  ({})  {}  [{}]",
            f.path.display(),
            format_bytes(f.size_bytes),
            conf,
            reasons
        )
    }
}

fn format_reasons(reasons: &[Reason]) -> String {
    reasons
        .iter()
        .map(|r| match r {
            Reason::InXdgDatabase => "xdg_db",
            Reason::ExecutableGone(_) => "exe_gone",
            Reason::ExactNameMatch => "name_match",
            Reason::PkgnameSubstringMatch => "substr_match",
            Reason::PacmanOrphan => "orphan",
            Reason::NotAccessedFor(_) => "not_accessed",
            Reason::UserAllowlist => "allowlist",
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

fn group_by_package(findings: &[Finding]) -> HashMap<String, Vec<Finding>> {
    let mut map: HashMap<String, Vec<Finding>> = HashMap::new();
    for f in findings {
        map.entry(f.package.clone()).or_default().push(f.clone());
    }
    map
}

fn auto_select(findings: &[Finding], level: &AutoConfirmLevel) -> Vec<PathBuf> {
    findings
        .iter()
        .filter(|f| should_default_checked(f.confidence, level))
        .map(|f| f.path.clone())
        .collect()
}

fn should_default_checked(confidence: Confidence, level: &AutoConfirmLevel) -> bool {
    match level {
        AutoConfirmLevel::None => false,
        AutoConfirmLevel::High => confidence == Confidence::High,
        AutoConfirmLevel::Medium => {
            confidence == Confidence::High || confidence == Confidence::Medium
        }
        AutoConfirmLevel::Low => true,
    }
}

fn is_interactive() -> bool {
    // Check if stdout is a terminal.
    use std::os::unix::io::AsRawFd;
    unsafe { libc::isatty(std::io::stdout().as_raw_fd()) == 1 }
}

/// Replace groups of findings that all live under a fully-orphaned directory
/// with a single entry for that directory. Processes ancestors shallowest-first
/// so the highest valid collapse point wins.
///
/// O(n * depth) total: the ancestor traversal builds a reverse map
/// (dir → finding indices) in one pass, eliminating the O(n * m) inner scan.
fn collapse_to_dirs(findings: Vec<Finding>) -> Vec<Finding> {
    if findings.len() < 2 {
        return findings;
    }

    // Build orphan_files set and, simultaneously, a reverse map from each
    // ancestor directory to the indices of findings beneath it.
    let mut orphan_files: HashSet<PathBuf> = HashSet::with_capacity(findings.len());
    // dir → indices of findings that live anywhere under that dir
    let mut dir_to_indices: HashMap<PathBuf, Vec<usize>> = HashMap::new();

    for (idx, finding) in findings.iter().enumerate() {
        orphan_files.insert(finding.path.clone());
        let mut cur = finding.path.as_path();
        while let Some(parent) = cur.parent() {
            if parent.as_os_str().is_empty() {
                break;
            }
            let parent_str = parent.to_string_lossy();
            if COLLAPSE_NEVER.contains(&&*parent_str) {
                break;
            }
            if parent.components().count() < COLLAPSE_MIN_COMPONENTS {
                break;
            }
            // Push this finding's index into the parent's list.
            // If the parent was already in the map, its ancestors already have
            // this finding too (they were added on a previous iteration of the
            // outer loop), so we can break early.
            let entry = dir_to_indices.entry(parent.to_path_buf()).or_default();
            entry.push(idx);
            let already_had_parent = entry.len() > 1;
            cur = parent;
            if already_had_parent {
                // Parent and all its ancestors were already processed for a
                // previous finding; they'll get this index appended via the
                // continued upward walk on the *next* finding. But we still need
                // to walk up to append *this* index everywhere, so don't break.
                // (The break optimisation only applies when we know the parent
                // was already fully set up, which we can't guarantee here.)
                _ = already_had_parent; // acknowledged — keep walking
            }
        }
    }

    // Determine which ancestor directories are entirely orphaned.
    // Sort deepest-first so children are resolved before parents.
    let mut candidates: Vec<PathBuf> = dir_to_indices.keys().cloned().collect();
    candidates.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

    let orphan_dirs = collect_orphan_dirs(&candidates, &orphan_files);

    // Re-sort shallowest first so a high-level collapse subsumes all its children.
    let mut collapse_candidates: Vec<PathBuf> = orphan_dirs.into_iter().collect();
    collapse_candidates.sort_by_key(|p| p.components().count());

    let mut consumed: HashSet<PathBuf> = HashSet::new();
    let mut collapsed: Vec<Finding> = Vec::new();

    for dir in &collapse_candidates {
        // Use the pre-built index list — O(under.len()) not O(all_findings).
        let Some(indices) = dir_to_indices.get(dir) else { continue };
        let under: Vec<&Finding> = indices
            .iter()
            .filter_map(|&i| {
                let f = &findings[i];
                if consumed.contains(&f.path) { None } else { Some(f) }
            })
            .collect();

        if under.is_empty() {
            continue;
        }

        let total_size: u64 = under.iter().map(|f| f.size_bytes).sum();
        let min_confidence = under.iter().map(|f| f.confidence).min().expect("non-empty");
        let rep = under[0];

        collapsed.push(Finding {
            path: dir.clone(),
            size_bytes: total_size,
            package: rep.package.clone(),
            confidence: min_confidence,
            reasons: rep.reasons.clone(),
            category: rep.category,
            file_count: under.len() as u64,
        });

        for f in &under {
            consumed.insert(f.path.clone());
        }
    }

    // Append findings that weren't collapsed into any directory.
    for f in findings {
        if !consumed.contains(&f.path) {
            collapsed.push(f);
        }
    }

    collapsed
}

/// Returns the subset of `candidates` (pre-sorted deepest-first) that are
/// entirely orphaned, using one `read_dir` per candidate directory.
///
/// A directory is entirely orphaned when every immediate child is either
/// a file in `orphan_files` or a subdirectory already marked as an orphan dir.
fn collect_orphan_dirs(
    candidates_deepest_first: &[PathBuf],
    orphan_files: &HashSet<PathBuf>,
) -> HashSet<PathBuf> {
    let mut orphan_dirs: HashSet<PathBuf> = HashSet::new();

    'dir: for dir in candidates_deepest_first {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.filter_map(std::result::Result::ok) {
            let child = entry.path();
            let Ok(meta) = child.symlink_metadata() else { continue };
            if meta.is_dir() {
                if !orphan_dirs.contains(&child) {
                    continue 'dir; // subdirectory has non-orphaned contents
                }
            } else if !orphan_files.contains(&child) {
                continue 'dir; // non-orphaned file present
            }
        }
        orphan_dirs.insert(dir.clone());
    }

    orphan_dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanners::{Category, Confidence, Reason};
    use std::time::Duration;
    use tempfile::TempDir;

    fn make_finding(path: &str, size: u64) -> Finding {
        Finding {
            path: PathBuf::from(path),
            size_bytes: size,
            package: String::new(),
            confidence: Confidence::Low,
            reasons: vec![Reason::PacmanOrphan],
            category: Category::SystemOrphan,
            file_count: 0,
        }
    }

    #[test]
    fn collapses_fully_orphaned_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Build nested dirs so the collapse has candidates at multiple depths.
        let dir = root.join("a/b/c/d");
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("file1.txt");
        let f2 = dir.join("file2.txt");
        std::fs::write(&f1, "x").unwrap();
        std::fs::write(&f2, "x").unwrap();

        let findings = vec![
            make_finding(f1.to_str().unwrap(), 1),
            make_finding(f2.to_str().unwrap(), 1),
        ];

        let result = collapse_to_dirs(findings);

        // The algorithm finds the highest fully-orphaned ancestor with enough
        // depth. Either way: result must be a single entry that is an ancestor
        // of both original files, and the size must be the sum.
        assert_eq!(result.len(), 1, "expected exactly one collapsed entry");
        let collapsed = &result[0];
        assert!(
            f1.starts_with(&collapsed.path) && f2.starts_with(&collapsed.path),
            "collapsed path {} must be ancestor of both files",
            collapsed.path.display()
        );
        assert_eq!(collapsed.size_bytes, 2);
    }

    #[test]
    fn does_not_collapse_dir_with_non_orphan_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let dir = root.join("a/b/c/d");
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("orphan.txt");
        let f2 = dir.join("owned.txt"); // NOT in findings
        std::fs::write(&f1, "x").unwrap();
        std::fs::write(&f2, "x").unwrap();

        let findings = vec![make_finding(f1.to_str().unwrap(), 1)];

        let result = collapse_to_dirs(findings);
        // Should NOT collapse because f2 is not in the orphan set.
        assert!(result.iter().all(|f| f.path != dir));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn single_finding_not_collapsed() {
        let findings = vec![make_finding("/usr/lib/node_modules/foo/index.js", 100)];
        let result = collapse_to_dirs(findings.clone());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, findings[0].path);
    }

    // Silence "unused import" for Duration which is part of the Reason enum shape
    // needed in the wildcard import but not directly used in these tests.
    #[allow(dead_code)]
    fn _use_duration(_: Duration) {}
}
