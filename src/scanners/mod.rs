use std::path::PathBuf;
use std::time::Duration;

pub mod name_heuristic;
pub mod orphan_deps;
pub mod pacman_orphan;
pub mod xdg_db;

#[derive(Debug, Clone)]
pub struct Finding {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub package: String,
    pub confidence: Confidence,
    pub reasons: Vec<Reason>,
    pub category: Category,
    /// Number of individual files this finding represents.
    /// 0 = a single file/dir (not collapsed). >0 = a collapsed directory entry.
    pub file_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    InXdgDatabase,
    ExecutableGone(String),
    ExactNameMatch,
    PkgnameSubstringMatch,
    PacmanOrphan,
    NotAccessedFor(Duration),
    UserAllowlist,
}

#[derive(Debug, Clone, Copy)]
pub enum Category {
    Config,
    Cache,
    Data,
    State,
    SystemOrphan,
    OrphanDep,
}

pub struct ScanContext {
    pub removed_packages: Vec<String>,
    pub pacman_db: crate::pacman::db::PacmanDb,
    pub config: crate::config::Config,
    /// The real user's home directory. Do NOT use `std::env::var`(`"HOME"`) in scanners —
    /// the hook runs as root and `$HOME` is `/root`, not the user's actual home.
    pub home_dir: std::path::PathBuf,
}

pub trait Scanner {
    fn name(&self) -> &'static str;
    fn scan(&self, ctx: &ScanContext) -> anyhow::Result<Vec<Finding>>;
}
