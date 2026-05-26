# Testing pacrid

## Automated tests

```sh
cargo test           # unit + integration (21 tests)
cargo clippy -- -D warnings  # zero warnings required
cargo build --release
```

## VM test plan

Use a clean Arch Linux VM (or container) to test end-to-end.

### Setup

```sh
# 1. Build and install pacrid
git clone https://github.com/sauccydev/pacrid
cd pacrid
makepkg -si

# 2. Verify the hook is installed
ls /usr/share/libalpm/hooks/pacrid.hook
```

### Test 1: steam leftover removal

```sh
# Install steam (creates ~/.steam, ~/.local/share/Steam, etc. on first run)
sudo pacman -S steam
steam &
sleep 10
kill %1

# Verify leftover dirs exist
ls ~/.steam ~/.local/share/Steam ~/.config/steam 2>/dev/null

# Remove steam — pacrid hook fires automatically
sudo pacman -Rns steam

# Expected: pacrid presents an interactive prompt listing High-confidence findings:
#   ~/.steam            (High)  [xdg_db + exe_gone]
#   ~/.local/share/Steam (High) [xdg_db + exe_gone]
#   ~/.steampath        (High)  [xdg_db + exe_gone]
#   ~/.steampid         (High)  [xdg_db + exe_gone]
#   ~/.config/steam     (High)  [name_match + exe_gone]
#   ~/.cache/steam      (High)  [name_match + exe_gone]
```

### Test 2: dry-run mode

```sh
# Should show findings without deleting anything
pacrid clean steam --dry-run
```

### Test 3: manual clean

```sh
# After removing a package, clean leftovers manually
sudo pacman -Rns htop
pacrid clean htop
```

### Test 4: undo

```sh
# After pacrid removes files, restore them
pacrid list-journal        # note the entry ID
pacrid undo                # restore most recent
# OR
pacrid undo 2026-05-24T10-32-00Z

# Verify files are back
ls ~/.steam
```

### Test 5: non-interactive mode

```sh
# Only auto-removes High-confidence items, logs skipped
sudo pacman -Rns <small-pkg>
# In another terminal, simulate hook:
echo "<small-pkg>" | pacrid hook --non-interactive --dry-run
```

### Test 6: orphan detection

```sh
# Install a package that pulls in deps, then remove the top-level package
sudo pacman -S some-meta-package
sudo pacman -Rn some-meta-package  # leaves orphans

pacrid orphans         # lists orphans
pacrid orphans --remove  # removes them (triggers another hook run)
# Verify recursion guard works: no infinite loop
```

### Test 7: sweep

```sh
# Find system files not owned by any package
pacrid sweep --dry-run 2>/dev/null | head -20
# Should not take more than ~30 seconds on a typical install
```

### Test 8: hook recursion guard

```sh
# Verify the hook doesn't recurse when pacrid orphans removes packages
PACRID_IN_HOOK=1 pacrid hook <<< "steam"
# Should exit immediately with no output
echo $?  # must be 0
```

### Test 9: safety rail — refuses top-level paths

```sh
# This must refuse with a non-zero exit and an error message
pacrid clean --purge /  # should fail
pacrid clean --purge /home  # should fail
```

### Pass criteria

- [ ] Hook fires automatically after `pacman -Rns`
- [ ] All steam leftover paths found as High confidence
- [ ] Interactive prompt shown; selecting Y removes to trash
- [ ] `pacrid undo` restores files from quarantine
- [ ] `pacrid list-journal` shows the entry
- [ ] `--dry-run` never modifies filesystem
- [ ] `--non-interactive` auto-confirms only High
- [ ] Recursion guard prevents double-invocation
- [ ] Top-level paths are refused
- [ ] Hook exits 0 in all cases (even on error)
