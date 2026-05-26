use anyhow::Context;
use std::process::Command;

/// Returns all files owned by `pkgname` via `pacman -Qlq`.
pub fn files_for_package(pkgname: &str) -> anyhow::Result<Vec<std::path::PathBuf>> {
    assert!(!pkgname.is_empty(), "pkgname must not be empty");
    assert!(
        !pkgname.contains(';') && !pkgname.contains('&') && !pkgname.contains('|'),
        "pkgname must not contain shell metacharacters"
    );

    let output = Command::new("pacman")
        .args(["-Qlq", pkgname])
        .output()
        .context("running pacman -Qlq")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(std::path::PathBuf::from)
        .collect())
}

/// Returns orphan package names via `pacman -Qdtq`.
pub fn orphan_packages() -> anyhow::Result<Vec<String>> {
    let output = Command::new("pacman")
        .args(["-Qdtq"])
        .output()
        .context("running pacman -Qdtq")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Returns basic info for a package via `pacman -Qi`.
pub fn package_info(pkgname: &str) -> anyhow::Result<String> {
    assert!(!pkgname.is_empty(), "pkgname must not be empty");
    assert!(
        !pkgname.contains(';') && !pkgname.contains('&') && !pkgname.contains('|'),
        "pkgname must not contain shell metacharacters"
    );

    let output = Command::new("pacman")
        .args(["-Qi", pkgname])
        .output()
        .context("running pacman -Qi")?;

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
