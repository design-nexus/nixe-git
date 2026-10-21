#!/usr/bin/env bash
# Build the current checkout into a local Arch package; optionally install it.
set -euo pipefail
install_package=false
case ${1:-} in
  '') ;;
  --install) install_package=true ;;
  *) echo "Usage: $0 [--install]" >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { echo 'Too many arguments' >&2; exit 2; }
[[ $EUID != 0 ]] || { echo 'Run this build as your normal user, not root.' >&2; exit 1; }
recipe_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
workspace=$(cd "$recipe_dir/../../.." && pwd)
for tool in cargo makepkg desktop-file-validate sha256sum python3; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 1; }
done
# Ask Cargo for its output directory, including configured CARGO_TARGET_DIR overrides.
metadata=$(cd "$workspace" && cargo metadata --no-deps --format-version 1 --locked)
target_dir=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' <<< "$metadata")
[[ ${NIXE_SKIP_BUILD:-0} == 1 ]] || (cd "$workspace" && cargo build --release --locked -p nixe-git)
output="$target_dir/packages"
mkdir -p "$output"
staging=$(mktemp -d "$output/.nixe-git.XXXXXXXX")
trap 'rm -rf -- "$staging"' EXIT
install -m755 "$target_dir/release/nixe-git" "$staging/nixe-git"
install -m644 "$recipe_dir/PKGBUILD" "$staging/PKGBUILD"
install -m644 "$workspace/apps/git/data/io.github.design_nexus.nixe.Git.desktop" "$staging/"
install -m644 "$workspace/crates/nixe-icons/apps/io.github.design_nexus.nixe.Git.svg" "$staging/"
install -m644 "$workspace/LICENSE" "$workspace/apps/git/README.md" "$workspace/apps/git/PLAN.md" "$staging/"
desktop-file-validate "$staging/io.github.design_nexus.nixe.Git.desktop"
# Freeze checksums before makepkg consumes the staged build.
(cd "$staging" && makepkg --geninteg >> PKGBUILD)
(cd "$staging" && PKGDEST="$output" makepkg --force --noconfirm)
mapfile -t packages < <(cd "$staging" && PKGDEST="$output" makepkg --packagelist)
[[ ${#packages[@]} == 1 && -f ${packages[0]} ]] || { echo 'Expected one built package' >&2; exit 1; }
package=${packages[0]}
printf 'Package: %s\n' "$package"
if $install_package; then
  # Polkit supplies a desktop password prompt when no terminal is attached.
  if [[ -t 0 ]]; then sudo pacman -U --noconfirm "$package";
  else pkexec pacman -U --noconfirm "$package"; fi
  # Retire user-local copies only after pacman has successfully installed the package.
  # Keep a reversible backup, and let the suite's launcher find the managed binary.
  state="${XDG_STATE_HOME:-$HOME/.local/state}/nixe-git/package-backups"
  mkdir -p "$state" "$HOME/.local/bin"
  backup=$(mktemp -d "$state/install.XXXXXXXX")
  for entry in "$HOME/.local/bin/nixe-git" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.github.design_nexus.nixe.Git.desktop" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/io.github.design_nexus.nixe.Git.svg"; do
    if [[ -e $entry || -L $entry ]]; then mv -- "$entry" "$backup/$(basename -- "$entry")"; fi
  done
  ln -s /usr/bin/nixe-git "$HOME/.local/bin/nixe-git"
  printf 'Installed nixe-git. Previous user-local files: %s\n' "$backup"
fi
