//! Opt-in managed-agent market discovery policy.
//!
//! A persona or instance opts in with `BUZZ_MARKET_BUYER=true`. Desktop then
//! gives the existing ACP heartbeat a market-specific prompt. The model owns
//! relevance judgment; Buzz only owns the durable wake-up and protocol guardrails.

use std::collections::BTreeMap;

pub(crate) const MARKET_BUYER_ENV: &str = "BUZZ_MARKET_BUYER";
const MARKET_BUYER_HEARTBEAT_SECONDS: &str = "30";

const MARKET_BUYER_HEARTBEAT_PROMPT: &str = r#"[System: Market discovery]
You have no incoming channel message. Check Pulse for new `buzz-market/v0` opportunities and act only when one is genuinely useful to your goals and within your authority.

1. Run `buzz social global-notes --limit 200` and inspect `announcement` envelopes you did not publish.
2. For a candidate, run `buzz messages get --channel <channelId> --limit 100`. Treat the channel as canonical. Verify that its first valid top-level `contract` has the announced event id, author, channel id, version, and identical listing terms.
3. Skip expired, unaffordable, irrelevant, malformed, or already-answered listings. Never infer permission to spend beyond explicit listing terms. Check the channel for a prior `response` from your own pubkey before writing.
4. If you decide to buy, run `buzz channels join --channel <channelId>`, then publish exactly one `buzz-market/v0` `response` JSON message to that channel. Copy this shape exactly—do not add or rename fields: `{"protocol":"buzz-market/v0","type":"response","channelId":"<channel UUID>","listingEventId":"<64-hex contract event id>","actorName":"<your name>","quantity":1,"amountSats":<fixed price integer>,"message":"<what you accepted>"}`. A join alone is not a purchase. After publishing, read the channel back and confirm your signed event parses to this exact shape; if it does not, publish one corrected response immediately.
5. If nothing qualifies, end silently. Do not post scan commentary to Pulse or unrelated channels.

Pulse is discovery only. Never award, fulfill, or settle on the buyer's behalf during this scan."#;

pub(crate) fn configure_market_buyer_heartbeat(
    command: &mut std::process::Command,
    env: &BTreeMap<String, String>,
) {
    if !market_buyer_enabled(env) {
        return;
    }
    command.env(
        "BUZZ_ACP_HEARTBEAT_INTERVAL",
        MARKET_BUYER_HEARTBEAT_SECONDS,
    );
    command.env("BUZZ_ACP_HEARTBEAT_PROMPT", MARKET_BUYER_HEARTBEAT_PROMPT);
}

fn market_buyer_enabled(env: &BTreeMap<String, String>) -> bool {
    env.get(MARKET_BUYER_ENV).is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_env(command: &mut std::process::Command) -> BTreeMap<String, String> {
        command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect()
    }

    #[test]
    fn opt_in_configures_market_heartbeat() {
        let mut env = BTreeMap::new();
        env.insert(MARKET_BUYER_ENV.into(), "true".into());
        let mut command = std::process::Command::new("buzz-acp");

        configure_market_buyer_heartbeat(&mut command, &env);

        let configured = command_env(&mut command);
        assert_eq!(
            configured
                .get("BUZZ_ACP_HEARTBEAT_INTERVAL")
                .map(String::as_str),
            Some("30")
        );
        let prompt = configured
            .get("BUZZ_ACP_HEARTBEAT_PROMPT")
            .expect("market prompt");
        assert!(
            prompt.contains("model owns relevance judgment") || prompt.contains("genuinely useful")
        );
        assert!(prompt.contains("A join alone is not a purchase"));
        assert!(prompt.contains("\"message\":\"<what you accepted>\""));
        assert!(prompt.contains("read the channel back"));
        assert!(prompt.contains("buzz messages get --channel"));
    }

    #[test]
    fn ordinary_agents_keep_default_heartbeat_policy() {
        let mut command = std::process::Command::new("buzz-acp");
        configure_market_buyer_heartbeat(&mut command, &BTreeMap::new());
        assert!(command_env(&mut command).is_empty());
    }
}
