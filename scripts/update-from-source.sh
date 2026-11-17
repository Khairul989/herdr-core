#!/bin/sh
# Fork-only: what `hc update` runs for a source-built macOS core. Merges
# upstream into the configured branch (keeping local patches), then builds,
# signs, installs and restarts through install-local.sh. Progress goes to
# stderr; the last stdout line is the installed version.
set -eu
cd "$(dirname "$0")/.."
config="${XDG_CONFIG_HOME:-$HOME/.config}/herdr-core/source-update.env"
fail() {
  echo "update-from-source: $*" >&2
  exit 1
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
git fetch --quiet "${upstream%%/*}" >&2 || fail "cannot fetch ${upstream%%/*}; nothing changed"
bin="${HC_INSTALL_DIR:-$HOME/.local/bin}"
stamp="${XDG_STATE_HOME:-$HOME/.local/state}/herdr-core/source-update.head"
if git merge-base --is-ancestor "$upstream" HEAD; then
  if [ "$(cat "$stamp" 2>/dev/null || true)" = "$(git rev-parse HEAD)" ]; then
    echo "already up to date with $upstream" >&2
    "$bin/herdr-core" --version
    exit 0
  fi
else
  if ! git merge --no-edit "$upstream" >&2; then
    git merge --abort
    fail "$upstream conflicts with $HC_SOURCE_BRANCH; nothing changed, merge it by hand"
  fi
fi
before=$(shasum -a 256 "$bin/herdr-core" 2>/dev/null || true)
if ! sh scripts/install-local.sh >&2; then
  if [ "$before" = "$(shasum -a 256 "$bin/herdr-core" 2>/dev/null || true)" ]; then
    fail "build or signing failed; $HC_SOURCE_BRANCH now includes $upstream, the installed core is unchanged"
  fi
  fail "installed the new core but restarting it failed; run hc server restart"
fi
# Upstream alone lacks the local messaging patch: never call that a success.
"$bin/herdr-core" replies --help >/dev/null 2>&1 || fail "the installed core lacks hc replies: the local patch is missing"
mkdir -p "$(dirname "$stamp")"
git rev-parse HEAD > "$stamp"
"$bin/herdr-core" --version
