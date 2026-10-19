#!/bin/sh
# The version of a build: read it, decide whether this build needs a newer one, write it into the files.
#
#   .github/scripts/version.sh read          prints the version in Cargo.toml
#   .github/scripts/version.sh next          prints version=, bumped= and rerun= lines (for $GITHUB_OUTPUT)
#   .github/scripts/version.sh apply <ver>   writes <ver> into Cargo.toml, Cargo.lock and the AppStream file
#
# `next` is what makes the version rise by itself. A build of main whose version was released already (its v<version>
# tag exists on another commit) gets the next free version, and the release job then commits that to main and tags
# the commit. Nothing is bumped for:
#   - a build that is not a push to main (pull requests, other branches, tags, manual runs),
#   - a push that only changes documentation or the workflows (*.md, docs/, .github/, LICENSE, .gitignore),
#   - a commit whose message contains [skip bump] (and then nothing is released for it).
# A commit message with [minor] or [major] raises that part instead of the patch number.
#
# The environment of `next` (all set by the workflow): GITHUB_EVENT_NAME, GITHUB_REF, GITHUB_SHA, GITHUB_REPOSITORY,
# BEFORE (the commit main pointed at before the push), COMMIT_MESSAGE, and GH_TOKEN for the `gh` command.
set -eu

cd "$(dirname "$0")/../.."

read_version() {
    v="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
    case "$v" in *[!0-9.]* | "") echo "can't read a version from Cargo.toml" >&2; exit 1 ;; esac
    echo "$v"
}

# bump <version> <patch|minor|major>
bump() {
    major="${1%%.*}"; rest="${1#*.}"; minor="${rest%%.*}"; patch="${rest#*.}"
    case "$2" in
        major) echo "$((major + 1)).0.0" ;;
        minor) echo "$major.$((minor + 1)).0" ;;
        *) echo "$major.$minor.$((patch + 1))" ;;
    esac
}

# The commit a tag points at, or nothing when there is no such tag.
tag_sha() {
    sha="$(gh api "repos/$GITHUB_REPOSITORY/commits/$1" --jq .sha 2>/dev/null || true)"
    case "$sha" in *[!0-9a-f]*) sha="" ;; esac
    echo "$sha"
}

# The first parent of the commit a tag points at.
tag_parent() {
    sha="$(gh api "repos/$GITHUB_REPOSITORY/commits/$1" --jq '.parents[0].sha' 2>/dev/null || true)"
    case "$sha" in *[!0-9a-f]*) sha="" ;; esac
    echo "$sha"
}

# Whether the push changed anything besides documentation and workflows. When that can't be told, it did.
changes_code() {
    case "${BEFORE:-}" in "" | 0000000000000000000000000000000000000000) return 0 ;; esac
    files="$(git diff --name-only "$BEFORE" "$GITHUB_SHA" 2>/dev/null)" || return 0
    [ -n "$files" ] || return 0
    echo "$files" | grep -qvE '(\.md$|^docs/|^\.github/|^LICENSE$|^\.gitignore$)'
}

next() {
    cur="$(read_version)"
    keep() { echo "version=$cur"; echo "bumped=false"; echo "rerun=false"; exit 0; }
    [ "${GITHUB_EVENT_NAME:-}" = push ] && [ "${GITHUB_REF:-}" = refs/heads/main ] || keep
    case "${COMMIT_MESSAGE:-}" in *"[skip bump]"*) keep ;; esac
    changes_code || keep
    released="$(tag_sha "v$cur")"
    # a version that was not released yet is simply released as it is, and a re-run of its own build too
    [ -n "$released" ] && [ "$released" != "$GITHUB_SHA" ] || keep
    level=patch
    case "${COMMIT_MESSAGE:-}" in *"[major]"*) level=major ;; *"[minor]"*) level=minor ;; esac
    candidate="$(bump "$cur" "$level")"
    n=0
    while [ "$n" -lt 100 ]; do
        sha="$(tag_sha "v$candidate")"
        if [ -z "$sha" ]; then
            echo "version=$candidate"; echo "bumped=true"; echo "rerun=false"
            return 0
        fi
        # the release of this very commit, made by an earlier run: this is a re-run, it refreshes that release
        if [ "$(tag_parent "v$candidate")" = "$GITHUB_SHA" ]; then
            echo "version=$candidate"; echo "bumped=true"; echo "rerun=true"
            return 0
        fi
        candidate="$(bump "$candidate" patch)"
        n=$((n + 1))
    done
    echo "no free version found after $cur" >&2
    exit 1
}

# Replace the first line that matches in a file with a new line, through a temporary file (sed -i differs between
# systems).
replace_line() { # file awk-condition new-line
    awk -v new="$3" -v done=0 "done == 0 && ($2) { print new; done = 1; next } { print }" "$1" > "$1.tmp"
    mv "$1.tmp" "$1"
}

apply() {
    v="$1"
    case "$v" in *[!0-9.]* | "") echo "bad version '$v'" >&2; exit 1 ;; esac
    replace_line Cargo.toml '/^version = /' "version = \"$v\""
    # the version line right after the package called blockforge
    awk -v v="$v" '
        /^name = "blockforge"$/ { hit = 1; print; next }
        hit == 1 && /^version = / { print "version = \"" v "\""; hit = 0; next }
        { print }' Cargo.lock > Cargo.lock.tmp
    mv Cargo.lock.tmp Cargo.lock
    meta=packaging/linux/io.github.riyadmondol2006.BlockForge.metainfo.xml
    if [ -f "$meta" ] && ! grep -q "<release version=\"$v\"" "$meta"; then
        today="$(date -u +%Y-%m-%d)"
        awk -v line="    <release version=\"$v\" date=\"$today\"/>" '
            done == 0 && /<release version=/ { print line; done = 1 }
            { print }' "$meta" > "$meta.tmp"
        mv "$meta.tmp" "$meta"
    fi
    echo "Version $v"
}

case "${1:-}" in
    read) read_version ;;
    next) next ;;
    apply) apply "${2:?usage: version.sh apply <version>}" ;;
    *) echo "usage: version.sh read | next | apply <version>" >&2; exit 2 ;;
esac
