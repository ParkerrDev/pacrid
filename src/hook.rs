use crate::config;
use crate::executor::{execute, DeleteMode};
use crate::pacman::db::PacmanDb;
use crate::review::interactive_review;
use crate::scanners::{ScanContext, Scanner};
use crate::user_homes::user_homes_to_scan;
use std::io::{self, BufRead};

/// Entry point for `pacrid hook` invoked by the pacman `PostTransaction` hook.
///
/// Safety invariants:
/// - Returns `Ok(())` always (never exit nonzero — pacman hook must not fail).
/// - Sets `PACRID_IN_HOOK=1` for child invocations to prevent recursion.
/// - Bails early if already inside a hook invocation.
pub fn run_hook(dry_run: bool, non_interactive: bool) -> anyhow::Result<()> {
    // Recursion guard: pacman hook fires for every transaction, so a child
    // process spawned by pacrid (e.g. via a scanner) that itself triggers
    // pacman would loop forever. The env var prevents that.
    if std::env::var("PACRID_IN_HOOK").is_ok() {
        tracing::debug!("PACRID_IN_HOOK set — bailing to prevent recursion");
        return Ok(());
    }
    // SAFETY: set_var is unsafe in Rust 1.81+ because concurrent reads from
    // other threads could observe a torn write. We call it here at the very
    // start of the hook, before any threads are spawned and before any
    // library code reads the environment. No aliasing or lifetime invariant
    // is at risk.
    unsafe { std::env::set_var("PACRID_IN_HOOK", "1") };

    let cfg = config::load();

    if !cfg.hook_enabled {
        tracing::debug!("hook disabled by config");
        return Ok(());
    }

    let packages = read_packages_from_stdin()?;
    if packages.is_empty() {
        tracing::debug!("no packages from stdin, exiting");
        return Ok(());
    }

    tracing::info!("hook triggered for packages: {:?}", packages);

    // Defensive isolation, not control flow: the pacman PostTransaction hook
    // must never abort the parent pacman process. If anything inside the
    // scanner pipeline panics we log it and exit cleanly. Errors via Result
    // are the normal path; this only catches unexpected panics.
    let result =
        std::panic::catch_unwind(|| run_hook_inner(&packages, &cfg, dry_run, non_interactive));

    match result {
        Ok(inner_result) => {
            if let Err(e) = inner_result {
                tracing::error!("hook error (non-fatal): {e:#}");
            }
        }
        Err(_panic) => {
            // Safety invariant: hook must never panic-propagate.
            tracing::error!("hook panicked — suppressing to not break pacman");
        }
    }

    Ok(())
}

fn run_hook_inner(
    packages: &[String],
    cfg: &crate::config::Config,
    dry_run: bool,
    non_interactive: bool,
) -> anyhow::Result<()> {
    let db = PacmanDb::load().unwrap_or_else(|e| {
        tracing::warn!("could not load pacman db: {e} — proceeding without ownership checks");
        PacmanDb::default()
    });

    // The hook runs as root. Find all real user home directories to scan.
    let homes = user_homes_to_scan();
    tracing::info!("scanning user homes: {:?}", homes);

    let mut all_findings = Vec::new();

    for home_dir in homes {
        scan_for_home(packages, cfg, &db, &home_dir, &mut all_findings);
    }

    // Deduplicate by path (multiple scanners or homes may overlap).
    all_findings.dedup_by(|a, b| a.path == b.path);

    if all_findings.is_empty() {
        tracing::info!("no findings for packages: {:?}", packages);
        return Ok(());
    }

    // The hook runs without a TTY; non_interactive is always effectively true.
    let to_delete = interactive_review(all_findings, &cfg.auto_confirm, non_interactive)?;

    if to_delete.is_empty() {
        tracing::info!("no paths confirmed for deletion");
        return Ok(());
    }

    let mode = if cfg.use_trash {
        DeleteMode::Trash
    } else {
        DeleteMode::Quarantine
    };

    let result = execute(&to_delete, packages, &db, mode, "hook", dry_run)?;

    println!(
        "pacrid: removed {} paths, {} failed.",
        result.succeeded.len(),
        result.failed.len()
    );

    Ok(())
}

fn scan_for_home(
    packages: &[String],
    cfg: &crate::config::Config,
    db: &PacmanDb,
    home_dir: &std::path::Path,
    all_findings: &mut Vec<crate::scanners::Finding>,
) {
    tracing::debug!("scanning home: {}", home_dir.display());

    let ctx = ScanContext {
        removed_packages: packages.to_vec(),
        pacman_db: db.clone(),
        config: cfg.clone(),
        home_dir: home_dir.to_path_buf(),
    };

    if cfg.scanners.xdg_db {
        match crate::scanners::xdg_db::XdgDbScanner.scan(&ctx) {
            Ok(mut found) => all_findings.append(&mut found),
            Err(e) => tracing::warn!("xdg_db scanner error: {e}"),
        }
    }

    if cfg.scanners.name_heuristic {
        match crate::scanners::name_heuristic::NameHeuristicScanner.scan(&ctx) {
            Ok(mut found) => all_findings.append(&mut found),
            Err(e) => tracing::warn!("name_heuristic scanner error: {e}"),
        }
    }
}

fn read_packages_from_stdin() -> anyhow::Result<Vec<String>> {
    let stdin = io::stdin();
    let mut packages = Vec::new();
    let mut line_count: usize = 0;

    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim().to_owned();
        if !trimmed.is_empty() {
            packages.push(trimmed);
        }
        // saturating_add: line_count is bounded above by the explicit guard
        // below, so saturating arithmetic is correct and cannot wrap.
        line_count = line_count.saturating_add(1);
        // Safety invariant: bound the loop.
        if line_count > 10_000 {
            tracing::warn!("stdin exceeded 10000 lines — truncating");
            break;
        }
    }

    Ok(packages)
}
