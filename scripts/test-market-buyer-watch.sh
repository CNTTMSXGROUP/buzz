#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
filter="$root/scripts/market-buyer-watch.jq"
channel=123e4567-e89b-12d3-a456-426614174000
listing=$(printf 'a%.0s' {1..64})
announcement() {
  jq -cn --arg channel "$channel" --arg listing "$listing" --arg title "$1" --arg summary "$2" --argjson price "$3" --argjson quantity "$4" '{id:("b"*64),pubkey:("c"*64),created_at:100,content:({protocol:"buzz-market/v0",type:"announcement",channelId:$channel,version:1,listingEventId:$listing,listing:{actorName:"Seller",direction:"offer",mechanism:"fixed",title:$title,summary:$summary,quantity:$quantity,priceSats:$price}}|tojson),tags:[]}'
}
select_matches() { jq --arg keywords "$1" --arg buyer_pubkey "$(printf 'd%.0s' {1..64})" --argjson max_sats "$2" --argjson quantity 1 --argjson now 50 -f "$filter"; }

result=$(jq -cn --argjson event "$(announcement 'Daily standup digest' 'Cited channel summary' 50 1)" '[$event]' | select_matches 'digest, incident' 100)
[[ $(jq 'length' <<<"$result") == 1 ]]

result=$(jq -cn --argjson event "$(announcement 'Daily standup digest' 'Cited channel summary' 50 1)" '[$event]' | select_matches 'translation' 100)
[[ $(jq 'length' <<<"$result") == 0 ]]

result=$(jq -cn --argjson event "$(announcement 'Daily standup digest' 'Cited channel summary' 150 1)" '[$event]' | select_matches 'digest' 100)
[[ $(jq 'length' <<<"$result") == 0 ]]

expired=$(announcement 'Daily standup digest' 'Cited channel summary' 50 1 | jq '.content |= (fromjson | .listing.closesAt=40 | tojson)')
result=$(jq -cn --argjson event "$expired" '[$event]' | select_matches 'digest' 100)
[[ $(jq 'length' <<<"$result") == 0 ]]

request=$(announcement 'Need a digest' 'Requester pays' 50 1 | jq '.content |= (fromjson | .listing.direction="request" | tojson)')
result=$(jq -cn --argjson event "$request" '[$event]' | select_matches 'digest' 100)
[[ $(jq 'length' <<<"$result") == 0 ]]

self=$(announcement 'Daily standup digest' 'Cited channel summary' 50 1 | jq '.pubkey = ("d" * 64)')
result=$(jq -cn --argjson event "$self" '[$event]' | select_matches 'digest' 100)
[[ $(jq 'length' <<<"$result") == 0 ]]

malformed=$(announcement 'Daily standup digest' 'Cited channel summary' 50 1 | jq '.content |= (fromjson | .listing.priceSats="free" | tojson)')
result=$(jq -cn --argjson event "$malformed" '[$event]' | select_matches 'digest' 100)
[[ $(jq 'length' <<<"$result") == 0 ]]

duplicate=$(announcement 'Daily standup digest' 'Cited channel summary' 50 1 | jq '.id = ("e" * 64)')
result=$(jq -cn --argjson first "$(announcement 'Daily standup digest' 'Cited channel summary' 50 1)" --argjson second "$duplicate" '[$first,$second]' | select_matches 'digest' 100)
[[ $(jq 'length' <<<"$result") == 2 ]]

echo 'market buyer watcher tests passed'
