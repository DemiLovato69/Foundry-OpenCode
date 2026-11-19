#!/usr/bin/env bash
set -euo pipefail

version=$1
tap=$2
branch=automation/foundry-opencode
formula=Formula/foundry-opencode.rb
tmp=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/foundry-tap.XXXXXXXX")
archive="$tmp/source.tar.gz"
url="https://github.com/$GITHUB_REPOSITORY/archive/refs/tags/v$version.tar.gz"
curl --fail --silent --show-error --location --retry 3 --proto '=https' --proto-redir '=https' "$url" -o "$archive"
ruby packaging/homebrew/render.rb "$url" "$archive" > "$tmp/formula.rb"

git_auth() {
  git -c credential.helper= -c 'credential.helper=!gh auth git-credential' "$@"
}

git_auth clone "https://github.com/$tap.git" "$tmp/tap"
cd "$tmp/tap"
base=$(git symbolic-ref --short refs/remotes/origin/HEAD)
old=$(git rev-parse --verify "refs/remotes/origin/$branch" 2>/dev/null || true)

for ref in "$base" "${old:-$base}"; do
  existing=$(git show "$ref:$formula" 2>/dev/null || true)
  current=$(sed -nE 's@^  url "[^" ]*/v([0-9]+\.[0-9]+\.[0-9]+)\.tar\.gz"$@\1@p' <<< "$existing")
  if [[ -n "$existing" && -z "$current" ]]; then
    echo "Cannot read the version of $formula at $ref" >&2
    exit 1
  fi
  if [[ -n "$current" && "$current" != "$version" && "$(printf '%s\n' "$version" "$current" | sort -V | tail -1)" == "$current" ]]; then
    echo "Tap already has a newer formula ($current); skipping $version"
    exit 0
  fi
  if [[ "$current" == "$version" ]]; then
    if ! diff -q <(printf '%s\n' "$existing") "$tmp/formula.rb"; then
      echo "Formula already exists for $version with different contents" >&2
      exit 1
    fi
    [[ "$ref" != "$base" ]] || exit 0
  fi
done

mkdir -p Formula
cp "$tmp/formula.rb" "$formula"
git add "$formula"
git -c user.name='github-actions[bot]' -c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
  commit -m "foundry-opencode $version"
if [[ -z "$old" || "$(git rev-parse HEAD^{tree})" != "$(git rev-parse "$old^{tree}")" ]]; then
  git_auth push "--force-with-lease=refs/heads/$branch:$old" origin "HEAD:refs/heads/$branch"
fi

pr=$(gh pr list --repo "$tap" --head "$branch" --state open --json number --jq '.[] | .number')
if [[ -z "$pr" ]]; then
  gh pr create --repo "$tap" --base "${base#origin/}" --head "$branch" \
    --title 'Update Foundry OpenCode formula' --body 'The formula pins the SHA-256 of the new tagged source archive.'
fi
