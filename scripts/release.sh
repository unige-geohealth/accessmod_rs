#!/bin/sh
# Release from main: check the tree, run every gate, set the version, commit,
# tag and push. CI makes the GitHub release from the tag.
#
#   npm run release -- <bump|version> [--dry-run]   (or sh scripts/release.sh …)
#
# The bump is computed from the latest tag (as release-it / npm version):
#   pre    next pre-release of the same stage: 6.0.0-alpha.3 -> 6.0.0-alpha.4
#          (from a release: 6.0.0 -> 6.0.1-alpha.1)
#   alpha | beta | rc   that stage: 6.0.0-alpha.3 -> 6.0.0-beta.1
#   patch | minor | major   a release: 6.0.0-rc.2 -> 6.0.0 (patch, or minor
#          / major when the pre-release already starts one), 6.0.0 -> 6.0.1
# or an explicit X.Y.Z[-(alpha|beta|rc).N]. Without argument, the candidates
# are listed.
#
# Before: update docs/coverage.md, the README status and AGENTS for the
# commits since the last tag (AGENTS.md, "Releasing").
#
# --dry-run runs the checks and the gates, then stops before changing files.
set -e
cd "$(dirname "$0")/.."

fail() {
	echo "release: $*" >&2
	exit 1
}

# The latest tag in semver order: a release after its pre-releases. sort -V
# puts "6.0.0" before "6.0.0-alpha.1", so a release sorts as "6.0.0-zzz"
# (after alpha, beta, rc).
semver_last() {
	sed '/-/!s/$/-zzz/' | sort -V | tail -1 | sed 's/-zzz$//'
}
latest=$(git tag --list 'v*' | sed 's/^v//' | semver_last)
[ -n "$latest" ] || latest=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)

# bump <keyword>: the version after $latest, or nothing for an unknown keyword.
bump() {
	core=${latest%%-*}
	pre=
	[ "$core" = "$latest" ] || pre=${latest#*-}
	major=${core%%.*}
	rest=${core#*.}
	minor=${rest%%.*}
	patch=${rest#*.}
	stage=${pre%%.*}
	n=${pre#*.}
	case $1 in
	pre)
		if [ -n "$pre" ]; then
			echo "$core-$stage.$((n + 1))"
		else
			echo "$major.$minor.$((patch + 1))-alpha.1"
		fi
		;;
	alpha | beta | rc)
		if [ -z "$pre" ]; then
			echo "$major.$minor.$((patch + 1))-$1.1"
		elif [ "$stage" = "$1" ]; then
			echo "$core-$1.$((n + 1))"
		else
			echo "$core-$1.1" # an earlier stage is refused by the order check
		fi
		;;
	patch)
		if [ -n "$pre" ]; then echo "$core"; else echo "$major.$minor.$((patch + 1))"; fi
		;;
	minor)
		if [ -n "$pre" ] && [ "$patch" = 0 ]; then
			echo "$core"
		else
			echo "$major.$((minor + 1)).0"
		fi
		;;
	major)
		if [ -n "$pre" ] && [ "$minor" = 0 ] && [ "$patch" = 0 ]; then
			echo "$core"
		else
			echo "$((major + 1)).0.0"
		fi
		;;
	esac
}

usage() {
	echo "usage: npm run release -- <bump|version> [--dry-run]" >&2
	echo "latest: v$latest; bumps:" >&2
	for k in pre alpha beta rc patch minor major; do
		printf '  %-6s %s\n' "$k" "$(bump "$k")" >&2
	done
	exit 2
}

[ -n "$1" ] || usage
[ "$#" -le 2 ] || usage
dry=0
if [ "$#" -eq 2 ]; then
	[ "$2" = "--dry-run" ] || usage
	dry=1
fi
case $1 in
pre | alpha | beta | rc | patch | minor | major) version=$(bump "$1") ;;
*) version=$1 ;;
esac

# Semantic version: X.Y.Z with an optional pre-release (-alpha.1, -beta.2, -rc.1).
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$' ||
	fail "version '$version' is not X.Y.Z or X.Y.Z-(alpha|beta|rc).N"
tag="v$version"
echo "release: v$latest -> $tag"

[ "$(git branch --show-current)" = main ] || fail "not on main"
[ -z "$(git status --porcelain)" ] || fail "the working tree is not clean"
git fetch -q origin main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || fail "main differs from origin/main"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && fail "tag $tag exists"

# The new version must sort after the latest tag.
top=$(printf '%s\n%s\n' "$latest" "$version" | semver_last)
[ "$top" = "$version" ] && [ "$latest" != "$version" ] || fail "$version is not after the latest tag v$latest"

echo "release: gates for $tag"
sh scripts/check.sh
npm ci --silent
npm run check
npm -w @accessmod/web run e2e

if [ "$dry" -eq 1 ]; then
	echo "release: dry run passed; would set $version, commit 'chore(release): $tag', tag $tag and push"
	exit 0
fi

# The version lives in the Cargo workspace, the desktop crate, the two
# npm packages and the engine version the R package installs; a version
# already set (e.g. by hand for the first release)
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
	sed -i.bak "s/^Config\/accessmod\/engine: .*/Config\/accessmod\/engine: $version/" r/DESCRIPTION
	# README badge: shields.io writes "-" as "--".
	sed -i.bak "s/version-$(echo "$current" | sed 's/-/--/g')-/version-$(echo "$version" | sed 's/-/--/g')-/" README.md
	rm -f Cargo.toml.bak desktop/Cargo.toml.bak web/package.json.bak design_system/design-system/package.json.bak README.md.bak r/DESCRIPTION.bak
	cargo update -q -w
	cargo update -q -w --manifest-path desktop/Cargo.toml
	npm install --package-lock-only --ignore-scripts --silent
	git add Cargo.toml Cargo.lock desktop/Cargo.toml desktop/Cargo.lock package-lock.json web/package.json design_system/design-system/package.json README.md r/DESCRIPTION
	git commit -q -m "chore(release): $tag"
fi
[ "$(cargo run -q -p accessmod-cli -- --version)" = "accessmod $version" ] || fail "the binary does not report $version"
grep -qxF "Config/accessmod/engine: $version" r/DESCRIPTION || fail "r/DESCRIPTION does not pin the engine $version (Config/accessmod/engine)"

git tag -a "$tag" -m "AccessMod $version"
git push -q origin main "$tag"
echo "release: pushed $tag"
