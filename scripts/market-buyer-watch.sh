#!/usr/bin/env bash
# Autonomous Pulse discovery and acceptance loop for a buzz-market/v0 buyer.
set -euo pipefail

usage() {
  cat <<'TXT'
Usage:
  market-buyer-watch.sh --actor NAME [--buyer-pubkey HEX] [--keywords WORDS] [--max-sats N] [--quantity N] [--interval-seconds N] [--since UNIX] [--state-file PATH] [--once] [--dry-run]

Polls global Pulse, selects open offer announcements matching the buyer's comma-
or space-separated keywords and budget, verifies each canonical channel contract,
joins its open channel, and writes one idempotent response. Empty keywords mean
"all offers". BUZZ_RELAY_URL and BUZZ_PRIVATE_KEY select the buyer identity.
Use --buyer-pubkey when that identity has no profile on the relay.
TXT
}

die() { echo "market-buyer-watch: $*" >&2; exit 1; }
actor="" buyer_pubkey="" keywords="" max_sats="" quantity=1 interval=15 since="" state_file="" once=false dry_run=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --actor) actor="${2:-}"; shift 2 ;;
    --buyer-pubkey) buyer_pubkey="${2:-}"; shift 2 ;;
    --keywords) keywords="${2:-}"; shift 2 ;;
    --max-sats) max_sats="${2:-}"; shift 2 ;;
    --quantity) quantity="${2:-}"; shift 2 ;;
    --interval-seconds) interval="${2:-}"; shift 2 ;;
    --since) since="${2:-}"; shift 2 ;;
    --state-file) state_file="${2:-}"; shift 2 ;;
    --once) once=true; shift ;;
    --dry-run) dry_run=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[[ -n "$actor" ]] || die "missing --actor"
[[ -z "$buyer_pubkey" || "$buyer_pubkey" =~ ^[0-9a-fA-F]{64}$ ]] || die "--buyer-pubkey must be 64 hex characters"
[[ "$quantity" =~ ^[1-9][0-9]*$ ]] || die "--quantity must be a positive integer"
[[ "$interval" =~ ^[1-9][0-9]*$ ]] || die "--interval-seconds must be a positive integer"
[[ -z "$max_sats" || "$max_sats" =~ ^[1-9][0-9]*$ ]] || die "--max-sats must be a positive integer"
[[ -z "$since" || "$since" =~ ^[0-9]+$ ]] || die "--since must be a Unix timestamp"
command -v jq >/dev/null || die "jq is required"
command -v buzz >/dev/null || die "buzz is required"
if [[ -z "$buyer_pubkey" ]]; then
  buyer_pubkey=$(buzz users get | jq -er '.[0].pubkey') || die "could not resolve buyer pubkey; pass --buyer-pubkey"
fi
buyer_pubkey=$(printf '%s' "$buyer_pubkey" | tr '[:upper:]' '[:lower:]')
if [[ -z "$state_file" ]]; then
  state_file="${XDG_STATE_HOME:-$HOME/.local/state}/buzz/market-buyer-watch-${buyer_pubkey}.json"
fi
mkdir -p "$(dirname "$state_file")"
lock_dir="${state_file}.lock"
mkdir "$lock_dir" 2>/dev/null || die "another watcher is using $state_file"
trap 'rmdir "$lock_dir" 2>/dev/null || true' EXIT
[[ -f "$state_file" ]] || printf '{"responded":{}}\n' > "$state_file"
jq -e '.responded | type == "object"' "$state_file" >/dev/null || die "invalid state file: $state_file"

already_responded() { jq -e --arg id "$1" '.responded[$id] != null' "$state_file" >/dev/null; }
remember_response() {
  local listing_id="$1" response_id="$2" next
  next=$(mktemp "${state_file}.XXXXXX")
  jq --arg listing "$listing_id" --arg response "$response_id" '.responded[$listing] = $response' "$state_file" > "$next"
  mv "$next" "$state_file"
}
contract_matches() {
  local announcement="$1" messages="$2"
  jq -e --argjson announcement "$announcement" '
    [ .[] | . as $event | (try (.content | fromjson) catch null) as $body
      | select($event.id == $announcement.listing_event_id
          and $event.pubkey == $announcement.publisher_pubkey
          and ($event.tags | any(.[]; .[0] == "h" and .[1] == $announcement.channel_id))
          and $body.protocol == "buzz-market/v0"
          and $body.type == "contract"
          and $body.version == 1
          and $body.channelId == $announcement.channel_id
          and $body.listing == $announcement.listing) ] | length == 1
  ' <<<"$messages" >/dev/null
}
existing_response() {
  local listing_id="$1" channel_id="$2" messages="$3"
  jq -er --arg listing "$listing_id" --arg channel "$channel_id" --arg buyer "$buyer_pubkey" '
    [ .[] | . as $event | (try (.content | fromjson) catch null) as $body
      | select($event.pubkey == $buyer
          and ($event.tags | any(.[]; .[0] == "h" and .[1] == $channel))
          and $body.protocol == "buzz-market/v0"
          and $body.type == "response"
          and $body.channelId == $channel
          and $body.listingEventId == $listing) ]
    | sort_by(.created_at, .id)
    | .[0].id
  ' <<<"$messages"
}
scan_once() {
  local now notes matches announcement channel listing price messages response response_id
  now=$(date +%s)
  local args=(social global-notes --limit 200)
  [[ -n "$since" ]] && args+=(--since "$since")
  notes=$(buzz "${args[@]}")
  matches=$(jq --arg keywords "$keywords" --arg buyer_pubkey "$buyer_pubkey" --argjson quantity "$quantity" --argjson now "$now" --argjson max_sats "${max_sats:-null}" -f "$(dirname "$0")/market-buyer-watch.jq" <<<"$notes")
  while IFS= read -r announcement; do
    listing=$(jq -r '.listing_event_id' <<<"$announcement")
    already_responded "$listing" && continue
    channel=$(jq -r '.channel_id' <<<"$announcement")
    price=$(jq -r 'if .listing.mechanism == "fixed" then .listing.priceSats else (.listing.maxBudgetSats // empty) end' <<<"$announcement")
    messages=$(buzz messages get --channel "$channel" --limit 100)
    if ! contract_matches "$announcement" "$messages"; then
      echo "market-buyer-watch: rejected unverified announcement $listing" >&2
      continue
    fi
    if response_id=$(existing_response "$listing" "$channel" "$messages"); then
      remember_response "$listing" "$response_id"
      continue
    fi
    response=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --arg listingEventId "$listing" --arg actorName "$actor" --argjson quantity "$quantity" --arg amount "$price" '{protocol:$protocol,type:"response",channelId:$channelId,listingEventId:$listingEventId,actorName:$actorName,quantity:$quantity,message:"Automatically accepted after Pulse relevance and budget checks."} + (if $amount != "" then {amountSats:($amount|tonumber)} else {} end)')
    if $dry_run; then
      jq -cn --argjson announcement "$announcement" --argjson response "$response" '{action:"would-respond",announcement:$announcement,response:$response}'
      continue
    fi
    # Open channels are globally readable but NIP-29 writes require membership.
    buzz channels join --channel "$channel" >/dev/null
    response_id=$(buzz messages send --channel "$channel" --content "$response" | jq -er '.event_id')
    remember_response "$listing" "$response_id"
    jq -cn --arg channel_id "$channel" --arg listing_event_id "$listing" --arg response_event_id "$response_id" '{action:"responded",channel_id:$channel_id,listing_event_id:$listing_event_id,response_event_id:$response_event_id}'
  done < <(jq -c '.[]' <<<"$matches")
}

while true; do
  scan_once
  $once && break
  sleep "$interval"
done
