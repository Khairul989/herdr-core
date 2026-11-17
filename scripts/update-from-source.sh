#!/bin/sh
# Fork-only: what `hc update` runs for a source-built macOS core. Merges
# upstream into the configured branch (keeping local patches), then builds,
# signs, installs and restarts through install-local.sh. Build output goes to
# stderr, plus `progress: <step>` lines when HC_PROGRESS is set. Stdout ends
# with what happened (merged=N, from=, to=, rebuilt=0|1) and the installed
# version as its last line.
set -eu
cd "$(dirname "$0")/.."
config="${XDG_CONFIG_HOME:-$HOME/.config}/herdr-core/source-update.env"
fail() {
  echo "update-from-source: $*" >&2
  exit 1
}
progress() {
  [ -z "${HC_PROGRESS:-}" ] || echo "progress: $*" >&2
}
[ -f "$config" ] || fail "missing $config"
. "$config"
[ -n "${HC_SOURCE_BRANCH:-}" ] || fail "set HC_SOURCE_BRANCH in $config"
upstream=${HC_SOURCE_UPSTREAM:-origin/master}
# The app starts hc without a login shell's PATH.
PATH="$HOME/.cargo/bin:$PATH"
export PATH
branch=$(git symbolic-ref --short -q HEAD || true)
[ "$branch" = "$HC_SOURCE_BRANCH" ] || fail "$PWD is on ${branch:-a detached HEAD}, not $HC_SOURCE_BRANCH; nothing changed"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "commit or stash the changes in $PWD first; nothing changed"
progress "Fetching $upstream"
git fetch --quiet "${upstream%%/*}" >&2 || fail "cannot fetch ${upstream%%/*}; nothing changed"
bin="${HC_INSTALL_DIR:-$HOME/.local/bin}"
stamp="${XDG_STATE_HOME:-$HOME/.local/state}/herdr-core/source-update.head"
from=$(git rev-parse HEAD)
merged=$(git rev-list --count "HEAD..$upstream")
if [ "$merged" = 0 ]; then
  if [ "$(cat "$stamp" 2>/dev/null || true)" = "$from" ]; then
    progress "No new upstream commits"
    printf 'merged=0\nfrom=%s\nto=%s\nrebuilt=0\n' "$from" "$from"
    "$bin/herdr-core" --version
    exit 0
  fi
else
  progress "Merging $merged new upstream commit(s) into $HC_SOURCE_BRANCH"
  if ! git merge --no-edit "$upstream" >&2; then
    git merge --abort
    fail "$upstream conflicts with $HC_SOURCE_BRANCH; nothing changed, merge it by hand"
  fi
fi
to=$(git rev-parse HEAD)
before=$(shasum -a 256 "$bin/herdr-core" 2>/dev/null || true)
if ! sh scripts/install-local.sh >&2; then
  if [ "$before" = "$(shasum -a 256 "$bin/herdr-core" 2>/dev/null || true)" ]; then
    fail "build or signing failed; $HC_SOURCE_BRANCH now includes $upstream, the installed core is unchanged"
  fi
  fail "installed the new core but restarting it failed; run hc server restart"
fi
progress "Checking the local patch"
# Upstream alone lacks the local messaging patch: never call that a success.
"$bin/herdr-core" replies --help >/dev/null 2>&1 || fail "the installed core lacks hc replies: the local patch is missing"
mkdir -p "$(dirname "$stamp")"
echo "$to" > "$stamp"
printf 'merged=%s\nfrom=%s\nto=%s\nrebuilt=1\n' "$merged" "$from" "$to"
"$bin/herdr-core" --version
