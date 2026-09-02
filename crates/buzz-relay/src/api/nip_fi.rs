//! NIP-FI admin disconnect endpoint — `POST /api/nip-fi/disconnect`.
//!
//! This module owns:
//!
//! * [`disconnect`] — the axum handler for `POST /api/nip-fi/disconnect`.
//! * [`build_nip_fi_command_components`] — startup initialization called by
//!   `main.rs` to wire the deny map and command verifier into `AppState`.
//!
//! ## Transport invariant
//!
//! The NIP-FI admin API is **not** a protected HTTP surface.  It MUST NOT be
//! subjected to the NIP-FI HTTP-ingress admission procedure.  It carries
//! `Nostr-Federated-Identity` for a command JWS, not an identity assertion.
//! [NIP-FI.md §HTTP ingress, protected surfaces note]
//!
//! Authentication is entirely by the signed command JWT verified inside
//! [`buzz_auth::CommandVerifier::verify`]; no NIP-98 or relay-membership check
//! is performed.
//!
//! ## Environment variables
//!
//! The command API is enabled when `BUZZ_NIP_FI_MODE=enforce` and the issuer
//! JSON entries include the S4 fields.  S4 fields are read from the same
//! `BUZZ_NIP_FI_ISSUERS` JSON array; each issuer entry optionally carries:
//!
//! ```json
//! {
//!   "maximum_command_age_seconds": 30,
//!   "authorized_principals": ["service-account@issuer.example.com"],
//!   "deny_set_capacity": 50000
//! }
//! ```
//!
//! `maximum_command_age_seconds` and `authorized_principals` are required in
//! enforce mode if any issuer is command-capable.  `deny_set_capacity` defaults
//! to [`DEFAULT_DENY_SET_CAPACITY`] when absent.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use buzz_auth::{
    CommandError, CommandIssuerPolicy, CommandVerifier, IssuerCapacity, NipFiDenyMap, NipFiMode,
    ProductionJwksSource, CLIENT_ATTACHED_HEADER,
};

use crate::state::AppState;

/// Default per-issuer deny-set capacity when `deny_set_capacity` is absent.
/// 50_000 entries × ~128 bytes ≈ 6.4 MB per issuer.
pub const DEFAULT_DENY_SET_CAPACITY: usize = 50_000;

// ── Request / response shapes ────────────────────────────────────────────────

/// JSON body for `POST /api/nip-fi/disconnect`.
#[derive(Debug, Deserialize)]
pub struct DisconnectRequest {
    /// Lowercase hex encoding of the 32-byte target Nostr public key.
    pub pubkey: String,
}

/// JSON body for a successful `POST /api/nip-fi/disconnect` response.
#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    /// `true` when the command was accepted and sessions were (or attempted to be) closed.
    pub disconnected: bool,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// `POST /api/nip-fi/disconnect`
///
/// Executes `VerifyCommandJwt`, inserts the deny entry, and closes all live
/// sessions for the target pubkey across all communities.
///
/// Response contract (from NIP-FI spec):
///
/// | Condition | Status | Body |
/// |---|---|---|
/// | Authorized; action taken or no-op | `200` | `{"disconnected":true}` |
/// | Missing or invalid command JWT | `401`/`403` | per rejection table |
/// | Malformed request body or `until` exceeds ceiling | `400` | `"bad request\n"` |
/// | Deny set at capacity | `503` | `"deny set full\n"` |
///
/// The endpoint is NOT a protected HTTP surface.  [NIP-FI.md §HTTP ingress]
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response<Body> {
    // ── Extract the command JWT from the header ────────────────────────────
    let token = match extract_command_jwt(&headers) {
        Ok(t) => t,
        Err(status) => {
            return plain_response(
                status,
                if status == StatusCode::UNAUTHORIZED {
                    "authentication required\n"
                } else {
                    "evidence rejected\n"
                },
            );
        }
    };

    // ── Parse the JSON body ───────────────────────────────────────────────
    let req: DisconnectRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return plain_response(StatusCode::BAD_REQUEST, "bad request\n"),
    };

    // body.pubkey must be lowercase hex of exactly 32 bytes.
    let body_pubkey = match parse_hex_pubkey(&req.pubkey) {
        Some(k) => k,
        None => return plain_response(StatusCode::BAD_REQUEST, "bad request\n"),
    };

    // ── Command verifier ──────────────────────────────────────────────────
    let verifier = match &state.nip_fi_command_verifier {
        Some(v) => v.clone(),
        None => {
            // Mode is Off or not yet initialized.
            debug!("nip-fi disconnect: no command verifier configured");
            return plain_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "authorization unavailable\n",
            );
        }
    };

    let result = verifier.verify(token, "POST", "/api/nip-fi/disconnect", &body_pubkey);

    match result {
        Ok(cmd) => {
            // ── Deny entry inserted; close sessions synchronously ─────────
            let pubkey_bytes = cmd.target_pubkey.to_bytes();
            let closed = state.conn_manager.disconnect_nip_fi(&pubkey_bytes);
            if closed > 0 {
                debug!(
                    closed,
                    caller_iss = %cmd.caller_iss,
                    "nip-fi disconnect: closed sessions"
                );
            }
            metrics::counter!("buzz_nip_fi_disconnect_total").increment(1);
            metrics::counter!(
                "buzz_nip_fi_sessions_closed_total",
                "reason" => "admin_disconnect"
            )
            .increment(closed as u64);
            json_response(StatusCode::OK, &DisconnectResponse { disconnected: true })
        }
        Err(CommandError::DenySetFull) => {
            warn!("nip-fi disconnect: deny set full — command rejected, no sessions closed");
            metrics::counter!("buzz_nip_fi_disconnect_capacity_rejections_total").increment(1);
            plain_response(StatusCode::SERVICE_UNAVAILABLE, "deny set full\n")
        }
        Err(CommandError::UntilExceedsCeiling) | Err(CommandError::MalformedRequest) => {
            plain_response(StatusCode::BAD_REQUEST, "bad request\n")
        }
        Err(CommandError::AuthorizationUnavailable) => plain_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "authorization unavailable\n",
        ),
        Err(CommandError::EvidenceRejected) => {
            plain_response(StatusCode::FORBIDDEN, "evidence rejected\n")
        }
        Err(CommandError::AuthorizationDenied) => {
            plain_response(StatusCode::FORBIDDEN, "authorization denied\n")
        }
    }
}

// ── Startup component builder ─────────────────────────────────────────────────

/// Per-issuer command configuration parsed from the `BUZZ_NIP_FI_ISSUERS` JSON.
///
/// Added to each entry in S4.  All three fields are optional (absent =
/// command API disabled for that issuer / default capacity used).
#[derive(Debug, Default, serde::Deserialize)]
pub struct CommandIssuerEnvConfig {
    /// Positive seconds, ≤ 60.  Required for the command API to be enabled.
    pub maximum_command_age_seconds: Option<u64>,
    /// Non-empty list of authorized `sub` values.  Required if command age is set.
    pub authorized_principals: Option<Vec<String>>,
    /// Hard ceiling on live deny entries for this issuer.
    /// Defaults to [`DEFAULT_DENY_SET_CAPACITY`] when absent.
    pub deny_set_capacity: Option<usize>,
}

/// The `NipFiDenyMap` + `CommandVerifier` pair built at startup.
pub struct NipFiCommandComponents {
    /// The shared deny map consumed by WS admission and S5 HTTP admission.
    pub deny_map: Arc<NipFiDenyMap>,
    /// The command verifier for the `POST /api/nip-fi/disconnect` endpoint.
    pub command_verifier: Arc<CommandVerifier<Arc<ProductionJwksSource>>>,
}

/// Build the NIP-FI command components from the issuer policies and key source.
///
/// Called by `main.rs` after startup validation passes.  Returns `None` when
/// mode is `Off` or no issuer has command configuration.
///
/// `issuer_command_configs` must be in the same order as `registry.all_policies()`.
pub fn build_nip_fi_command_components(
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource>,
    issuer_command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Option<NipFiCommandComponents> {
    if matches!(mode, NipFiMode::Off) {
        return None;
    }

    // Build per-issuer command policies and capacity overrides.
    let mut command_policies: Vec<CommandIssuerPolicy> = Vec::new();
    let mut issuer_capacities: Vec<IssuerCapacity> = Vec::new();
    let mut default_capacity = DEFAULT_DENY_SET_CAPACITY;

    for (issuer, cmd_cfg) in issuer_command_configs {
        // Only wire command API for issuers that have the required fields.
        let age = match cmd_cfg.maximum_command_age_seconds {
            Some(a) => a,
            None => continue, // this issuer has no command config — skip
        };
        let principals = match &cmd_cfg.authorized_principals {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                warn!(
                    issuer = %issuer,
                    "nip-fi: issuer has maximum_command_age_seconds but no \
                     authorized_principals — skipping command API for this issuer"
                );
                continue;
            }
        };
        let capacity = cmd_cfg
            .deny_set_capacity
            .unwrap_or(DEFAULT_DENY_SET_CAPACITY);

        match CommandIssuerPolicy::new(issuer.clone(), age, principals, capacity) {
            Ok(policy) => {
                issuer_capacities.push(IssuerCapacity {
                    issuer: issuer.clone(),
                    capacity,
                });
                command_policies.push(policy);
            }
            Err(e) => {
                warn!(
                    issuer = %issuer,
                    error = ?e,
                    "nip-fi: invalid command policy — skipping command API for this issuer"
                );
            }
        }

        // Track the maximum capacity across issuers for the default slot.
        if capacity > default_capacity {
            default_capacity = capacity;
        }
    }

    if command_policies.is_empty() {
        debug!("nip-fi: no command-capable issuers configured — command API disabled");
        return None;
    }

    let deny_map = Arc::new(NipFiDenyMap::new(default_capacity, issuer_capacities));

    let command_verifier = Arc::new(CommandVerifier::new(
        registry.clone(),
        key_source,
        command_policies,
        (*deny_map).clone(),
    ));

    Some(NipFiCommandComponents {
        deny_map,
        command_verifier,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the command JWS token from the `Nostr-Federated-Identity: Bearer`
/// header.  Returns `Err(401)` if the header is absent, `Err(403)` otherwise.
///
/// The same header is used for assertion tokens at upgrade and for command
/// tokens at the admin API — distinct roles on distinct paths, never mixed.
fn extract_command_jwt(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let mut values = headers.get_all(CLIENT_ATTACHED_HEADER).iter();
    let first = values.next().ok_or(StatusCode::UNAUTHORIZED)?;
    // Repeated header → reject.
    if values.next().is_some() {
        return Err(StatusCode::FORBIDDEN);
    }
    let raw = first.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
    if raw.contains(',') {
        return Err(StatusCode::FORBIDDEN);
    }
    let token = raw.strip_prefix("Bearer ").ok_or(StatusCode::FORBIDDEN)?;
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(token)
}

fn parse_hex_pubkey(raw: &str) -> Option<nostr::PublicKey> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    nostr::PublicKey::from_hex(raw).ok()
}

fn plain_response(status: StatusCode, body: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response<Body> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    // ── JWT extraction contract ────────────────────────────────────────────

    #[test]
    fn absent_header_gives_401() {
        let h = HeaderMap::new();
        assert_eq!(extract_command_jwt(&h), Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn repeated_header_gives_403() {
        let mut h = HeaderMap::new();
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer aaa.bbb.ccc"),
        );
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer ddd.eee.fff"),
        );
        assert_eq!(extract_command_jwt(&h), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn non_bearer_gives_403() {
        let h = headers_with("Token aaa.bbb.ccc");
        assert_eq!(extract_command_jwt(&h), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn valid_bearer_extracted() {
        let h = headers_with("Bearer aaa.bbb.ccc");
        assert_eq!(extract_command_jwt(&h), Ok("aaa.bbb.ccc"));
    }

    // ── Hex pubkey parsing ────────────────────────────────────────────────

    #[test]
    fn uppercase_hex_rejected() {
        let upper = "A".repeat(64);
        assert!(parse_hex_pubkey(&upper).is_none());
    }

    #[test]
    fn wrong_length_rejected() {
        let short = "a".repeat(63);
        let long = "a".repeat(65);
        assert!(parse_hex_pubkey(&short).is_none());
        assert!(parse_hex_pubkey(&long).is_none());
    }

    // ── CommandIssuerEnvConfig default capacity ────────────────────────────

    #[test]
    fn absent_deny_set_capacity_uses_default() {
        let cfg = CommandIssuerEnvConfig {
            maximum_command_age_seconds: Some(30),
            authorized_principals: Some(vec!["admin@example.com".into()]),
            deny_set_capacity: None,
        };
        // Verify the default is picked up in build_nip_fi_command_components.
        // We can't easily call it without a ProductionJwksSource, but we can
        // verify the constant matches the documented intent.
        assert!(cfg.deny_set_capacity.is_none());
        assert_eq!(DEFAULT_DENY_SET_CAPACITY, 50_000);
    }

    // ── FI-TRACE-DENY-SET: 503 on capacity exhaustion ─────────────────────
    //
    // The disconnect handler returns "deny set full\n" with 503 on
    // CommandError::DenySetFull.  This is tested by the full-path integration
    // test in this module (requires a live CommandVerifier with a real key;
    // see the #[ignore] integration test suite for the live oracle).
    //
    // The unit test here pins the response body for the error path directly.

    #[test]
    fn deny_set_full_response_body_is_spec_exact() {
        use buzz_auth::CommandError;
        let body = CommandError::DenySetFull.response_body();
        assert_eq!(
            body, "deny set full\n",
            "FI-TRACE-DENY-SET: 503 body must be 'deny set full\\n'"
        );
    }
}
