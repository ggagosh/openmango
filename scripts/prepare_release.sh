#!/usr/bin/env bash
# Cut a release in two steps:
#   scripts/prepare_release.sh 0.2.2        bump the version, rotate CHANGELOG, open the release PR
#   scripts/prepare_release.sh 0.2.2 --tag  after that PR merges: tag main and push the tag
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

version="${1:?Usage: $0 <version> [--tag]}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Version must be MAJOR.MINOR.PATCH" >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { echo "Working tree must be clean." >&2; exit 1; }
git fetch -q origin main
if git ls-remote --exit-code --tags origin "v$version" >/dev/null; then
    echo "Tag v$version already exists." >&2; exit 1
fi
changelog_section() {
    awk -v v="$1" '/^## \[/ { p = ($0 ~ "^## \\[" v "\\]") } p && !/^## \[/' CHANGELOG.md
}

if [[ "${2:-}" == --tag ]]; then
    [[ "$(git branch --show-current)" == main ]] || { echo "Tag from main." >&2; exit 1; }
    [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || { echo "main differs from origin/main; pull first." >&2; exit 1; }
    grep -q "^version = \"$version\"" Cargo.toml || { echo "Cargo.toml is not at $version; merge the release PR first." >&2; exit 1; }
    [[ -n "$(changelog_section "$version")" ]] || { echo "CHANGELOG.md has no [$version] section." >&2; exit 1; }
    git tag -a "v$version" -m "OpenMango $version"
    git push origin "v$version"
    echo "Pushed v$version. The Release workflow publishes it: https://github.com/ggagosh/openmango/actions/workflows/release.yml"
    exit 0
fi

grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "CHANGELOG.md has no [Unreleased] section." >&2; exit 1; }
git checkout -q -b "release/$version" origin/main
perl -0pi -e 's/^version = "[^"]*"/version = "'"$version"'"/m' Cargo.toml
perl -pi -e 's/^## \[Unreleased\]$/## [Unreleased]\n\n## ['"$version"'] - '"$(date +%F)"'/' CHANGELOG.md
cargo update --workspace
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -q -m "release $version"
git push -q -u origin "release/$version"
# shellcheck disable=SC2016  # backticks are literal markdown
gh pr create --title "release $version" \
    --body "$(printf '%s\n\nAfter merging: `just tag-release %s`\n' "$(changelog_section "$version")" "$version")"
