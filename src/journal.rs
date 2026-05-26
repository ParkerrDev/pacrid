use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const JOURNAL_ROOT_PRIVILEGED: &str = "/var/lib/pacrid/journal";
const JOURNAL_ROOT_USER: &str = ".local/state/pacrid/journal";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalAction {
    pub original: PathBuf,
    pub moved_to: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub trigger: String,
    pub packages: Vec<String>,
    pub actions: Vec<JournalAction>,
}

impl JournalEntry {
    pub fn new(trigger: &str, packages: Vec<String>) -> Self {
        let timestamp = Utc::now();
        let id = timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();
        Self {
            id,
            timestamp,
            trigger: trigger.to_owned(),
            packages,
            actions: Vec::new(),
        }
    }

    pub fn add_action(&mut self, original: PathBuf, moved_to: String, size: u64) {
        assert!(
            !original.as_os_str().is_empty(),
            "original path must not be empty"
        );
        self.actions.push(JournalAction {
            original,
            moved_to,
            size,
        });
    }
}

fn journal_dir() -> PathBuf {
    // Use privileged dir if we're root, else user dir.
    if nix_is_root() {
        PathBuf::from(JOURNAL_ROOT_PRIVILEGED)
    } else {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(JOURNAL_ROOT_USER)
    }
}

fn nix_is_root() -> bool {
    // SAFETY: libc::getuid() is a POSIX call with no preconditions, no
    // side effects, and a defined-for-all-processes return value. It cannot
    // fail or invalidate any Rust aliasing/lifetime invariants.
    unsafe { libc::getuid() == 0 }
}

pub fn write_entry(entry: &JournalEntry) -> anyhow::Result<PathBuf> {
    assert!(
        !entry.actions.is_empty(),
        "refusing to write empty journal entry"
    );

    let dir = journal_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating journal dir {}", dir.display()))?;

    let filename = format!("{}.json", entry.id);
    let path = dir.join(&filename);

    let json = serde_json::to_string_pretty(entry).context("serializing journal entry")?;
    std::fs::write(&path, json)
        .with_context(|| format!("writing journal entry to {}", path.display()))?;

    tracing::info!("wrote journal entry: {}", path.display());
    Ok(path)
}

pub fn read_entry(id: &str) -> anyhow::Result<JournalEntry> {
    assert!(!id.is_empty(), "journal id must not be empty");

    let path = journal_dir().join(format!("{id}.json"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading journal entry {}", path.display()))?;
    serde_json::from_str(&content).context("parsing journal entry")
}

pub fn list_entries() -> anyhow::Result<Vec<JournalEntry>> {
    let dir = journal_dir();
    // Invariant: journal_dir() always returns an absolute path. If this ever
    // failed it would indicate a regression in journal_dir or its inputs.
    assert!(
        dir.is_absolute(),
        "journal_dir() must return an absolute path, got {}",
        dir.display()
    );
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&dir)
        .with_context(|| format!("reading journal dir {}", dir.display()))?;

    for item in read_dir {
        let item = item.context("reading journal dir entry")?;
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        match serde_json::from_str::<JournalEntry>(&content) {
            Ok(e) => entries.push(e),
            Err(err) => {
                tracing::warn!("skipping malformed journal entry {}: {err}", path.display());
            }
        }
    }

    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    // Post-condition: sort_by_key on Reverse(timestamp) guarantees newest first.
    // windows(2) always yields slices of length 2, so first/get are total, but
    // we phrase the predicate without [] indexing to satisfy clippy::indexing_slicing.
    assert!(
        entries.windows(2).all(|w| {
            let (Some(a), Some(b)) = (w.first(), w.get(1)) else {
                return true;
            };
            a.timestamp >= b.timestamp
        }),
        "list_entries postcondition: entries must be sorted newest-first"
    );
    Ok(entries)
}

pub fn latest_entry_id() -> anyhow::Result<Option<String>> {
    let entries = list_entries()?;
    Ok(entries.into_iter().next().map(|e| e.id))
}

pub fn quarantine_dir() -> PathBuf {
    PathBuf::from("/var/lib/pacrid/quarantine")
}

/// Sanitize a path for use as a filesystem component in the quarantine dir.
pub fn sanitize_path_for_quarantine(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    // Strip leading slash so we can join under quarantine/<timestamp>/.
    PathBuf::from(s.trim_start_matches('/'))
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
    // JournalEntry::new calls chrono::Utc::now -> clock_gettime, which Miri
    // doesn't implement on Linux. The serde round-trip itself is pure but
    // can't run without a valid entry.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn journal_entry_round_trip() {
        let mut entry = JournalEntry::new("test", vec!["steam".to_owned()]);
        entry.add_action(PathBuf::from("/home/u/.steam"), "trash://".to_owned(), 1234);

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: JournalEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.trigger, "test");
        assert_eq!(parsed.packages, vec!["steam".to_owned()]);
        assert_eq!(parsed.actions.len(), 1);
        assert_eq!(parsed.actions[0].size, 1234);
    }
}
