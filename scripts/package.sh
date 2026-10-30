#!/bin/sh
# Self-contained archive of the bundled build (docs/distribution.md):
#
#   sh scripts/package.sh [target]     # default: the host target
#
# dist/accessmod-<version>-<target>.tar.gz (.zip on Windows) and its .sha256:
#   accessmod                  binary, GDAL/PROJ/GEOS/SQLite linked statically
#   share/proj/proj.db         PROJ database
#   share/accessmod/web/       built web client (accessmod serve)
#   LICENSE, COPYING, THIRD-PARTY-LICENSES.md, licenses/, README.txt
# Needs the Rust toolchain, cmake and a C/C++ compiler (GDAL is built from
# source), and Node/npm for the client.
#
# macOS signing and notarization (optional, from the environment):
#   ACCESSMOD_SIGN_IDENTITY   "Developer ID Application: …": sign, hardened runtime
#   ACCESSMOD_NOTARY_PROFILE  keychain profile (xcrun notarytool store-credentials), or
#   ACCESSMOD_NOTARY_KEY, ACCESSMOD_NOTARY_KEY_ID, ACCESSMOD_NOTARY_ISSUER
#                             App Store Connect API key (.p8 path), id and issuer (CI)
set -e
cd "$(dirname "$0")/.."

target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
name="accessmod-$version-$target"
stage="dist/$name"
exe=accessmod
case "$target" in *windows*) exe=accessmod.exe ;; esac

echo "package: $name"
cargo build -q --release --locked --target "$target" -p accessmod-cli --features bundled
npm ci --silent
npm -w @accessmod/web run build --silent

# The PROJ build of this target: its database and licence.
out=$(ls -td "target/$target/release/build/proj-sys-"*/out 2>/dev/null | head -1)
[ -f "$out/share/proj/proj.db" ] || { echo "package: no proj.db under target/$target" >&2; exit 1; }
# Sources of the native libraries, for their licences (versions from Cargo.lock).
locked() { sed -n "/^name = \"$1\"$/{n;s/^version = \"\(.*\)\"$/\1/p;}" Cargo.lock | head -1; }
src=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/ | head -1)
gdal_src="$src/gdal-src-$(locked gdal-src)"
geos_src="$src/geos-src-$(locked geos-src)"

rm -rf "$stage" "dist/$name.tar.gz" "dist/$name.zip"
mkdir -p "$stage/share/proj" "$stage/share/accessmod" "$stage/licenses"
cp "target/$target/release/$exe" "$stage/"
cp "$out/share/proj/proj.db" "$stage/share/proj/"
case "$target" in *apple-darwin)
	if [ -n "$ACCESSMOD_SIGN_IDENTITY" ]; then
		codesign --force --options runtime --timestamp --sign "$ACCESSMOD_SIGN_IDENTITY" "$stage/$exe"
		codesign --verify --strict --verbose=1 "$stage/$exe"
	fi
	if [ -n "$ACCESSMOD_NOTARY_PROFILE" ]; then
		notary="--keychain-profile $ACCESSMOD_NOTARY_PROFILE"
	elif [ -n "$ACCESSMOD_NOTARY_KEY" ]; then
		notary="--key $ACCESSMOD_NOTARY_KEY --key-id $ACCESSMOD_NOTARY_KEY_ID --issuer $ACCESSMOD_NOTARY_ISSUER"
	fi
	if [ -n "$notary" ]; then
		[ -n "$ACCESSMOD_SIGN_IDENTITY" ] || { echo "package: notarization needs ACCESSMOD_SIGN_IDENTITY" >&2; exit 1; }
		# A bare binary cannot be stapled: Gatekeeper checks the ticket online.
		ditto -c -k --keepParent "$stage/$exe" "dist/$name-notarize.zip"
		# shellcheck disable=SC2086
		result=$(xcrun notarytool submit "dist/$name-notarize.zip" $notary --wait)
		rm -f "dist/$name-notarize.zip"
		echo "$result"
		# --wait exits 0 on a rejected submission too: read the status.
		echo "$result" | grep -q "status: Accepted" || { echo "package: notarization not accepted" >&2; exit 1; }
	fi
	;;
esac
cp -R web/dist "$stage/share/accessmod/web"
cp LICENSE COPYING "$stage/"
cp "$gdal_src/source/LICENSE.TXT" "$stage/licenses/GDAL.txt"
cp "$out/share/doc/proj/COPYING" "$stage/licenses/PROJ.txt"
cp "$geos_src/source/COPYING" "$stage/licenses/GEOS.txt"
sh scripts/third-party.sh >"$stage/THIRD-PARTY-LICENSES.md"
cat >"$stage/README.txt" <<EOF
AccessMod $version ($target)

Physical accessibility to health services. Run from this directory:

  ./$exe doctor                  check GDAL, PROJ, drivers, memory and disk
  ./$exe serve                   web interface: http://127.0.0.1:5099
                                 (projects in ~/AccessMod; --data to change)
  ./$exe --help                  every command (analyses run as jobs too)

The binary finds share/ next to it: keep the directory together.
Source, documentation and issues: https://github.com/unige-geohealth/accessmod_rs
Licence: LGPL-3.0-or-later (LICENSE, COPYING); bundled libraries:
THIRD-PARTY-LICENSES.md and licenses/.
EOF

cd dist
case "$target" in
*windows*)
	rm -f "$name.zip"
	if command -v 7z >/dev/null; then 7z a -bd -tzip "$name.zip" "$name" >/dev/null; else zip -qr "$name.zip" "$name"; fi
	archive="$name.zip"
	;;
*)
	tar -czf "$name.tar.gz" "$name"
	archive="$name.tar.gz"
	;;
esac
if command -v shasum >/dev/null; then shasum -a 256 "$archive" >"$archive.sha256"; else sha256sum "$archive" >"$archive.sha256"; fi
echo "package: dist/$archive ($(du -h "$archive" | cut -f1))"
