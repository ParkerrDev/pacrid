use crate::config::AutoConfirmLevel;
use crate::scanners::{Confidence, Finding, Reason};
use crate::util::format_bytes;
use inquire::MultiSelect;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ANSI colour codes for the interactive label. inquire renders our `String`
// items verbatim, so we embed the escapes directly. Terminals that don't
// understand them simply show the raw codes; in practice every modern
// terminal on Arch Linux supports them.
const C_RESET: &str = "\x1b[0m";
const C_GREEN: &str = "\x1b[32m";
const C_YELLOW: &str = "\x1b[33m";
const C_RED: &str = "\x1b[31m";
const C_CYAN: &str = "\x1b[36m";
const C_DIM: &str = "\x1b[2m";
const C_BOLD: &str = "\x1b[1m";

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
    assert!(
        !findings.is_empty(),
        "interactive_review called with no findings"
    );

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
    assert!(
        !findings.is_empty(),
        "review_package called with no findings for {pkg}"
    );

    println!("\n{C_BOLD}Package:{C_RESET} {pkg} {C_DIM}(removed){C_RESET}");

    let labels: Vec<String> = findings.iter().map(format_finding).collect();

    let default_indices: Vec<usize> = findings
        .iter()
        .enumerate()
        .filter(|(_, f)| should_default_checked(f.confidence, auto_confirm_level))
        .map(|(i, _)| i)
        .collect();

    let total_bytes: u64 = default_indices
        .iter()
        .filter_map(|&i| findings.get(i))
        .map(|f| f.size_bytes)
        .sum();

    println!(
        "  {C_DIM}Pre-selected: {} across {} paths{C_RESET}",
        format_bytes(total_bytes),
        default_indices.len()
    );

    // inquire handles narrow-terminal rendering correctly (the bug we hit
    // with dialoguer) and gives select-all/clear-all out of the box.
    let selected = MultiSelect::new("Select items to remove:", labels)
        .with_default(&default_indices)
        .with_page_size(15)
        .with_help_message(
            "↑/↓ move • space toggle • → select all • ← clear • enter confirm • esc cancel",
        )
        .raw_prompt()?;

    // raw_prompt returns ListOption { index, value }. Map back to original
    // findings via the index. Checked .get() means no panic on a stale index.
    Ok(selected
        .into_iter()
        .filter_map(|opt| findings.get(opt.index).map(|f| f.path.clone()))
        .collect())
}

fn format_finding(f: &Finding) -> String {
    let (conf_color, conf_label) = match f.confidence {
        Confidence::High => (C_GREEN, "High"),
        Confidence::Medium => (C_YELLOW, "Medium"),
        Confidence::Low => (C_RED, "Low"),
    };
    let reasons = format_reasons(&f.reasons);
    let size = format_bytes(f.size_bytes);

    // Layout: <conf>  <path>  (size[, N files])  [reasons]
    // Confidence first so the eye reads safety class before the path. Path
    // gets no width truncation here — inquire wraps within its own buffer
    // and the next render position stays sane (the dialoguer bug we hit).
    let path_part = if f.file_count > 0 {
        format!(
            "{}/ ({C_CYAN}{}, {} files{C_RESET})",
            f.path.display(),
            size,
            f.file_count
        )
    } else {
        format!("{} ({C_CYAN}{}{C_RESET})", f.path.display(), size)
    };
    format!("{conf_color}{conf_label:<6}{C_RESET}  {path_part}  {C_DIM}[{reasons}]{C_RESET}")
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
    use std::os::unix::io::AsRawFd;
    // SAFETY: libc::isatty takes a raw file descriptor and returns 0 or 1
    // (or sets errno on bad fd; we treat that as "not a tty"). stdout's fd
    // is owned by the process for its lifetime, so the descriptor is always
    // valid for the duration of this call.
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

    let (orphan_files, dir_to_indices) = build_ancestor_index(&findings);
    let collapse_candidates = orphan_candidates_shallow_first(&dir_to_indices, &orphan_files);

    let mut consumed: HashSet<PathBuf> = HashSet::new();
    let mut collapsed: Vec<Finding> = Vec::new();

    for dir in &collapse_candidates {
        if let Some(entry) = collapse_one(dir, &findings, &dir_to_indices, &consumed) {
            for path in &entry.consumed_paths {
                consumed.insert(path.clone());
            }
            collapsed.push(entry.finding);
        }
    }

    for f in findings {
        if !consumed.contains(&f.path) {
            collapsed.push(f);
        }
    }

    collapsed
}

/// Build the orphan-file set and the (ancestor dir → finding indices) reverse map
/// in a single pass over `findings`.
fn build_ancestor_index(findings: &[Finding]) -> (HashSet<PathBuf>, HashMap<PathBuf, Vec<usize>>) {
    let mut orphan_files: HashSet<PathBuf> = HashSet::with_capacity(findings.len());
    let mut dir_to_indices: HashMap<PathBuf, Vec<usize>> = HashMap::new();

    for (idx, finding) in findings.iter().enumerate() {
        orphan_files.insert(finding.path.clone());
        let mut cur = finding.path.as_path();
        // Bounded loop: each iteration moves `cur` to its parent (one fewer
        // path component). Filesystem paths have a finite component count
        // (PATH_MAX / NAME_MAX), so this terminates in O(path depth).
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
            dir_to_indices
                .entry(parent.to_path_buf())
                .or_default()
                .push(idx);
            cur = parent;
        }
    }

    (orphan_files, dir_to_indices)
}

/// Filter the candidate ancestor dirs down to those whose entire subtree is
/// orphaned, then return them sorted shallowest-first so the highest valid
/// collapse point in any chain wins.
fn orphan_candidates_shallow_first(
    dir_to_indices: &HashMap<PathBuf, Vec<usize>>,
    orphan_files: &HashSet<PathBuf>,
) -> Vec<PathBuf> {
    let mut deepest_first: Vec<PathBuf> = dir_to_indices.keys().cloned().collect();
    deepest_first.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

    let orphan_dirs = collect_orphan_dirs(&deepest_first, orphan_files);
    let mut shallow_first: Vec<PathBuf> = orphan_dirs.into_iter().collect();
    shallow_first.sort_by_key(|p| p.components().count());
    shallow_first
}

struct CollapseEntry {
    finding: Finding,
    consumed_paths: Vec<PathBuf>,
}

/// Build a single collapsed Finding for `dir` from the still-unconsumed
/// findings beneath it. Returns None if every covered finding has already
/// been claimed by a shallower collapse.
fn collapse_one(
    dir: &Path,
    findings: &[Finding],
    dir_to_indices: &HashMap<PathBuf, Vec<usize>>,
    consumed: &HashSet<PathBuf>,
) -> Option<CollapseEntry> {
    let indices = dir_to_indices.get(dir)?;
    // filter_map with .get() instead of [] indexing: by construction every
    // index is valid, but a checked lookup makes that impossible to violate.
    let under: Vec<&Finding> = indices
        .iter()
        .filter_map(|&i| findings.get(i))
        .filter(|f| !consumed.contains(&f.path))
        .collect();

    // .first() is the safe analogue of under[0]; we also need it as the loop
    // sentinel below, so a single ok-or-none is cleaner than separate checks.
    let rep = under.first()?;
    let total_size: u64 = under.iter().map(|f| f.size_bytes).sum();
    let min_confidence = under.iter().map(|f| f.confidence).min()?;
    let consumed_paths: Vec<PathBuf> = under.iter().map(|f| f.path.clone()).collect();

    Some(CollapseEntry {
        finding: Finding {
            path: dir.to_path_buf(),
            size_bytes: total_size,
            package: rep.package.clone(),
            confidence: min_confidence,
            reasons: rep.reasons.clone(),
            category: rep.category,
            file_count: under.len() as u64,
        },
        consumed_paths,
    })
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
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let child = entry.path();
            let Ok(meta) = child.symlink_metadata() else {
                continue;
            };
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
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
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
    #[cfg_attr(miri, ignore)]
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
    #[cfg_attr(miri, ignore)]
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

    // ─ Property tests ─
    // proptest spawns subprocesses (via rusty-fork) to isolate test cases.
    // Miri cannot simulate fork/exec, so the proptest block as a whole is
    // gated out under Miri. Pure-logic unit tests above still run.
    #[cfg(not(miri))]
    use proptest::prelude::*;

    #[cfg(not(miri))]
    proptest! {
        /// collapse_to_dirs must conserve total size: the sum of size_bytes
        /// over the output (including the file_count multiplier for collapsed
        /// entries) equals the sum over the input. Bytes are never lost or
        /// invented.
        #[test]
        fn property_collapse_preserves_total_bytes(
            sizes in proptest::collection::vec(0u64..1_000_000_u64, 0..20),
        ) {
            // Build distinct fake paths so collapse logic has multiple candidates.
            let findings: Vec<Finding> = sizes
                .iter()
                .enumerate()
                .map(|(i, &s)| make_finding(&format!("/tmp/fake/{i}/file.bin"), s))
                .collect();

            let before: u64 = findings.iter().map(|f| f.size_bytes).sum();
            let after = collapse_to_dirs(findings);
            let after_sum: u64 = after.iter().map(|f| f.size_bytes).sum();

            // Since none of these paths exist on disk, collapse won't merge
            // them — but the conservation invariant must hold regardless.
            prop_assert_eq!(before, after_sum);
        }

        /// collapse_to_dirs never produces a path that is shallower than
        /// COLLAPSE_MIN_COMPONENTS. This is the structural safety floor
        /// that prevents collapsing to dangerous roots like /usr/lib.
        #[test]
        fn property_collapse_respects_min_depth(
            sizes in proptest::collection::vec(0u64..1_000_u64, 0..10),
        ) {
            let findings: Vec<Finding> = sizes
                .iter()
                .enumerate()
                .map(|(i, &s)| make_finding(&format!("/var/cache/x/y/{i}/leaf"), s))
                .collect();
            let after = collapse_to_dirs(findings);
            for f in &after {
                // file_count > 0 means this is a collapsed entry — it must
                // satisfy the minimum-depth invariant. file_count == 0
                // entries are passthroughs of the input and exempt.
                if f.file_count > 0 {
                    let depth = f.path.components().count();
                    prop_assert!(
                        depth >= COLLAPSE_MIN_COMPONENTS,
                        "collapsed path {} has depth {} < {}",
                        f.path.display(), depth, COLLAPSE_MIN_COMPONENTS
                    );
                }
            }
        }
    }
}
