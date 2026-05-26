#![deny(warnings)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::must_use_candidate
)]

use anyhow::Context;
use clap::{Parser, Subcommand};
use pacrid::{
    config,
    executor::{self, DeleteMode},
    journal,
    pacman::db::PacmanDb,
    review::interactive_review,
    scanners::{
        name_heuristic::NameHeuristicScanner, orphan_deps, pacman_orphan::PacmanOrphanScanner,
        xdg_db::XdgDbScanner, ScanContext, Scanner,
    },
    util::format_bytes,
};
use std::path::PathBuf;

#[derive(Parser)]
#[allow(clippy::struct_excessive_bools)] // CLI args struct legitimately has many bool flags
#[command(
    name = "pacrid",
    version,
    about = "Leftover-file reaper for Arch Linux",
    long_about = "pacrid detects and removes files left behind after package removal."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Show what would happen without deleting anything.
    #[arg(long, global = true)]
    dry_run: bool,

    /// No prompts; auto-confirm only at the configured threshold.
    #[arg(long, global = true)]
    non_interactive: bool,

    /// Override auto-confirm level: high, medium, low, none.
    #[arg(long, global = true, value_name = "LEVEL")]
    auto_confirm: Option<String>,

    /// Skip trash; permanently delete (use with extreme caution).
    #[arg(long, global = true)]
    purge: bool,

    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,

    /// Increase verbosity.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress non-error output.
    #[arg(short, long, global = true)]
    quiet: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Internal — invoked by pacman `PostTransaction` hook.
    Hook,

    /// Manually clean leftovers for the given packages.
    Clean {
        /// Package names to clean.
        #[arg(required = true)]
        packages: Vec<String>,
    },

    /// System-wide pacman-orphan scan (slow — not on hook path).
    Sweep {
        /// Additional filesystem roots to scan.
        #[arg(long = "root", value_name = "PATH")]
        roots: Vec<PathBuf>,
    },

    /// List or remove orphan dependency packages (pacman -Qdt).
    Orphans {
        /// Remove orphans via sudo pacman -Rns.
        #[arg(long)]
        remove: bool,
    },

    /// Restore the last (or a specified) deletion from the journal.
    Undo {
        /// Journal entry ID to restore (default: most recent).
        journal_id: Option<String>,
    },

    /// Empty the pacrid quarantine directory.
    Empty,

    /// Show past pacrid actions.
    ListJournal,

    /// Debug: show xdg-db entry for a package.
    Db {
        #[command(subcommand)]
        sub: DbSubcommand,
    },
}

#[derive(Subcommand)]
enum DbSubcommand {
    /// Show what the xdg-db knows about a package.
    Check {
        /// Package name.
        pkgname: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.quiet);

    let mut cfg = config::load();
    if let Some(ref level) = cli.auto_confirm {
        cfg.auto_confirm = parse_auto_confirm_level(level)?;
    }

    dispatch_command(&cli, &cfg)
}

fn init_logging(verbose: u8, quiet: bool) {
    let log_level = if quiet {
        tracing::Level::ERROR
    } else if verbose >= 2 {
        tracing::Level::TRACE
    } else if verbose == 1 {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .init();
}

fn dispatch_command(cli: &Cli, cfg: &config::Config) -> anyhow::Result<()> {
    match &cli.command {
        Command::Hook => {
            // Hook must never exit nonzero — log internal errors and continue.
            if let Err(e) = pacrid::hook::run_hook(cli.dry_run, cli.non_interactive) {
                tracing::error!("hook error: {e}");
            }
            Ok(())
        }
        Command::Clean { packages } => {
            run_clean(packages, cfg, cli.dry_run, cli.non_interactive, cli.purge)
        }
        Command::Sweep { roots } => {
            run_sweep(roots, cfg, cli.dry_run, cli.non_interactive, cli.purge)
        }
        Command::Orphans { remove } => run_orphans(*remove, cli.dry_run),
        Command::Undo { journal_id } => run_undo(journal_id.as_deref(), cli.dry_run),
        Command::Empty => executor::empty_quarantine(cli.dry_run),
        Command::ListJournal => run_list_journal(),
        Command::Db { sub } => match sub {
            DbSubcommand::Check { pkgname } => {
                pacrid::scanners::xdg_db::print_db_entry(pkgname);
                Ok(())
            }
        },
    }
}

fn run_clean(
    packages: &[String],
    cfg: &config::Config,
    dry_run: bool,
    non_interactive: bool,
    purge: bool,
) -> anyhow::Result<()> {
    let db = PacmanDb::load().unwrap_or_else(|e| {
        tracing::warn!("could not load pacman db: {e}");
        PacmanDb::default()
    });

    let ctx = build_scan_context(packages, cfg, &db);
    let all_findings = run_scanners(&ctx, cfg);

    if all_findings.is_empty() {
        println!("No leftover files found for: {}", packages.join(", "));
        return Ok(());
    }

    let to_delete = interactive_review(all_findings, &cfg.auto_confirm, non_interactive)?;
    if to_delete.is_empty() {
        println!("Nothing to remove.");
        return Ok(());
    }

    let mode = pick_delete_mode(purge, cfg.use_trash);
    let result = executor::execute(&to_delete, packages, &db, mode, "manual", dry_run)?;
    println!(
        "Done. {} removed, {} failed.",
        result.succeeded.len(),
        result.failed.len()
    );
    Ok(())
}

fn build_scan_context(packages: &[String], cfg: &config::Config, db: &PacmanDb) -> ScanContext {
    let home_dir = pacrid::user_homes::user_homes_to_scan()
        .into_iter()
        .next()
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()));
    ScanContext {
        removed_packages: packages.to_vec(),
        pacman_db: db.clone(),
        config: cfg.clone(),
        home_dir,
    }
}

fn run_scanners(ctx: &ScanContext, cfg: &config::Config) -> Vec<pacrid::scanners::Finding> {
    let mut all = Vec::new();
    if cfg.scanners.xdg_db {
        match XdgDbScanner.scan(ctx) {
            Ok(mut f) => all.append(&mut f),
            Err(e) => tracing::warn!("xdg_db scanner: {e}"),
        }
    }
    if cfg.scanners.name_heuristic {
        match NameHeuristicScanner.scan(ctx) {
            Ok(mut f) => all.append(&mut f),
            Err(e) => tracing::warn!("name_heuristic scanner: {e}"),
        }
    }
    all.dedup_by(|a, b| a.path == b.path);
    all
}

fn pick_delete_mode(purge: bool, use_trash: bool) -> DeleteMode {
    if purge {
        DeleteMode::Purge
    } else if use_trash {
        DeleteMode::Trash
    } else {
        DeleteMode::Quarantine
    }
}

fn run_sweep(
    extra_roots: &[PathBuf],
    cfg: &config::Config,
    dry_run: bool,
    non_interactive: bool,
    purge: bool,
) -> anyhow::Result<()> {
    let db = PacmanDb::load()?;

    let mut roots = PacmanOrphanScanner::default().roots;
    roots.extend_from_slice(extra_roots);
    let scanner = PacmanOrphanScanner { roots };

    let home_dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let ctx = ScanContext {
        removed_packages: Vec::new(),
        pacman_db: db.clone(),
        config: cfg.clone(),
        home_dir,
    };

    let findings = scanner.scan(&ctx)?;

    if findings.is_empty() {
        println!("No orphaned system files found.");
        return Ok(());
    }

    println!("Found {} orphaned paths.", findings.len());

    let to_delete = interactive_review(findings, &cfg.auto_confirm, non_interactive)?;

    if to_delete.is_empty() {
        println!("Nothing to remove.");
        return Ok(());
    }

    let mode = if purge {
        DeleteMode::Purge
    } else {
        DeleteMode::Quarantine
    };
    let result = executor::execute(
        &to_delete,
        &["sweep".to_owned()],
        &db,
        mode,
        "sweep",
        dry_run,
    )?;
    println!(
        "Done. {} removed, {} failed.",
        result.succeeded.len(),
        result.failed.len()
    );

    Ok(())
}

fn run_orphans(remove: bool, dry_run: bool) -> anyhow::Result<()> {
    let orphans = orphan_deps::list_orphans()?;

    if orphans.is_empty() {
        println!("No orphan packages found.");
        return Ok(());
    }

    println!("Orphan packages:");
    for o in &orphans {
        println!("  {o}");
    }

    if remove {
        orphan_deps::remove_orphans(&orphans, dry_run)?;
    } else {
        println!("\nRun `pacrid orphans --remove` to remove them.");
    }

    Ok(())
}

fn run_undo(journal_id: Option<&str>, dry_run: bool) -> anyhow::Result<()> {
    let id = match journal_id {
        Some(id) => id.to_owned(),
        None => journal::latest_entry_id()?.context("no journal entries found")?,
    };

    let entry = journal::read_entry(&id)?;
    println!(
        "Restoring journal entry: {} ({})",
        entry.id, entry.timestamp
    );
    for action in &entry.actions {
        println!("  {} <- {}", action.original.display(), action.moved_to);
    }

    executor::undo(&entry, dry_run)?;
    Ok(())
}

fn run_list_journal() -> anyhow::Result<()> {
    let entries = journal::list_entries()?;

    if entries.is_empty() {
        println!("No journal entries.");
        return Ok(());
    }

    for e in &entries {
        let total: u64 = e.actions.iter().map(|a| a.size).sum();
        println!(
            "  {}  {:8}  packages: {}  paths: {}  size: {}",
            e.timestamp.format("%Y-%m-%d %H:%M:%S"),
            e.trigger,
            e.packages.join(", "),
            e.actions.len(),
            format_bytes(total),
        );
    }

    Ok(())
}

fn parse_auto_confirm_level(s: &str) -> anyhow::Result<config::AutoConfirmLevel> {
    match s {
        "high" => Ok(config::AutoConfirmLevel::High),
        "medium" => Ok(config::AutoConfirmLevel::Medium),
        "low" => Ok(config::AutoConfirmLevel::Low),
        "none" => Ok(config::AutoConfirmLevel::None),
        other => {
            anyhow::bail!("unknown auto-confirm level: {other}; expected high|medium|low|none")
        }
    }
}
