#!/usr/bin/env bash
# pacrid installer.
#   Default: builds from source via cargo (requires git + rust).
#   --binary: downloads the latest prebuilt release tarball from GitHub.
# Usage:
#   curl -sSf https://raw.githubusercontent.com/ParkerrDev/pacrid/refs/heads/master/install.sh | bash
#   curl -sSf https://raw.githubusercontent.com/ParkerrDev/pacrid/refs/heads/master/install.sh | bash -s -- --binary
# set -euo pipefail

REPO_URL="https://github.com/ParkerrDev/pacrid"
RELEASE_BASE="${REPO_URL}/releases/latest/download"
BINARY_DEST="/usr/bin/pacrid"
HOOK_DEST="/usr/share/libalpm/hooks/pacrid.hook"
JOURNAL_DIR="/var/lib/pacrid/journal"

USE_BINARY=0
for arg in "$@"; do
    case "$arg" in
        --binary) USE_BINARY=1 ;;
        -h|--help)
            sed -n '2,7p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "unknown option: $arg (try --help)" >&2; exit 1 ;;
    esac
done

RED='\033[0;31m'
GRN='\033[0;32m'
YLW='\033[1;33m'
BLD='\033[1m'
RST='\033[0m'

die()  { echo -e "${RED}error: $*${RST}" >&2; exit 1; }
info() { echo -e "${GRN}==>${RST} ${BLD}$*${RST}"; }
warn() { echo -e "${YLW}warning: $*${RST}"; }
step() { echo -e "    ${BLD}$*${RST}"; }

# ── pre-flight ────────────────────────────────────────────────────────────────

check_arch() {
    [[ -f /etc/arch-release ]] \
        || die "pacrid is Arch Linux only. /etc/arch-release not found."
}

check_sudo() {
    command -v sudo &>/dev/null \
        || die "sudo is required to install files to /usr/bin and /usr/share."
}

ensure_rust() {
    if command -v cargo &>/dev/null; then
        info "Rust found: $(rustc --version)"
        return
    fi
    warn "Rust not found — installing via rustup (non-interactive, stable toolchain)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --default-toolchain stable
    # shellcheck source=/dev/null
    source "${CARGO_HOME:-$HOME/.cargo}/env"
    info "Rust installed: $(rustc --version)"
}

ensure_git() {
    command -v git &>/dev/null \
        || die "git is required to clone the repository. Install it with: sudo pacman -S git"
}

# ── build ─────────────────────────────────────────────────────────────────────

locate_or_clone_source() {
    # If the current directory IS the pacrid repo, build in place.
    if [[ -f Cargo.toml ]] && grep -q '^name = "pacrid"' Cargo.toml 2>/dev/null; then
        echo "$(pwd)"
        return
    fi

    local tmp
    tmp="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp'" EXIT

    info "Cloning $REPO_URL ..."
    git clone --depth 1 "$REPO_URL" "$tmp"
    echo "$tmp"
}

build() {
    local src="$1"
    info "Building pacrid (release, LTO) — this takes ~30 s on first run ..."
    cargo build --release --manifest-path "$src/Cargo.toml"
    step "Binary: $src/target/release/pacrid"
}

# ── install ───────────────────────────────────────────────────────────────────

install_files() {
    local binary="$1"
    local hook="$2"

    info "Installing binary → $BINARY_DEST"
    sudo install -Dm755 "$binary" "$BINARY_DEST"

    info "Installing pacman hook → $HOOK_DEST"
    sudo install -Dm644 "$hook" "$HOOK_DEST"

    info "Creating journal directory → $JOURNAL_DIR"
    sudo mkdir -p "$JOURNAL_DIR"

    step "All files installed."
}

# ── prebuilt-binary path ──────────────────────────────────────────────────────

detect_arch_asset() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64)  echo "pacrid-x86_64-linux.tar.gz" ;;
        aarch64) echo "pacrid-aarch64-linux.tar.gz" ;;
        *) die "no prebuilt binary for architecture: $arch (omit --binary to build from source)" ;;
    esac
}

download_release() {
    command -v curl &>/dev/null || die "curl is required for --binary"
    command -v tar  &>/dev/null || die "tar is required for --binary"

    local asset url tmp
    asset="$(detect_arch_asset)"
    url="${RELEASE_BASE}/${asset}"
    tmp="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp'" EXIT

    info "Downloading prebuilt $asset"
    step "from: $url"
    curl -fL --proto '=https' --tlsv1.2 -sS "$url" -o "$tmp/$asset" \
        || die "download failed. Omit --binary to build from source instead."

    # Verify checksum if the .sha256 is published alongside.
    if curl -fL --proto '=https' --tlsv1.2 -sS "${url}.sha256" -o "$tmp/$asset.sha256" 2>/dev/null; then
        info "Verifying SHA256"
        (cd "$tmp" && sha256sum -c "$asset.sha256" >/dev/null) \
            || die "checksum verification failed"
        step "Checksum OK"
    else
        warn "no .sha256 checksum published — skipping verification"
    fi

    tar -xzf "$tmp/$asset" -C "$tmp" \
        || die "failed to extract $asset"
    [[ -x "$tmp/pacrid"     ]] || die "tarball missing pacrid binary"
    [[ -f "$tmp/pacrid.hook" ]] || die "tarball missing pacrid.hook"

    echo "$tmp"
}

# ── verify ────────────────────────────────────────────────────────────────────

verify() {
    local ver
    ver="$("$BINARY_DEST" --version 2>&1 || true)"
    info "Installed: $ver"
    step "Hook active: $HOOK_DEST"
}

# ── summary ───────────────────────────────────────────────────────────────────

print_done() {
    echo
    echo -e "${GRN}${BLD}pacrid is installed and active.${RST}"
    echo
    echo "  The pacman hook runs automatically after every package removal."
    echo "  No configuration needed — it just works."
    echo
    echo "  Quick reference:"
    echo "    pacrid clean <pkg>    — manually scan a package's leftovers"
    echo "    pacrid undo           — restore the last auto-removal"
    echo "    pacrid sweep          — full orphan scan of the system"
    echo "    pacrid --help         — full help"
    echo
    echo "  Config file (optional): ~/.config/pacrid/config.toml"
    echo "  Journal:                $JOURNAL_DIR"
    echo
    echo "  Try it now:"
    echo -e "    ${BLD}paru -Rns <some-package-you-want-gone>${RST}"
    echo
}

# ── main ──────────────────────────────────────────────────────────────────────

main() {
    echo
    echo -e "${BLD}pacrid installer${RST}"
    echo "─────────────────────────────────────────"
    echo

    check_arch
    check_sudo

    if (( USE_BINARY )); then
        local extracted
        extracted="$(download_release)"
        install_files "$extracted/pacrid" "$extracted/pacrid.hook"
    else
        ensure_git
        ensure_rust
        local src
        src="$(locate_or_clone_source)"
        build "$src"
        install_files "$src/target/release/pacrid" "$src/hooks/pacrid.hook"
    fi

    verify
    print_done
}

main "$@"
