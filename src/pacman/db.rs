use anyhow::Context;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const PACMAN_LOCAL: &str = "/var/lib/pacman/local";

/// All paths known to be owned by installed pacman packages.
#[derive(Debug, Default, Clone)]
pub struct PacmanDb {
    owned: HashSet<PathBuf>,
}

impl PacmanDb {
    /// Load the database from the default pacman local db directory.
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(Path::new(PACMAN_LOCAL))
    }

    /// Load from an alternative directory (for testing).
    pub fn load_from(local_dir: &Path) -> anyhow::Result<Self> {
        assert!(local_dir.is_dir(), "pacman local db dir must exist: {}", local_dir.display());

        let mut owned: HashSet<PathBuf> = HashSet::new();
        let mut pkg_count: usize = 0;

        let entries = std::fs::read_dir(local_dir)
            .with_context(|| format!("reading pacman db dir {}", local_dir.display()))?;

        for entry in entries {
            let entry = entry.context("reading pacman db entry")?;
            let pkg_dir = entry.path();
            if !pkg_dir.is_dir() {
                continue;
            }
            let files_path = pkg_dir.join("files");
            if !files_path.exists() {
                continue;
            }
            parse_files_into(&files_path, &mut owned)?;
            pkg_count += 1;
        }

        assert!(
            pkg_count > 0 || !local_dir.as_os_str().eq(PACMAN_LOCAL),
            "expected at least one package in real pacman db"
        );

        tracing::debug!("loaded pacman db: {} packages, {} paths", pkg_count, owned.len());
        Ok(Self { owned })
    }

    pub fn owns(&self, path: &Path) -> bool {
        self.owned.contains(path)
    }

    pub fn owned_paths(&self) -> &HashSet<PathBuf> {
        &self.owned
    }

    pub fn package_count(&self) -> usize {
        self.owned.len()
    }
}

/// Parse a pacman `files` file, inserting all owned paths into `out`.
/// The format is:
///   %FILES%
///   usr/bin/foo
///   usr/lib/foo.so
///   (paths without leading slash — we add it)
fn parse_files_into(path: &Path, out: &mut HashSet<PathBuf>) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    let mut in_files = false;
    for line in content.lines() {
        let line = line.trim();
        if line == "%FILES%" {
            in_files = true;
            continue;
        }
        if line.starts_with('%') {
            in_files = false;
            continue;
        }
        if in_files && !line.is_empty() {
            // Prepend '/' — pacman db stores paths without leading slash.
            let full = PathBuf::from("/").join(line);
            out.insert(full);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_fixture(dir: &Path, pkg: &str, files: &[&str]) {
        let pkg_dir = dir.join(pkg);
        fs::create_dir_all(&pkg_dir).unwrap();
        let mut content = "%FILES%\n".to_owned();
        for f in files {
            content.push_str(f);
            content.push('\n');
        }
        fs::write(pkg_dir.join("files"), content).unwrap();
    }

    #[test]
    fn parses_fixture_correctly() {
        let tmp = TempDir::new().unwrap();
        make_fixture(
            tmp.path(),
            "steam-1.0",
            &["usr/bin/steam", "usr/lib/steam/steam.so"],
        );

        let db = PacmanDb::load_from(tmp.path()).unwrap();
        assert!(db.owns(Path::new("/usr/bin/steam")));
        assert!(db.owns(Path::new("/usr/lib/steam/steam.so")));
        assert!(!db.owns(Path::new("/usr/bin/nonexistent")));
    }

    #[test]
    fn ignores_non_files_sections() {
        let tmp = TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("pkg-1.0");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let content = "%DESC%\nA package\n\n%FILES%\nusr/bin/tool\n\n%DEPENDS%\nglibc\n";
        std::fs::write(pkg_dir.join("files"), content).unwrap();

        let db = PacmanDb::load_from(tmp.path()).unwrap();
        assert!(db.owns(Path::new("/usr/bin/tool")));
        assert!(!db.owns(Path::new("/glibc")));
    }
}
