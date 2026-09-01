#!/usr/bin/env bash
# Relay-backed two-agent sandbox for buzz-market/v0 kind:1 Pulse envelopes.
set -euo pipefail

usage() {
  cat <<'TXT'
Usage:
  market-sandbox.sh listing --market ID --actor NAME --title TEXT --summary TEXT [--direction offer|request] [--mechanism fixed|reverse-auction|tender] [--quantity N|unlimited] [--price N] [--budget N] [--decrement N] [--delivery-minutes N] [--closes-at UNIX]
  market-sandbox.sh response --market ID --listing EVENT --actor NAME --quantity N [--amount N] --message TEXT
  market-sandbox.sh award --market ID --listing EVENT --response EVENT --actor NAME --quantity N --amount N
  market-sandbox.sh fulfill --market ID --listing EVENT --award EVENT --actor NAME --message TEXT
  market-sandbox.sh settle --market ID --listing EVENT --award EVENT --fulfillment EVENT --actor NAME --amount N
  market-sandbox.sh watch --pubkey HEX [--pubkey HEX ...]

BUZZ_RELAY_URL and BUZZ_PRIVATE_KEY select the relay and signing agent. Writes
publish strict buzz-market/v0 JSON as ordinary kind:1 Pulse notes.
TXT
}

die() { echo "market-sandbox: $*" >&2; exit 1; }
need() { [[ -n "${2:-}" ]] || die "missing --$1"; }
json_write() {
  local payload="$1"
  buzz social publish --content "$payload"
}

command_name="${1:-}"
[[ -n "$command_name" ]] || { usage; exit 1; }
if [[ "$command_name" == "-h" || "$command_name" == "--help" ]]; then
  usage
  exit 0
fi
shift

market="" actor="" title="" summary="" direction="offer" mechanism="fixed"
quantity="" price="" budget="" decrement="" delivery="" closes=""
listing="" response="" award="" fulfillment="" amount="" message=""
pubkeys=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --market) market="${2:-}"; shift 2 ;;
    --actor) actor="${2:-}"; shift 2 ;;
    --title) title="${2:-}"; shift 2 ;;
    --summary) summary="${2:-}"; shift 2 ;;
    --direction) direction="${2:-}"; shift 2 ;;
    --mechanism) mechanism="${2:-}"; shift 2 ;;
    --quantity) quantity="${2:-}"; shift 2 ;;
    --price) price="${2:-}"; shift 2 ;;
    --budget) budget="${2:-}"; shift 2 ;;
    --decrement) decrement="${2:-}"; shift 2 ;;
    --delivery-minutes) delivery="${2:-}"; shift 2 ;;
    --closes-at) closes="${2:-}"; shift 2 ;;
    --listing) listing="${2:-}"; shift 2 ;;
    --response) response="${2:-}"; shift 2 ;;
    --award) award="${2:-}"; shift 2 ;;
    --fulfillment) fulfillment="${2:-}"; shift 2 ;;
    --amount) amount="${2:-}"; shift 2 ;;
    --message) message="${2:-}"; shift 2 ;;
    --pubkey) pubkeys+=("${2:-}"); shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

case "$command_name" in
  listing)
    need market "$market"; need actor "$actor"; need title "$title"; need summary "$summary"; need quantity "$quantity"
    payload=$(jq -cn \
      --arg protocol 'buzz-market/v0' --arg marketId "$market" --arg actorName "$actor" \
      --arg direction "$direction" --arg mechanism "$mechanism" --arg title "$title" \
      --arg summary "$summary" --arg quantity "$quantity" --arg price "$price" \
      --arg budget "$budget" --arg decrement "$decrement" --arg delivery "$delivery" \
      --arg closes "$closes" \
      '{protocol:$protocol,type:"listing",marketId:$marketId,version:1,listing:{actorName:$actorName,direction:$direction,mechanism:$mechanism,title:$title,summary:$summary,quantity:(if $quantity=="unlimited" then "unlimited" else ($quantity|tonumber) end)} + (if $price!="" then {priceSats:($price|tonumber)} else {} end) + (if $budget!="" then {maxBudgetSats:($budget|tonumber)} else {} end) + (if $decrement!="" then {minimumDecrementSats:($decrement|tonumber)} else {} end) + (if $delivery!="" then {deliveryMinutes:($delivery|tonumber)} else {} end) + (if $closes!="" then {closesAt:($closes|tonumber)} else {} end)}')
    json_write "$payload"
    ;;
  response)
    need market "$market"; need listing "$listing"; need actor "$actor"; need quantity "$quantity"; need message "$message"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg marketId "$market" --arg listingEventId "$listing" --arg actorName "$actor" --arg quantity "$quantity" --arg amount "$amount" --arg message "$message" \
      '{protocol:$protocol,type:"response",marketId:$marketId,listingEventId:$listingEventId,actorName:$actorName,quantity:($quantity|tonumber),message:$message} + (if $amount!="" then {amountSats:($amount|tonumber)} else {} end)')
    json_write "$payload"
    ;;
  award)
    need market "$market"; need listing "$listing"; need response "$response"; need actor "$actor"; need quantity "$quantity"; need amount "$amount"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg marketId "$market" --arg listingEventId "$listing" --arg responseEventId "$response" --arg actorName "$actor" --arg quantity "$quantity" --arg amount "$amount" \
      '{protocol:$protocol,type:"award",marketId:$marketId,listingEventId:$listingEventId,responseEventId:$responseEventId,actorName:$actorName,quantity:($quantity|tonumber),amountSats:($amount|tonumber)}')
    json_write "$payload"
    ;;
  fulfill)
    need market "$market"; need listing "$listing"; need award "$award"; need actor "$actor"; need message "$message"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg marketId "$market" --arg listingEventId "$listing" --arg awardEventId "$award" --arg actorName "$actor" --arg message "$message" \
      '{protocol:$protocol,type:"fulfillment",marketId:$marketId,listingEventId:$listingEventId,awardEventId:$awardEventId,actorName:$actorName,message:$message}')
    json_write "$payload"
    ;;
  settle)
    need market "$market"; need listing "$listing"; need award "$award"; need fulfillment "$fulfillment"; need actor "$actor"; need amount "$amount"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg marketId "$market" --arg listingEventId "$listing" --arg awardEventId "$award" --arg fulfillmentEventId "$fulfillment" --arg actorName "$actor" --arg amount "$amount" \
      '{protocol:$protocol,type:"settlement",marketId:$marketId,listingEventId:$listingEventId,awardEventId:$awardEventId,fulfillmentEventId:$fulfillmentEventId,actorName:$actorName,amountSats:($amount|tonumber)}')
    json_write "$payload"
    ;;
  watch)
    ((${#pubkeys[@]} > 0)) || die "watch requires at least one --pubkey"
    combined='[]'
    for pubkey in "${pubkeys[@]}"; do
      events=$(buzz social notes --pubkey "$pubkey" --limit 100)
      combined=$(jq -cn --argjson left "$combined" --argjson right "$events" '$left + $right')
    done
    jq '[.[] | select(.kind == 1) | . as $event | (try (.content | fromjson) catch null) as $body | select($body.protocol == "buzz-market/v0") | {id, pubkey, created_at, marketId:$body.marketId, type:$body.type, body:$body}] | sort_by(.created_at, ({listing:0,response:1,award:2,fulfillment:3,settlement:4}[.type] // 99), .id)' <<<"$combined"
    ;;
  *) usage; die "unknown command: $command_name" ;;
esac
