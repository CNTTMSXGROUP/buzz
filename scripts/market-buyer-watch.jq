def words:
  ascii_downcase
  | [scan("[[:alnum:]]+")]
  | map(select(length >= 2));

def listing_words:
  ((.listing.title // "") + " " + (.listing.summary // "")) | words;

def keyword_words($keywords):
  ($keywords | gsub(","; " ") | words);

def relevant($keywords):
  if ($keywords | length) == 0 then true
  else (listing_words as $listing | any(keyword_words($keywords)[]; . as $word | $listing | index($word)))
  end;

def positive_integer:
  type == "number" and . > 0 and . == floor;

def valid_listing:
  (.listing | type) == "object"
  and (.listing.actorName | type) == "string"
  and (.listing.title | type) == "string"
  and (.listing.summary | type) == "string"
  and (.listing.direction == "offer")
  and (.listing.mechanism as $mechanism | (["fixed", "reverse-auction", "tender"] | index($mechanism)) != null)
  and (.listing.quantity == "unlimited" or (.listing.quantity | positive_integer))
  and (.listing.closesAt == null or (.listing.closesAt | positive_integer))
  and (if .listing.mechanism == "fixed"
       then (.listing.priceSats | positive_integer)
       elif .listing.mechanism == "reverse-auction"
       then (.listing.maxBudgetSats | positive_integer)
       else (.listing.maxBudgetSats == null or (.listing.maxBudgetSats | positive_integer))
       end);

def affordable($max_sats):
  if $max_sats == null then true
  elif .listing.mechanism == "fixed" then (.listing.priceSats <= $max_sats)
  else ((.listing.maxBudgetSats // 0) <= $max_sats)
  end;

[
  .[]
  | . as $event
  | (try (.content | fromjson) catch null) as $body
  | select(
      $body.protocol == "buzz-market/v0"
      and $event.pubkey != $buyer_pubkey
      and $body.type == "announcement"
      and $body.version == 1
      and ($body.channelId | type) == "string"
      and ($body.listingEventId | test("^[0-9a-f]{64}$"))
      and ($body | valid_listing)
      and ($body.listing.quantity == "unlimited" or $body.listing.quantity >= $quantity)
      and ($body.listing.closesAt == null or $body.listing.closesAt >= $now)
      and ($body | relevant($keywords))
      and ($body | affordable($max_sats))
    )
  | {
      announcement_event_id: $event.id,
      publisher_pubkey: $event.pubkey,
      created_at: $event.created_at,
      channel_id: $body.channelId,
      listing_event_id: $body.listingEventId,
      listing: $body.listing
    }
]
| sort_by(.created_at, .announcement_event_id)
