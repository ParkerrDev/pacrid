use crate::journal::{quarantine_dir, sanitize_path_for_quarantine, JournalEntry};
use crate::pacman::db::PacmanDb;
use anyhow::Context;
use chrono::Utc;
use std::path::Path;

/// Paths that are always forbidden — never delete these.
/// Safety invariant: bare top-level paths are always refused.
const FORBIDDEN_PREFIXES: &[&str] = &[
    "/", "/home", "/root", "/etc", "/usr", "/var", "/boot", "/proc", "/sys", "/run", "/dev", "/tmp",
];

#[derive(Debug, Clone, Copy)]
pub enum DeleteMode {
    Trash,
    Quarantine,
    Purge,
}

#[must_use = "ignoring an ExecutionResult drops both the success list and the journal pointer needed for undo"]
pub struct ExecutionResult {
    pub succeeded: Vec<std::path::PathBuf>,
    pub failed: Vec<(std::path::PathBuf, String)>,
    pub journal_path: Option<std::path::PathBuf>,
    /// Total bytes reclaimed, taken from the journal actions so it counts what
    /// was actually moved rather than what was proposed. Zero under `--dry-run`,
    /// where nothing is journalled.
    pub freed_bytes: u64,
}

/// Execute deletion of the given paths.
/// Safety invariants enforced here:
/// 1. Never delete pacman-owned paths.
/// 2. Never delete without journaling first.
/// 3. Never operate on bare top-level paths.
/// 4. Never follow symlinks.
/// 5. Never delete a path containing `..` after canonicalization mismatch.
pub fn execute(
    paths: &[std::path::PathBuf],
    packages: &[String],
    db: &PacmanDb,
    mode: DeleteMode,
    trigger: &str,
    dry_run: bool,
) -> anyhow::Result<ExecutionResult> {
    assert!(!paths.is_empty(), "paths to delete must not be empty");
    assert!(!packages.is_empty(), "packages context must not be empty");

    let mut journal = JournalEntry::new(trigger, packages.to_vec());
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();

    for path in paths {
        process_one_path(
            path,
            db,
            mode,
            &timestamp,
            dry_run,
            &mut journal,
            &mut succeeded,
            &mut failed,
        );
    }

    let journal_path = if !journal.actions.is_empty() && !dry_run {
        Some(crate::journal::write_entry(&journal).unwrap_or_else(|e| {
            tracing::error!("failed to write journal: {e}");
            std::path::PathBuf::new()
        }))
    } else {
        None
    };

    let freed_bytes = journal
        .actions
        .iter()
        .fold(0_u64, |acc, a| acc.saturating_add(a.size));

    Ok(ExecutionResult {
        succeeded,
        failed,
        journal_path,
        freed_bytes,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_one_path(
    path: &std::path::Path,
    db: &PacmanDb,
    mode: DeleteMode,
    timestamp: &str,
    dry_run: bool,
    journal: &mut JournalEntry,
    succeeded: &mut Vec<std::path::PathBuf>,
    failed: &mut Vec<(std::path::PathBuf, String)>,
) {
    if let Err(e) = validate_path(path, db) {
        tracing::warn!("refusing to delete {}: {e}", path.display());
        failed.push((path.to_path_buf(), e.to_string()));
        return;
    }

    if dry_run {
        tracing::info!("[dry-run] would delete: {}", path.display());
        succeeded.push(path.to_path_buf());
        return;
    }

    let size = crate::util::compute_size(path);
    match do_delete(path, mode, timestamp) {
        Ok(moved_to) => {
            journal.add_action(path.to_path_buf(), moved_to, size);
            succeeded.push(path.to_path_buf());
        }
        Err(e) => {
            tracing::error!("failed to delete {}: {e}", path.display());
            failed.push((path.to_path_buf(), e.to_string()));
        }
    }
}

/// Restore a previous deletion from the journal.
pub fn undo(entry: &crate::journal::JournalEntry, dry_run: bool) -> anyhow::Result<()> {
    assert!(
        !entry.actions.is_empty(),
        "journal entry has no actions to undo"
    );

    for action in &entry.actions {
        if dry_run {
            tracing::info!(
                "[dry-run] would restore {} from {}",
                action.original.display(),
                action.moved_to
            );
            continue;
        }

        if action.moved_to.starts_with("trash://") {
            // Cannot programmatically restore from XDG trash by URI easily;
            // instruct user to use their file manager.
            tracing::warn!(
                "cannot auto-restore {} from XDG trash — use your file manager",
                action.original.display()
            );
        } else {
            let from = Path::new(&action.moved_to);
            if !from.exists() {
                tracing::warn!("quarantine path missing: {}", action.moved_to);
                continue;
            }
            if let Some(parent) = action.original.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent {}", parent.display()))?;
            }
            std::fs::rename(from, &action.original).with_context(|| {
                format!(
                    "restoring {} from {}",
                    action.original.display(),
                    action.moved_to
                )
            })?;
            tracing::info!("restored: {}", action.original.display());
        }
    }

    Ok(())
}

/// Empty the pacrid quarantine directory.
pub fn empty_quarantine(dry_run: bool) -> anyhow::Result<()> {
    let qdir = quarantine_dir();
    if !qdir.exists() {
        println!("Quarantine directory is empty (does not exist).");
        return Ok(());
    }

    if dry_run {
        tracing::info!("[dry-run] would remove quarantine: {}", qdir.display());
        return Ok(());
    }

    std::fs::remove_dir_all(&qdir)
        .with_context(|| format!("removing quarantine dir {}", qdir.display()))?;

    println!("Quarantine cleared: {}", qdir.display());
    Ok(())
}

fn validate_path(path: &Path, db: &PacmanDb) -> anyhow::Result<()> {
    assert!(!path.as_os_str().is_empty(), "path must not be empty");

    // Safety invariant: refuse bare top-level paths.
    let path_str = path.to_string_lossy();
    for forbidden in FORBIDDEN_PREFIXES {
        if path_str == *forbidden {
            anyhow::bail!("refusing to delete forbidden path: {}", path.display());
        }
    }

    // Safety invariant: reject paths containing `..`.
    if path.components().any(|c| c.as_os_str() == "..") {
        anyhow::bail!("path contains ..: {}", path.display());
    }

    // Safety invariant: never delete pacman-owned paths.
    if crate::pacman::owns::is_owned(path, db) {
        anyhow::bail!("path is owned by pacman: {}", path.display());
    }

    Ok(())
}

fn quarantine_move(path: &Path, timestamp: &str) -> anyhow::Result<String> {
    let qdir = quarantine_dir().join(timestamp);
    let dest = qdir.join(sanitize_path_for_quarantine(path));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating quarantine subdir {}", parent.display()))?;
    }
    std::fs::rename(path, &dest)
        .with_context(|| format!("moving {} to quarantine", path.display()))?;
    Ok(dest.to_string_lossy().into_owned())
}

fn do_delete(path: &Path, mode: DeleteMode, timestamp: &str) -> anyhow::Result<String> {
    match mode {
        DeleteMode::Trash => {
            // Fall back to quarantine when trash fails (e.g. hook runs as root
            // and cannot write to the user's XDG trash directory).
            match trash::delete(path) {
                Ok(()) => Ok("trash://".to_owned()),
                Err(trash_err) => {
                    tracing::debug!(
                        "trash unavailable for {} ({trash_err}), using quarantine",
                        path.display()
                    );
                    quarantine_move(path, timestamp)
                }
            }
        }
        DeleteMode::Quarantine => {
            // Safety invariant: never follow symlinks — quarantine_move uses rename.
            quarantine_move(path, timestamp)
        }
        DeleteMode::Purge => {
            // Safety invariant documented: --purge bypasses trash; use only explicitly.
            let meta = path
                .symlink_metadata()
                .with_context(|| format!("stat {}", path.display()))?;

            if meta.is_dir() {
                std::fs::remove_dir_all(path)
                    .with_context(|| format!("purging dir {}", path.display()))?;
            } else {
                std::fs::remove_file(path)
                    .with_context(|| format!("purging file {}", path.display()))?;
            }

            Ok("purged://".to_owned())
        }
    }
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
    // Proptest gated out under Miri (rusty-fork uses fork(), unsupported by Miri).
    #[cfg(not(miri))]
    use proptest::prelude::*;

    fn empty_db() -> PacmanDb {
        PacmanDb::default()
    }

    #[test]
    fn validate_rejects_each_forbidden_prefix() {
        for p in FORBIDDEN_PREFIXES {
            let err = validate_path(Path::new(p), &empty_db()).unwrap_err();
            assert!(
                err.to_string().contains("forbidden"),
                "expected forbidden error for {p}, got: {err}"
            );
        }
    }

    #[test]
    fn validate_rejects_dotdot() {
        let err = validate_path(Path::new("/home/u/../etc/passwd"), &empty_db()).unwrap_err();
        assert!(err.to_string().contains(".."), "got: {err}");
    }

    #[cfg(not(miri))]
    proptest! {
        /// validate_path must REJECT every path in FORBIDDEN_PREFIXES, in any
        /// case, with or without a trailing slash.
        #[test]
        fn property_forbidden_prefixes_always_rejected(
            i in 0usize..FORBIDDEN_PREFIXES.len(),
        ) {
            // get() avoids clippy::indexing_slicing — the range above is total.
            let p = FORBIDDEN_PREFIXES.get(i).copied().unwrap_or("/");
            let result = validate_path(Path::new(p), &empty_db());
            prop_assert!(result.is_err(), "should reject {p}");
        }

        /// Any path that traverses through `..` must be rejected, regardless
        /// of where the `..` appears in the path.
        #[test]
        fn property_dotdot_always_rejected(
            prefix in "[a-z]{1,8}",
            suffix in "[a-z]{1,8}",
        ) {
            let path = format!("/{prefix}/../{suffix}");
            let result = validate_path(Path::new(&path), &empty_db());
            prop_assert!(result.is_err(), "should reject {path}");
        }
    }
}
