#!/bin/sh
# Desktop application (Tauri, docs/desktop.md) around the bundled engine:
#
#   sh scripts/package-desktop.sh [target]     # default: the host target
#
# Builds the engine archive (scripts/package.sh), stages its binary as the
# sidecar and its share/ and licences as resources of desktop/, then
# bundles: macOS .app and .dmg, Linux .deb and .AppImage, Windows setup
# .exe (NSIS; MSI refuses pre-release versions). Bundles are copied to
# dist/ with their .sha256. Needs the Tauri CLI (cargo install tauri-cli
# --version "^2" --locked) and, on Linux, WebKitGTK 4.1 (docs/desktop.md).
# Unsigned for now.
set -e
cd "$(dirname "$0")/.."

host=$(rustc -vV | sed -n 's/^host: //p')
target=${1:-$host}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
name="accessmod-$version-$target"
exe=accessmod
case "$target" in *windows*) exe=accessmod.exe ;; esac

sh scripts/package.sh "$target"

# Stage the engine where tauri.conf.json expects it.
rm -rf desktop/binaries desktop/share desktop/licenses
mkdir -p desktop/binaries desktop/licenses
case "$target" in
*windows*) cp "dist/$name/$exe" "desktop/binaries/accessmod-$target.exe" ;;
*) cp "dist/$name/$exe" "desktop/binaries/accessmod-$target" ;;
esac
cp -R "dist/$name/share" desktop/share
cp "dist/$name/LICENSE" "dist/$name/COPYING" "dist/$name/THIRD-PARTY-LICENSES.md" desktop/licenses/
cp "dist/$name/licenses/"* desktop/licenses/

case "$target" in
*apple-darwin) bundles=app,dmg ;;
*linux*) bundles=deb,appimage ;;
*windows*) bundles=nsis ;;
esac
# The host target builds without --target (the CLI asks rustup for it).
if [ "$target" = "$host" ]; then
	(cd desktop && CI=true cargo tauri build --bundles "$bundles" --ci)
	out=desktop/target/release/bundle
else
	(cd desktop && CI=true cargo tauri build --target "$target" --bundles "$bundles" --ci)
	out="desktop/target/$target/release/bundle"
fi

# Bundles to dist/, named after the version and target.
copy() {
	[ -f "$1" ] || return 0
	cp "$1" "dist/$2"
	(cd dist && if command -v shasum >/dev/null; then shasum -a 256 "$2"; else sha256sum "$2"; fi >"$2.sha256")
	echo "package-desktop: dist/$2 ($(du -h "dist/$2" | cut -f1))"
}
for f in "$out"/dmg/*.dmg; do copy "$f" "AccessMod-$version-$target.dmg"; done
for f in "$out"/deb/*.deb; do copy "$f" "AccessMod-$version-$target.deb"; done
for f in "$out"/appimage/*.AppImage; do copy "$f" "AccessMod-$version-$target.AppImage"; done
for f in "$out"/nsis/*.exe; do copy "$f" "AccessMod-$version-$target-setup.exe"; done
