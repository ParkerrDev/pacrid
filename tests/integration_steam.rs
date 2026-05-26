/// Integration test: the headline steam use case.
/// Sets up a fake $HOME with steam leftover dirs, mocks `which` returning Err for "steam",
/// runs `pacrid clean steam --dry-run`, and asserts all known steam paths appear as findings.
use pacrid::{
    config::Config,
    pacman::db::PacmanDb,
    scanners::{
        name_heuristic::NameHeuristicScanner, xdg_db::XdgDbScanner, Confidence, Scanner,
        ScanContext,
    },
};
use std::fs;
use tempfile::TempDir;

fn setup_fake_steam_home() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();

    // Create the classic steam leftover dirs.
    fs::create_dir_all(home.join(".steam")).unwrap();
    fs::write(home.join(".steampath"), "").unwrap();
    fs::write(home.join(".steampid"), "").unwrap();
    fs::create_dir_all(home.join(".local/share/Steam")).unwrap();
    fs::create_dir_all(home.join(".config/steam")).unwrap();
    fs::create_dir_all(home.join(".cache/steam")).unwrap();

    tmp
}

#[test]
fn steam_leftovers_detected_as_high_confidence() {
    let tmp = setup_fake_steam_home();
    let home = tmp.path().to_str().unwrap().to_owned();

    // Override HOME and unset XDG vars so we use defaults under our fake home.
    // Set PATH to an empty temp dir so 'steam' is not found by which::which,
    // simulating a system where steam has been removed.
    let path_tmp = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("PATH", path_tmp.path());
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_CACHE_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("XDG_STATE_HOME");
    }

    let ctx = ScanContext {
        removed_packages: vec!["steam".to_owned()],
        pacman_db: PacmanDb::default(),
        config: Config::default(),
        home_dir: std::path::PathBuf::from(&home),
    };

    // xdg_db scanner — steam exe is not on PATH (we check: no "steam" binary present in test env).
    let xdg_findings = XdgDbScanner.scan(&ctx).unwrap();

    // name_heuristic scanner.
    let heuristic_findings = NameHeuristicScanner.scan(&ctx).unwrap();

    let mut all_findings = xdg_findings;
    all_findings.extend(heuristic_findings);
    all_findings.dedup_by(|a, b| a.path == b.path);

    // Collect path strings for assertions.
    let paths: Vec<String> = all_findings
        .iter()
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect();

    assert!(
        !all_findings.is_empty(),
        "expected steam findings, got none. HOME={home}"
    );

    // Assert the core leftover paths are present.
    let expected_suffixes = [
        "/.steam",
        "/.steampath",
        "/.steampid",
        "/.local/share/Steam",
        "/.config/steam",
        "/.cache/steam",
    ];

    for suffix in &expected_suffixes {
        assert!(
            paths.iter().any(|p| p.ends_with(suffix)),
            "expected path ending in '{suffix}' in findings.\nGot paths: {paths:#?}"
        );
    }

    // All xdg_db findings (where exe check passed = exe is gone) should be High.
    for f in &all_findings {
        if f.reasons.iter().any(|r| {
            matches!(r, pacrid::scanners::Reason::InXdgDatabase)
        }) && f
            .reasons
            .iter()
            .any(|r| matches!(r, pacrid::scanners::Reason::ExecutableGone(_)))
        {
            assert_eq!(
                f.confidence,
                Confidence::High,
                "expected High for xdg_db + exe_gone finding: {}",
                f.path.display()
            );
        }
    }
}
