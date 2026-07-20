# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

pacrid is a single Rust binary (plus a library crate) for Arch Linux that finds and removes files left behind after `pacman` removes a package. It runs both as a CLI (`pacrid clean <pkg>`) and as a pacman `PostTransaction` hook that fires automatically on every removal, running as **root**.

Because it deletes files on other people's machines with no human in the loop, this repo treats safety as a hard constraint enforced by tooling, not convention. See "Safety rules enforced by CI" below — they will fail the build in non-obvious ways.

## Commands

```bash
cargo build                          # debug build (build.rs needs network on first build)
cargo build --release
cargo test                           # unit + integration
cargo test confidence                # single module's unit tests
cargo test --test integration_steam  # one integration test file
cargo test -- --nocapture            # show println!/tracing output
cargo fmt --all -- --check           # CI gate
cargo clippy --all-targets --locked -- -D warnings   # CI gate; must be clean
./scripts/check-rules.sh             # CI gate; project safety rules (see below)
cargo miri test --lib                # UB check on unsafe blocks (nightly; non-blocking in CI)
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, tests, `check-rules.sh`, `cargo-audit`, `cargo-deny` (bans/licenses/sources/advisories per `deny.toml`), and Miri. All but Miri block merge.

Releases: pushing a `v*` tag triggers `release.yml`, which builds x86_64 and aarch64 tarballs and publishes them with SHA256s. `install.sh --binary` pulls from `releases/latest`.

Manual/VM verification steps live in `TESTING.md` — the scanner pipeline's real behaviour (hook firing, undo, recursion guard) can only be checked on an actual Arch system.

## Architecture

The flow, end to end:

```
pacman Remove → hooks/pacrid.hook → `pacrid hook` (as root, pkg names on stdin)
  → user_homes::user_homes_to_scan()   SUDO_USER → DOAS_USER → PKEXEC_UID → /etc/passwd UID≥1000
  → per home, per scanner:
      scanners::xdg_db          compiled-in PHF map lookup, zero I/O
      scanners::name_heuristic  probes XDG dirs / dot-files / /etc / /var from name variants
  → exec_check::executable_gone()  gate: if the binary is still on PATH, suppress everything
  → confidence::score(reasons, path, size, home_dir) → High | Medium | Low
  → review::interactive_review()   auto_select() when non-interactive; inquire MultiSelect when TTY
  → executor::execute()   validate_path → compute_size → write journal → move to trash/quarantine
```

Key structural facts that aren't obvious from any single file:

- **`build.rs` generates the app database at compile time.** It shallow-clones `pawel-0/xdg-unused-data`, merges the vendored JSON in `data/apps/`, and emits `$OUT_DIR/xdg_db.rs` as a `phf::Map` keyed by *executable name*. `src/scanners/xdg_db.rs` pulls it in with `include!`. Consequences: the first build needs network access (a failed clone degrades to an empty DB with a `cargo:warning`, it does not fail the build), and adding a `data/apps/<app>.json` entry requires a rebuild before `pacrid db check <app>` reflects it.
- **Scanners must never read `$HOME`.** The hook runs as root, so `$HOME` is `/root`. `ScanContext::home_dir` carries the detected real user home; the hook loops over *all* such homes. This is also why `confidence::score` takes `home_dir` as a parameter instead of looking it up.
- **The executable gate runs before confidence scoring, and it suppresses whole packages.** If a Flatpak/AppImage shadow-installs the same app, `which` finds the binary and every finding for that package is dropped. Scanner changes that bypass this gate are a correctness bug, not a tuning choice.
- **Journal is written before any file moves** (`executor::execute` → `journal::write_entry`), so a crash mid-deletion still leaves `pacrid undo` able to restore. Deletion is a move (XDG trash when interactive, `/var/lib/pacrid/quarantine/` via `rename(2)` when root), never an unlink, unless `--purge`.
- **pacman captures a hook's stdout *and* stderr** to fold them into its log (they surface as `ALPM-SCRIPTLET` lines). So inside the hook neither stream is a terminal and `review::is_interactive()` — an `isatty(stdout)` test — can never pass on its own. `tty::TtyRedirect` opens `/dev/tty` and `dup2`s it over both streams for the duration of the prompt; `hook::review_on_terminal` is the only caller. Because a terminal being present doesn't prove a human is watching it, `PACRID_NO_PROMPT` (and `hook_prompt = false`) force the non-interactive fallback — the hook's integration tests set it, since they scan the developer's real home and would otherwise block forever on a machine where the test package has leftovers. Note that inquire renders to **stderr** while crossterm reads keys from **/dev/tty** directly, which is why redirecting output alone is sufficient and why a missed redirect produces an *invisible prompt silently blocking the pacman transaction* rather than an error.
- **`hook.rs` is the only place allowed to swallow failures.** It wraps the pipeline in `catch_unwind`, logs, and returns `Ok(())` unconditionally — a non-zero exit or panic would break the user's `pacman` transaction. It also sets `PACRID_IN_HOOK=1` up front and bails if it's already set, which is what stops `pacrid orphans --remove` from re-triggering the hook forever.
- **All user-facing output goes through `ui.rs`**, which mirrors pacman's conventions (`::` bold-blue headers, ` ->` sub-items, `warning:`/`error:`) because pacrid prints *inside* someone else's transaction. `ui::colored()` re-checks `isatty` on every call rather than caching — the hook swaps stdout for `/dev/tty` partway through the process's life. `review.rs` sizes its columns from `ui::terminal_width()`; when space runs short the evidence column is dropped before the path is truncated, and `MIN_PATH_WIDTH` is a floor that deliberately allows overflow rather than rendering a useless path.
- **`review.rs` collapses fully-orphaned directories** into single entries before display, guarded by `COLLAPSE_NEVER` and `COLLAPSE_MIN_COMPONENTS` so it can never collapse up to a shallow root like `/usr/lib`.
- `src/pacman/db.rs` parses `/var/lib/pacman/local/*/files` directly (no libalpm linkage); `owns.rs` is the ownership check `executor::validate_path` calls before every deletion.

### Invariants asserted in code

`executor::FORBIDDEN_PREFIXES` rejects bare top-level paths. `confidence::score` forces Low for anything over 1 GB and for `/var/lib`, `/srv`, `/opt` regardless of other signals. All filesystem probing uses `symlink_metadata` — symlinks are never followed. If you change scoring or add a scanner, these are the behaviours the unit and property tests defend.

## Safety rules enforced by CI

`src/lib.rs` denies `warnings`, `clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `arithmetic_side_effects`, `float_arithmetic`, `exit`, and more, with `clippy::pedantic` as warnings. `scripts/check-rules.sh` adds grep/awk-based rules that clippy can't express. These are the ones that trip people up:

| Rule | What fails |
|---|---|
| No recursion | A function whose name appears as a call inside its own body |
| No unbounded loops | A bare `loop {` anywhere in `src/` — bound every loop explicitly (see `read_packages_from_stdin`'s 10,000-line cap) |
| Max 60 lines per function | Split helpers out; `hook.rs`/`executor.rs` are already factored this way |
| Assertions | Every `pub fn` over 15 lines needs at least one `assert!` |
| No `unwrap`/`expect` in production | Only allowed under `#[cfg(test)]` or an explicit `#[allow(clippy::unwrap_used)]` |
| Every `unsafe {}` needs a `// SAFETY:` comment | Immediately preceding the block |
| `src/lib.rs` must keep its deny list | The script greps for each required `deny(...)` literally |

Test modules carry a standard `#[allow(clippy::unwrap_used, expect_used, indexing_slicing, panic, arithmetic_side_effects)]` block — match that pattern when adding tests. Tests that hit syscalls Miri doesn't implement (e.g. anything calling `which`) need `#[cfg_attr(miri, ignore)]`.

Use `saturating_*`/`checked_*` arithmetic; `arithmetic_side_effects` is denied, so a plain `+` on integers won't compile.

## Conventions

- Comments explain *why* — hidden constraints, invariants, workarounds. Not what the code does; names cover that.
- `assert!` documents invariants the caller must uphold, not recoverable errors. `assert!(!reasons.is_empty())` is right; `assert!(file.exists())` is not.
- No clippy suppression without a justification comment on the same line or above.
- Errors propagate with `?` and `anyhow::Context`.
- Branch is `master` (the README's "open a PR against `main`" is stale).

## Adding an app to the XDG database

Highest-value contribution. Add `data/apps/<name>.json`:

```json
{
  "name": "MyApp",
  "executables": ["myapp"],
  "locations": [{"file": "$XDG_CONFIG_HOME/myapp"}, {"file": "$HOME/.myapp"}]
}
```

Supported variables: `$HOME`, `$XDG_CONFIG_HOME`, `$XDG_CACHE_HOME`, `$XDG_DATA_HOME`, `$XDG_STATE_HOME`. Then `cargo build && pacrid db check myapp`. Vendored entries supplement/override upstream, so prefer also submitting them to `pawel-0/xdg-unused-data`.

## Known drift

`PKGBUILD` says `pkgver=0.1.0` while `Cargo.toml` is at `0.1.2`. The README's architecture section still names `dialoguer`; the interactive UI moved to `inquire` in `b4ff3ae`.
