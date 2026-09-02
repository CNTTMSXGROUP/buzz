#!/bin/sh
set -eu

configured=false
old_ifs=$IFS
IFS=','
for encoded in ${DART_DEFINES:-}; do
  decoded=$(printf '%s' "$encoded" | base64 --decode 2>/dev/null || printf '%s' "$encoded" | base64 -D 2>/dev/null || true)
  case "$decoded" in
    BUZZ_PUSH_GATEWAY_URL=?*) configured=true ;;
  esac
done
IFS=$old_ifs

if [ "$configured" != true ]; then
  echo "error: BUZZ_PUSH_GATEWAY_URL must be supplied with --dart-define for every mobile build." >&2
  exit 1
fi
