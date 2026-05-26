use anyhow::Context;

/// List orphan packages via `pacman -Qdtq`.
pub fn list_orphans() -> anyhow::Result<Vec<String>> {
    crate::pacman::query::orphan_packages()
}

/// Remove orphan packages via `sudo pacman -Rns`.
/// Safety invariant: sets `PACRID_IN_HOOK=1` to prevent recursive hook triggering.
pub fn remove_orphans(packages: &[String], dry_run: bool) -> anyhow::Result<()> {
    assert!(!packages.is_empty(), "packages list must not be empty");
    assert!(
        packages.iter().all(|p| !p.contains(';') && !p.contains('&') && !p.contains('|')),
        "package names must not contain shell metacharacters"
    );

    if dry_run {
        tracing::info!("[dry-run] would remove orphans: {:?}", packages);
        return Ok(());
    }

    let status = std::process::Command::new("sudo")
        .env("PACRID_IN_HOOK", "1")
        .arg("pacman")
        .arg("-Rns")
        .args(packages)
        .status()
        .context("running sudo pacman -Rns")?;

    if !status.success() {
        anyhow::bail!("pacman -Rns failed with status: {status}");
    }

    Ok(())
}
