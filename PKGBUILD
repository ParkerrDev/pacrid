# Maintainer: Parker <sauccydev@gmail.com>
pkgname=pacrid
pkgver=0.1.0
pkgrel=1
pkgdesc="Leftover-file reaper for Arch Linux — removes ~/.config and other cruft after pacman -Rns"
arch=('x86_64' 'aarch64')
url="https://github.com/sauccydev/pacrid"
license=('GPL3')
depends=('gcc-libs' 'glibc')
makedepends=('rust' 'cargo' 'git')
backup=('etc/pacrid/config.toml')
source=("$pkgname::git+$url")
sha256sums=('SKIP')

prepare() {
    cd "$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --frozen --release --all-features
}

check() {
    cd "$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --frozen
}

package() {
    cd "$pkgname"
    install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
    install -Dm644 "hooks/pacrid.hook" "$pkgdir/usr/share/libalpm/hooks/pacrid.hook"
    install -Dm644 /dev/null "$pkgdir/etc/pacrid/config.toml"
    cat > "$pkgdir/etc/pacrid/config.toml" << 'EOF'
# pacrid configuration
# See: pacrid --help

auto_confirm = "high"          # high | medium | low | none
auto_remove_orphan_deps = false
use_trash = true               # false = quarantine to /var/lib/pacrid/quarantine
hook_enabled = true

[scanners]
xdg_db = true
name_heuristic = true
pacman_orphan = false          # only used by "pacrid sweep"

[ignore]
packages = ["linux", "linux-headers", "linux-lts"]
paths = []
EOF
    install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
