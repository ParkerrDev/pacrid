use crate::pacman::db::PacmanDb;
use std::path::Path;

/// Returns true if `path` is owned by any installed package.
/// Safety invariant: never delete pacman-owned paths.
pub fn is_owned(path: &Path, db: &PacmanDb) -> bool {
    assert!(!path.as_os_str().is_empty(), "path must not be empty");
    db.owns(path)
}
