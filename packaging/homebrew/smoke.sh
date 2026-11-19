#!/usr/bin/env bash
set -euo pipefail

tmp=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/foundry-brew.XXXXXXXX")
archive="$tmp/v0.1.999.tar.gz"
formula=foundry-ci/smoke/foundry-opencode
git archive --format=tar.gz --prefix=Foundry-OpenCode/ -o "$archive" HEAD
mkdir -p "$tmp/tap/Formula"
ruby packaging/homebrew/render.rb "file://$archive" "$archive" > "$tmp/tap/Formula/foundry-opencode.rb"
git -C "$tmp/tap" init -q
git -C "$tmp/tap" add Formula
git -C "$tmp/tap" -c user.name=CI -c user.email=ci@example.com commit -qm 'Add test formula'

export XDG_CONFIG_HOME="$tmp/config"
export HOMEBREW_DEVELOPER=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_AUTOREMOVE=1
prefix=$(brew --prefix)
test ! -e "$(brew --cellar)/foundry-opencode"
for executable in foundry-opencode foundry-opencode-gui; do
  [[ ! -e "$prefix/bin/$executable" && ! -L "$prefix/bin/$executable" ]]
done
if brew tap | grep -qx foundry-ci/smoke; then
  echo 'The temporary foundry-ci/smoke tap already exists' >&2
  exit 1
fi
brew tap foundry-ci/smoke "file://$tmp/tap"
trap 'if [[ -d "$(brew --cellar)/foundry-opencode" ]]; then brew uninstall "$formula"; fi; brew untap foundry-ci/smoke' EXIT
brew trust --formula "$formula"
brew style --formula "$formula"
brew audit --skip-style --formula "$formula"
brew install "$formula"
brew test "$formula"
test "$("$prefix/bin/foundry-opencode" version)" = 0.1.999
test "$("$prefix/bin/foundry-opencode-gui" --version)" = 0.1.999

mkdir -p "$tmp/home/.config/opencode"
printf 'keep\n' > "$tmp/home/.config/opencode/token"
HOME="$tmp/home" brew uninstall "$formula"
for executable in foundry-opencode foundry-opencode-gui; do
  [[ ! -e "$prefix/bin/$executable" && ! -L "$prefix/bin/$executable" ]]
done
grep -qx keep "$tmp/home/.config/opencode/token"
