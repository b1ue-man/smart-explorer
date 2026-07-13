#!/usr/bin/env bash
# Cross-host complete-release lock shared by Linux, WSL, and Windows.
#
# The lock is an atomically created repository file, not a PID-only advisory
# lock: Windows and WSL do not share a PID namespace or one kernel lock API.
# A hard crash deliberately leaves the file behind. Never guess that such a
# lock is stale; the operator must first verify that no release process remains.

release_lock_record() {
  local path=$1
  if [ -r "$path" ]; then
    sed -n '1,12p' "$path" 2>/dev/null || true
  else
    printf '%s\n' '(owner metadata is unreadable)'
  fi
}

release_lock_acquire() {
  local release_root=$1
  local owner=$2
  local inherited_token="${SMART_EXPLORER_RELEASE_LOCK_TOKEN:-}"
  local first_line=""

  RELEASE_LOCK_PATH="$release_root/.complete-release.lock"
  RELEASE_LOCK_OWNED=0
  RELEASE_LOCK_TOKEN=""
  mkdir -p "$release_root"

  if [ -n "$inherited_token" ]; then
    if [ -r "$RELEASE_LOCK_PATH" ]; then
      IFS= read -r first_line < "$RELEASE_LOCK_PATH" || true
    fi
    if [ "$first_line" != "token=$inherited_token" ]; then
      echo "Inherited complete-release lock token does not match $RELEASE_LOCK_PATH." >&2
      release_lock_record "$RELEASE_LOCK_PATH" >&2
      return 1
    fi
    RELEASE_LOCK_TOKEN="$inherited_token"
    export SMART_EXPLORER_RELEASE_LOCK_TOKEN="$RELEASE_LOCK_TOKEN"
    return 0
  fi

  RELEASE_LOCK_TOKEN="$(date -u +%Y%m%dT%H%M%SZ)-$$-${RANDOM}${RANDOM}"
  if ! (
    set -o noclobber
    umask 077
    {
      printf 'token=%s\n' "$RELEASE_LOCK_TOKEN"
      printf 'owner=%s\n' "$owner"
      printf 'pid=%s\n' "$$"
      printf 'host=%s\n' "${HOSTNAME:-unknown}"
      printf 'started_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } > "$RELEASE_LOCK_PATH"
  ) 2>/dev/null; then
    echo "Another complete release already owns $RELEASE_LOCK_PATH:" >&2
    release_lock_record "$RELEASE_LOCK_PATH" >&2
    echo "If the owner crashed, verify that no Windows, WSL, or Linux release process remains, then remove only this stale lock file." >&2
    RELEASE_LOCK_TOKEN=""
    return 1
  fi

  RELEASE_LOCK_OWNED=1
  export SMART_EXPLORER_RELEASE_LOCK_TOKEN="$RELEASE_LOCK_TOKEN"
}

release_lock_release() {
  local first_line=""
  if [ "${RELEASE_LOCK_OWNED:-0}" != "1" ] || [ -z "${RELEASE_LOCK_PATH:-}" ]; then
    return 0
  fi
  if [ -r "$RELEASE_LOCK_PATH" ]; then
    IFS= read -r first_line < "$RELEASE_LOCK_PATH" || true
  fi
  if [ "$first_line" = "token=${RELEASE_LOCK_TOKEN:-}" ]; then
    if ! rm -f -- "$RELEASE_LOCK_PATH"; then
      echo "Could not remove complete-release lock: $RELEASE_LOCK_PATH" >&2
      return 1
    fi
  else
    echo "Complete-release lock ownership changed; refusing to remove $RELEASE_LOCK_PATH." >&2
    release_lock_record "$RELEASE_LOCK_PATH" >&2
    return 1
  fi
  RELEASE_LOCK_OWNED=0
  unset SMART_EXPLORER_RELEASE_LOCK_TOKEN
}
