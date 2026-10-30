#!/bin/sh
# Release from main: check the tree, run every gate, set the version, commit,
# tag and push. CI makes the GitHub release from the tag.
#
#   sh scripts/release.sh 6.0.0-alpha.2 [--dry-run]
#
# --dry-run runs the checks and the gates, then stops before changing files.
set -e
cd "$(dirname "$0")/.."

usage() {
	echo "usage: sh scripts/release.sh <version> [--dry-run]" >&2
	exit 2
}

version=$1
dry=0
[ -n "$version" ] || usage
[ "$#" -le 2 ] || usage
if [ "$#" -eq 2 ]; then
	[ "$2" = "--dry-run" ] || usage
	dry=1
fi

fail() {
	echo "release: $*" >&2
	exit 1
}

# Semantic version: X.Y.Z with an optional pre-release (-alpha.1, -beta.2, -rc.1).
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$' ||
	fail "version '$version' is not X.Y.Z or X.Y.Z-(alpha|beta|rc).N"
tag="v$version"

[ "$(git branch --show-current)" = main ] || fail "not on main"
[ -z "$(git status --porcelain)" ] || fail "the working tree is not clean"
git fetch -q origin main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || fail "main differs from origin/main"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && fail "tag $tag exists"

# The new version must sort after the latest tag, in semver order: a
# release after its pre-releases. sort -V puts "6.0.0" before
# "6.0.0-alpha.1", so a release sorts as "6.0.0-zzz" (after alpha, beta, rc).
semver_last() {
	sed '/-/!s/$/-zzz/' | sort -V | tail -1 | sed 's/-zzz$//'
}
latest=$(git tag --list 'v*' | sed 's/^v//' | semver_last)
if [ -n "$latest" ]; then
	top=$(printf '%s\n%s\n' "$latest" "$version" | semver_last)
	[ "$top" = "$version" ] && [ "$latest" != "$version" ] || fail "$version is not after the latest tag v$latest"
fi

echo "release: gates for $tag"
sh scripts/check.sh
npm ci --silent
npm run check
npm -w @accessmod/web run e2e

if [ "$dry" -eq 1 ]; then
	echo "release: dry run passed; would set $version, commit 'chore(release): $tag', tag $tag and push"
	exit 0
fi

# The version lives in the Cargo workspace, the desktop crate and the two
# npm packages; a version already set (e.g. by hand for the first release)
# is only tagged.
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ "$current" != "$version" ]; then
	for f in Cargo.toml desktop/Cargo.toml; do
		sed -i.bak "s/^version = \"$current\"$/version = \"$version\"/" "$f"
	done
	for f in web/package.json design_system/design-system/package.json; do
		sed -i.bak "s/\"version\": \"$current\"/\"version\": \"$version\"/" "$f"
	done
	sed -i.bak "s/\"@accessmod\/ui\": \"$current\"/\"@accessmod\/ui\": \"$version\"/" web/package.json
	# README badge: shields.io writes "-" as "--".
	sed -i.bak "s/version-$(echo "$current" | sed 's/-/--/g')-/version-$(echo "$version" | sed 's/-/--/g')-/" README.md
	rm -f Cargo.toml.bak desktop/Cargo.toml.bak web/package.json.bak design_system/design-system/package.json.bak README.md.bak
	cargo update -q -w
	cargo update -q -w --manifest-path desktop/Cargo.toml
	npm install --package-lock-only --ignore-scripts --silent
	git add Cargo.toml Cargo.lock desktop/Cargo.toml desktop/Cargo.lock package-lock.json web/package.json design_system/design-system/package.json README.md
	git commit -q -m "chore(release): $tag"
fi
[ "$(cargo run -q -p accessmod-cli -- --version)" = "accessmod $version" ] || fail "the binary does not report $version"

git tag -a "$tag" -m "AccessMod $version"
git push -q origin main "$tag"
echo "release: pushed $tag"
