#!/bin/sh
set -eu

gateway_origin=
old_ifs=$IFS
IFS=','
for encoded in ${DART_DEFINES:-}; do
  decoded=$(printf '%s' "$encoded" | base64 --decode 2>/dev/null || printf '%s' "$encoded" | base64 -D 2>/dev/null || true)
  case "$decoded" in
    BUZZ_PUSH_GATEWAY_URL=*) gateway_origin=${decoded#BUZZ_PUSH_GATEWAY_URL=} ;;
  esac
done
IFS=$old_ifs

if [ -z "$gateway_origin" ]; then
  echo "error: BUZZ_PUSH_GATEWAY_URL must be supplied with --dart-define for every mobile build." >&2
  exit 1
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
dart_bin=${FLUTTER_ROOT:+$FLUTTER_ROOT/bin/dart}
if [ -z "$dart_bin" ] || [ ! -x "$dart_bin" ]; then
  dart_bin=$(command -v dart || true)
fi
if [ -z "$dart_bin" ]; then
  echo "error: Dart is required to validate BUZZ_PUSH_GATEWAY_URL." >&2
  exit 1
fi
"$dart_bin" "$script_dir/validate_push_gateway_origin.dart" "$gateway_origin"
