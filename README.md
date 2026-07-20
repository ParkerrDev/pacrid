# pacrid

**pacman removes packages. pacrid removes the mess they leave behind.**

When you uninstall a package, pacman deletes its binaries - but your configuration files, caches, and data directories are intentionally left alone. Over time these accumulate invisibly: `~/.config/steam`, `~/.cache/discord`, `~/.local/share/kiwix-desktop`. pacrid fixes this with a pacman `PostTransaction` hook that automatically finds and removes orphaned files every time you uninstall a package, with no manual steps required.

```bash
curl -sSf https://raw.githubusercontent.com/ParkerrDev/pacrid/refs/heads/master/install.sh | bash -s -- --binary
```

> Arch Linux only. Works transparently with `pacman`, `paru`, `yay`, and any pacman wrapper.
> Prefer to build it yourself? Drop the `-s -- --binary` and the installer will compile from source.

---

<details>
<summary><b>How it works</b></summary>

<br>

pacrid hooks into pacman's `PostTransaction` event. Every time a package is removed, pacrid receives the package names on stdin, runs its scanner pipeline against your home directory, and acts on what it finds — all before pacman exits.

### The scanning pipeline

Two scanners run for each removed package:

**1. XDG database scanner (`xdg_db`)**

At build time, pacrid downloads and compiles the [xdg-unused-data](https://github.com/pawel-0/xdg-unused-data) community database into a perfect hash map (PHF). This database contains known leftover paths for hundreds of applications, keyed by executable name. When a package is removed, pacrid checks the database for that package's known data paths and evaluates each one.

Additional vendored entries live in `data/apps/` for packages not yet upstream (e.g. `steam`).

**2. Name heuristic scanner (`name_heuristic`)**

For packages not in the database, pacrid generates candidate filesystem paths from the package name and its common variants (e.g. `kiwix-desktop` → `kiwix_desktop`, `kiwixdesktop`) and probes the standard XDG directories:

| Path pattern | Category |
|---|---|
| `~/.config/<name>` | Config |
| `~/.cache/<name>` | Cache |
| `~/.local/share/<name>` | Data |
| `~/.local/state/<name>` | State |
| `~/.<name>` and `~/.<name>rc` | Config (legacy dot-files) |
| `/etc/<name>`, `/etc/<name>.conf`, `/etc/<name>.d` | Config (system) |
| `/var/cache/<name>`, `/var/log/<name>` | Cache (system) |
| `/var/lib/<name>` | State (system, always Low confidence) |

### The executable safety gate

Before scanning any paths for a package, pacrid checks whether the package's binary is still present on `PATH` using the `which` crate. If the executable is still found — e.g. a Flatpak or AppImage shadow-installs the same app — all findings for that package are suppressed entirely. pacrid will never touch files for software that is still actually installed by another means.

### Confidence scoring

Every finding is scored before being acted on:

| Confidence | Conditions |
|---|---|
| **High** | XDG database entry + exe confirmed gone, OR exact name match + exe confirmed gone + path is under `~/.config`, `~/.cache`, `~/.local/share`, or `~/.local/state` |
| **Medium** | Exact name match alone, OR pacman orphan scan found it in `/etc/`, OR XDG database entry without exe confirmation |
| **Low** | Substring match only, path under `/var/lib/`, item > 1 GB, or anything else |

When you run `pacrid clean <pkg>` interactively, all findings are shown in a checkbox UI pre-selected at your configured threshold.

### Prompting from inside the hook

pacman captures a hook's stdout and stderr so it can fold them into its own log, so neither is a terminal while the hook runs. pacrid opens `/dev/tty` directly and points both streams at it for the duration of the review, which means **the hook prompts you** — the same checkbox UI as `pacrid clean`, in the middle of the pacman transaction. Confidence then only decides what is *pre-checked*: High items are ticked by default, anything below your `auto_confirm` threshold is listed unticked and one spacebar away.

This matters most for large directories. Anything over 1 GB is forced to Low confidence no matter how strong the evidence, because getting a 1.4 GB deletion wrong is expensive — but it is still shown to you rather than silently skipped.

If there is no controlling terminal — a cron job, a GUI package manager, an unattended script — there is nobody to ask, so pacrid falls back to auto-confirming at your configured threshold and never blocks the transaction. Set `hook_prompt = false` to force that behaviour always. Esc or Ctrl-C at the prompt means "remove nothing" and the transaction continues normally.

### Home directory detection

The pacman hook runs as root, which means `$HOME` is `/root`. pacrid works around this by detecting the real user's home from (in order):

1. `SUDO_USER` environment variable (sudo)
2. `DOAS_USER` environment variable (doas)
3. `PKEXEC_UID` environment variable (polkit)
4. All UID ≥ 1000 entries from `/etc/passwd` (multi-user fallback)

### Deletion and undo

Deleted files are never permanently erased by default. pacrid uses two safe-removal modes:

- **XDG Trash** (`~/.local/share/Trash`) — used for user home paths when running interactively. Files restored via your file manager.
- **Quarantine** (`/var/lib/pacrid/quarantine/`) — used when running as root (the hook) or when trash fails. Restored via `pacrid undo`.

Every deletion is written to the journal at `/var/lib/pacrid/journal/<timestamp>.json` **before** any file is moved. If pacrid crashes mid-deletion, the journal is intact and `pacrid undo` can restore whatever was moved.

</details>

---

<details>
<summary><b>Installation</b></summary>

<br>

### One-line installer (recommended)

Downloads a prebuilt binary from the [latest GitHub Release](https://github.com/ParkerrDev/pacrid/releases/latest), verifies its SHA256, and installs the pacman hook. No Rust toolchain required.

```bash
curl -sSf https://raw.githubusercontent.com/ParkerrDev/pacrid/refs/heads/master/install.sh | bash -s -- --binary
```

The script will:
1. Verify you are on Arch Linux
2. Detect your architecture (`x86_64` or `aarch64`) and pick the matching release tarball
3. Download it from `releases/latest/download/pacrid-<arch>-linux.tar.gz`
4. Verify the published SHA256 checksum
5. Install the binary to `/usr/bin/pacrid`
6. Install the pacman hook to `/usr/share/libalpm/hooks/pacrid.hook`
7. Create `/var/lib/pacrid/journal/`

### One-line installer (build from source)

Drop the `--binary` flag to compile locally instead. The script will install Rust via rustup if needed.

```bash
curl -sSf https://raw.githubusercontent.com/ParkerrDev/pacrid/refs/heads/master/install.sh | bash
```

### Build from source manually

```bash
# Prerequisites: rust (stable ≥ 1.80), git
git clone https://github.com/ParkerrDev/pacrid
cd pacrid
cargo build --release
sudo install -Dm755 target/release/pacrid /usr/bin/pacrid
sudo install -Dm644 hooks/pacrid.hook /usr/share/libalpm/hooks/pacrid.hook
sudo mkdir -p /var/lib/pacrid/journal
```

### Uninstall

```bash
sudo rm /usr/bin/pacrid /usr/share/libalpm/hooks/pacrid.hook
sudo rm -rf /var/lib/pacrid          # removes quarantine + journal
rm -rf ~/.config/pacrid              # removes user config (optional)
```

### Requirements

| Requirement | Notes |
|---|---|
| Arch Linux (or derivative) | Requires pacman and libalpm hooks |
| Rust ≥ 1.80 | Build-time only; not needed at runtime |
| Internet access at build time | `build.rs` fetches the xdg-unused-data database |

</details>

---

<details>
<summary><b>CLI reference</b></summary>

<br>

### Global flags (work with every subcommand)

| Flag | Description |
|---|---|
| `--dry-run` | Show what would be removed without deleting anything |
| `--non-interactive` | Skip prompts; auto-confirm at the configured threshold |
| `--auto-confirm <LEVEL>` | Override threshold: `high`, `medium`, `low`, `none` |
| `--purge` | Permanently delete instead of trashing/quarantining |
| `--json` | Machine-readable JSON output |
| `-v` / `-vv` | Increase verbosity (debug / trace) |
| `-q` | Suppress non-error output |

---

### `pacrid clean <pkg> [<pkg>...]`

Manually scan and remove leftovers for one or more packages. Presents an interactive checkbox UI unless `--non-interactive` is passed.

```bash
pacrid clean steam
pacrid clean discord slack --dry-run
pacrid clean kiwix-desktop --auto-confirm medium
```

---

### `pacrid sweep [--root <PATH>...]`

System-wide orphan scan using pacman's file database. Flags files not owned by any installed package. Slow — not run on the hook path.

```bash
pacrid sweep
pacrid sweep --root /opt --root /srv
```

---

### `pacrid orphans [--remove]`

List (`pacman -Qdt`) or remove orphan dependency packages.

```bash
pacrid orphans            # list
pacrid orphans --remove   # remove via sudo pacman -Rns
```

---

### `pacrid undo [<journal-id>]`

Restore files from the last (or a specific) journal entry.

```bash
pacrid undo                         # restore most recent batch
pacrid undo 2026-05-24T21-38-27Z    # restore a specific entry
```

Files moved to XDG Trash can only be restored via your file manager. Files in quarantine are restored by `rename(2)` — atomic, no data loss.

---

### `pacrid empty`

Permanently delete everything in the quarantine directory. Run this once you are confident you no longer need to undo past removals.

```bash
pacrid empty --dry-run   # preview
pacrid empty
```

---

### `pacrid list-journal`

Show a log of all past pacrid actions with timestamps, package names, path counts, and sizes.

---

### `pacrid db check <pkg>`

Debug: show what the XDG database knows about a package. Useful when adding new `data/apps/` entries.

```bash
pacrid db check steam
pacrid db check discord
```

</details>

---

<details>
<summary><b>Configuration</b></summary>

<br>

pacrid looks for a config file at `~/.config/pacrid/config.toml`. A system-wide default can be placed at `/etc/pacrid/config.toml`; the user config takes precedence on every field.

No config file is required. The defaults are safe for most users.

### Full example

```toml
# Auto-confirm level for the pacman hook (non-interactive).
# "high"   — only remove High-confidence findings automatically (default)
# "medium" — also remove Medium-confidence findings automatically
# "low"    — remove everything automatically (not recommended)
# "none"   — never remove anything automatically
auto_confirm = "high"

# Move files to XDG Trash instead of quarantine when running interactively.
# The hook always uses quarantine (runs as root, can't write user trash).
use_trash = true

# Enable/disable the pacman PostTransaction hook entirely.
hook_enabled = true

# Let the hook prompt on the controlling terminal instead of silently
# auto-confirming at the threshold above. Falls back to auto-confirm on its
# own when no terminal is attached; set false to never prompt from the hook.
hook_prompt = true

# Remove orphan dependency packages automatically after each transaction.
auto_remove_orphan_deps = false

# Extra filesystem roots to scan for leftover files.
scan_paths_extra = ["/opt/myapp-data"]

[scanners]
# XDG database scanner — looks up packages in the community database.
xdg_db = true
# Name heuristic scanner — probes standard XDG dirs by package name.
name_heuristic = true
# Full pacman orphan scan — slow, off by default (only used by `pacrid sweep`).
pacman_orphan = false

[ignore]
# Packages whose leftovers should never be touched.
packages = ["wine", "proton"]
# Specific paths that should never be removed regardless of confidence.
paths = [
    "/home/user/.config/shared-app",
]
```

### Confidence levels explained

- **High** — pacrid is very confident this is a leftover. Auto-removed by the hook. Example: `~/.config/steam` when `steam` is not on PATH and the XDG database confirms it.
- **Medium** — probably a leftover, but worth a human glance. Example: `/etc/myapp` (system config that might be shared or hand-edited).
- **Low** — suspicious but risky to auto-remove. Example: `/var/lib/myapp` (state data that might be shared between packages).

Set `auto_confirm = "medium"` to make pacrid more aggressive. Set `auto_confirm = "none"` to approve every removal manually via `pacrid clean <pkg>`.

</details>

---

<details>
<summary><b>Architecture</b></summary>

<br>

pacrid is a single Rust binary with a library core, built with Cargo. The codebase is deliberately flat — no async runtime, no global state.

### Source layout

```
pacrid/
├── build.rs                     # Fetches xdg-unused-data at build time, codegens PHF map
├── data/apps/                   # Vendored XDG entries (e.g. steam.json)
├── hooks/pacrid.hook            # Pacman PostTransaction hook definition
├── install.sh                   # One-line installer script
└── src/
    ├── main.rs                  # CLI entry point (clap derive)
    ├── lib.rs                   # Library root + global lint config
    ├── config.rs                # TOML config deserialization + defaults
    ├── confidence.rs            # Scoring: (reasons × path × size) → Confidence
    ├── exec_check.rs            # Executable presence check via `which`
    ├── executor.rs              # Safe deletion: validate → journal → move
    ├── hook.rs                  # Pacman hook entry point; per-home scanning loop
    ├── journal.rs               # JSON undo journal (write-before-delete)
    ├── review.rs                # Interactive checkbox UI (inquire::MultiSelect)
    ├── tty.rs                   # Redirects stdout/stderr to /dev/tty so the hook can prompt
    ├── user_homes.rs            # Real user home detection when running as root
    ├── util.rs                  # format_bytes, compute_size, expand_xdg_with_home
    ├── pacman/
    │   ├── db.rs                # /var/lib/pacman/local/*/files parser
    │   ├── owns.rs              # Ownership check: is this path pacman-owned?
    │   └── query.rs             # pacman -Qdt and related queries
    └── scanners/
        ├── mod.rs               # Finding, Confidence, Reason, ScanContext, Scanner trait
        ├── xdg_db.rs            # XDG database scanner (include! generated PHF map)
        ├── name_heuristic.rs    # Name-based filesystem probe scanner
        ├── pacman_orphan.rs     # System-wide unowned-file scanner (sweep command)
        └── orphan_deps.rs       # Orphan dependency package lister
```

### Data flow

```
pacman remove event
        │
        ▼
  hook.rs::run_hook()
        │  reads package names from stdin
        │  detects real user home(s) via user_homes.rs
        │  (SUDO_USER → DOAS_USER → PKEXEC_UID → /etc/passwd UID≥1000)
        │
        ├──► XdgDbScanner.scan()
        │       looks up pkg in compiled PHF map (zero I/O)
        │       expands $HOME/$XDG_* vars using ctx.home_dir
        │       checks executable_gone() → suppresses all findings if exe present
        │
        └──► NameHeuristicScanner.scan()
                generates name variants (foo, foo-bar, foo_bar, foobar)
                probes XDG dirs + dot-files + /etc + /var/cache + /var/lib
                includes ExecutableGone reason when exe is confirmed absent
                │
                ▼
        confidence::score(reasons, path, size, home_dir)
                │
                ▼
        interactive_review()
                auto_select() in non-interactive/hook mode
                MultiSelect checkbox UI in interactive mode
                │
                ▼
        executor::execute()
                validate_path() — refuses /, /usr, /home (bare), pacman-owned, ..
                compute_size() before moving
                write journal entry BEFORE touching any file
                trash::delete() → quarantine fallback if trash unavailable
```

### Build-time codegen (`build.rs`)

`build.rs` runs before compilation and:
1. Clones `xdg-unused-data` into `$OUT_DIR/xdg-data/` (shallow clone)
2. Reads additional JSON entries from `data/apps/`
3. Parses each entry into `XdgEntry { executables, locations }`
4. Writes `$OUT_DIR/xdg_db.rs` containing a `phf::Map<&str, XdgEntry>` keyed by executable name
5. `src/scanners/xdg_db.rs` brings this in via `include!(concat!(env!("OUT_DIR"), "/xdg_db.rs"))`

The result is zero-cost, zero-allocation lookups at runtime — the entire app database is a baked-in perfect hash map with no I/O on the hot path.

### Safety invariants enforced in code

1. **Never delete pacman-owned paths.** `executor::validate_path()` checks `PacmanDb::owns()` before any deletion.
2. **Never operate on bare top-level paths.** A hardcoded `FORBIDDEN_PREFIXES` list rejects `/`, `/home`, `/etc`, `/usr`, `/var`, `/boot`, etc.
3. **Never follow symlinks.** All probes use `symlink_metadata()`. Quarantine uses `rename(2)`.
4. **Never delete if the exe is still present.** The executable gate runs before any path is probed.
5. **Never auto-remove items > 1 GB.** Anything over 1 GB is forced to Low confidence.
6. **Write the journal before moving any files.** If pacrid crashes mid-deletion, the journal is still intact for `undo`.
7. **Never exit non-zero from a hook.** Hook errors are logged but never propagate to pacman.
8. **Never recurse.** `PACRID_IN_HOOK=1` is set before any child process is spawned.
9. **Never read `$HOME` from environment in scanners.** `ctx.home_dir` is always the detected real user home.
10. **Never touch `/var/lib/` paths at High confidence.** State paths are forced to Low regardless of other signals.

### Key dependencies

| Crate | Purpose |
|---|---|
| `clap` | CLI argument parsing (derive macros) |
| `phf` / `phf_codegen` | Compile-time perfect hash map for the XDG database |
| `trash` | XDG Trash spec implementation |
| `which` | Executable presence check via PATH |
| `dialoguer` | Interactive checkbox UI (`MultiSelect`) |
| `walkdir` | Symlink-safe recursive directory traversal |
| `chrono` | Timestamp generation for journal IDs |
| `serde` / `serde_json` / `toml` | Config and journal serialization |
| `tracing` / `tracing-subscriber` | Structured logging with level filtering |
| `anyhow` / `thiserror` | Ergonomic error propagation |
| `libc` | `isatty(3)` for TTY detection |
| `humansize` | Human-readable byte sizes |
| `tempfile` | Isolated temporary directories in tests |

</details>

---

<details>
<summary><b>Contributing</b></summary>

<br>

Contributions are welcome. The hard requirement: **`cargo clippy -- -D warnings` must pass clean on every commit.**

### Dev setup

```bash
git clone https://github.com/ParkerrDev/pacrid
cd pacrid
cargo build                       # debug build
cargo test                        # all tests
cargo clippy -- -D warnings       # must be clean
cargo build --release             # release build
```

### Adding an app to the XDG database

The most impactful contribution is adding a `data/apps/<appname>.json` entry for a package that leaves files behind:

```json
{
  "name": "MyApp",
  "executables": ["myapp", "myapp-helper"],
  "locations": [
    {"file": "$HOME/.myapp"},
    {"file": "$XDG_CONFIG_HOME/myapp"},
    {"file": "$XDG_CACHE_HOME/myapp"},
    {"file": "$XDG_DATA_HOME/myapp"}
  ]
}
```

Supported path variables: `$HOME`, `$XDG_CONFIG_HOME`, `$XDG_CACHE_HOME`, `$XDG_DATA_HOME`, `$XDG_STATE_HOME`.

Verify it compiled in correctly:

```bash
cargo build
pacrid db check myapp
```

Consider also submitting the entry upstream to [xdg-unused-data](https://github.com/pawel-0/xdg-unused-data) so all tools using that database benefit.

### Making a pull request

1. **Fork** the repository on GitHub.
2. **Create a branch**: `git checkout -b feat/add-discord-entry` or `fix/describe-the-fix`.
3. **Make focused commits** — one logical change per commit, no unrelated cleanup bundled in.
4. **Run the full check suite before pushing:**
   ```bash
   cargo test
   cargo clippy -- -D warnings
   cargo build --release
   ```
5. **Open a PR against `main`** with a description that explains:
   - What problem this solves or what it adds
   - How you tested it (especially for scanner changes — what package, what leftover path)
   - Any edge cases you considered

### Code conventions

- **No descriptive comments.** Well-named identifiers describe what the code does. Comments explain *why*: hidden constraints, invariants, workarounds for specific bugs.
- **No clippy suppressions** without a justification comment.
- **No `unwrap()` on the hot path.** Use `?` and `anyhow::Context`.
- **Asserts document invariants**, not recoverable errors. `assert!(!reasons.is_empty())` is correct; `assert!(file.exists())` is not.
- **The hook must never panic or return non-zero.** Wrap risky hook code in `std::panic::catch_unwind`.
- **Never read `$HOME` in scanner code.** Always use `ctx.home_dir`.

### Running specific tests

```bash
cargo test confidence           # unit tests for the scoring function
cargo test integration_steam    # end-to-end: fake $HOME, all six steam paths
cargo test -- --nocapture       # show println! output during tests
cargo test -vv                  # verbose test output
```

</details>

---

<details>
<summary><b>Reporting bugs and getting help</b></summary>

<br>

### Reporting a bug

Please include:

1. The package name(s) involved
2. The output of `pacrid clean <pkg> -vv --dry-run`
3. The contents of `~/.config/pacrid/config.toml` if you have one
4. The output of `pacrid db check <pkg>` to show what the XDG database knows

**Common issues:**

| Symptom | Likely cause |
|---|---|
| Hook fires but finds nothing | The executable is still on PATH (Flatpak/AppImage). Run `which <pkg>` to check. |
| Hook finds things but removes nothing | `auto_confirm` is `"none"` or the findings are below the threshold. Run `pacrid clean <pkg>` interactively. |
| pacrid removed something it shouldn't have | Run `pacrid undo` immediately. Then open an issue with details. |
| Build fails | Check that you have internet access (build.rs fetches xdg-unused-data). Run `cargo build -vv` for details. |

Open an issue at: https://github.com/ParkerrDev/pacrid/issues

</details>

---

<details>
<summary><b>License</b></summary>

<br>

pacrid is released under the **GNU General Public License v3.0 or later**.

You are free to use, modify, and distribute this software under the terms of the GPL-3.0.

The [xdg-unused-data](https://github.com/pawel-0/xdg-unused-data) database is fetched at build time under its own license.

</details>
