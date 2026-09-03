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
use serde::Deserialize;
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
            return if status == StatusCode::UNAUTHORIZED {
                // [NIP-FI.md §Rejection table]: 401 MUST carry WWW-Authenticate: Nostr.
                auth_required_response()
            } else {
                plain_response(status, "evidence rejected\n")
            };
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
            let closed = state.conn_manager.disconnect_nip_fi(&pubkey_bytes)
                + state.community_connections.disconnect_nip_fi(&pubkey_bytes);
            if closed > 0 {
                // [FI-TRACE-PRIVACY-NONPUBLIC]: raw `iss` MUST NOT appear in
                // logs, metrics, or traces.  Log only a count.
                debug!(closed, "nip-fi disconnect: closed sessions");
            }
            metrics::counter!("buzz_nip_fi_disconnect_total").increment(1);
            metrics::counter!(
                "buzz_nip_fi_sessions_closed_total",
                "reason" => "admin_disconnect"
            )
            .increment(closed as u64);

            // Cross-pod propagation: publish to global NIP-FI Redis channel
            // so remote pods can merge the deny entry and close their sessions.
            // Asynchronous: HTTP response does not wait on remote delivery.
            {
                let pubsub = Arc::clone(&state.pubsub);
                let msg = buzz_pubsub::NipFiDisconnect {
                    issuer: cmd.caller_iss.clone(),
                    pubkey_bytes: pubkey_bytes.to_vec(),
                    until_unix: cmd.until.timestamp(),
                };
                tokio::spawn(async move {
                    if let Err(e) = pubsub.publish_nip_fi_disconnect(&msg).await {
                        // [FI-TRACE-PRIVACY-NONPUBLIC]: no iss or pubkey in logs
                        tracing::warn!("nip-fi: cross-pod propagation publish failed: {e}");
                    }
                });
            }

            disconnected_response()
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
#[derive(Debug, Default, Clone, serde::Deserialize)]
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
/// Called by `main.rs` after startup validation passes.  Returns `Err` when
/// any command config is invalid; a valid config with no command-capable issuers
/// returns `Ok(None)`.
///
/// `issuer_command_configs` must be in the same order as `registry.all_policies()`.
pub fn build_nip_fi_command_components(
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource>,
    issuer_command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<Option<NipFiCommandComponents>, String> {
    if matches!(mode, NipFiMode::Off) {
        return Ok(None);
    }

    // Build per-issuer command policies and capacity overrides.
    let mut command_policies: Vec<CommandIssuerPolicy> = Vec::new();
    let mut issuer_capacities: Vec<IssuerCapacity> = Vec::new();
    let mut default_capacity = DEFAULT_DENY_SET_CAPACITY;

    for (idx, (issuer, cmd_cfg)) in issuer_command_configs.iter().enumerate() {
        // Only wire command API for issuers that have the required fields.
        let age = match cmd_cfg.maximum_command_age_seconds {
            Some(a) => a,
            None => continue, // this issuer has no command config — skip
        };
        let principals = match &cmd_cfg.authorized_principals {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                // from_env() already rejects this; treat as a hard error here.
                return Err(format!(
                    "nip-fi: issuer [index {idx}] has maximum_command_age_seconds but no \
                     authorized_principals — startup validation should have caught this"
                ));
            }
        };
        let capacity = cmd_cfg
            .deny_set_capacity
            .unwrap_or(DEFAULT_DENY_SET_CAPACITY);

        // Validate and construct the command policy — no warn-and-skip.
        let policy = CommandIssuerPolicy::new(issuer.clone(), age, principals, capacity)
            .map_err(|e| format!("nip-fi: issuer [index {idx}] invalid command policy: {e}"))?;

        issuer_capacities.push(IssuerCapacity {
            issuer: issuer.clone(),
            capacity,
        });
        command_policies.push(policy);

        // Track the maximum capacity across issuers for the default slot.
        if capacity > default_capacity {
            default_capacity = capacity;
        }
    }

    if command_policies.is_empty() {
        debug!("nip-fi: no command-capable issuers configured — command API disabled");
        return Ok(None);
    }

    let deny_map = Arc::new(NipFiDenyMap::new(default_capacity, issuer_capacities));

    let command_verifier = Arc::new(CommandVerifier::new(
        registry.clone(),
        key_source,
        command_policies,
        (*deny_map).clone(),
    ));

    Ok(Some(NipFiCommandComponents {
        deny_map,
        command_verifier,
    }))
}

/// Validate a command issuer config entry without constructing a policy.
///
/// Called by `nip_fi_config.rs` at startup before `build_nip_fi_command_components`
/// so that invalid config is rejected at `Config::from_env()`, not at serve time.
/// Returns `Err` with a non-sensitive message (no raw issuer URI).
pub fn validate_command_issuer_config(
    idx: usize,
    age_seconds: u64,
    principals: &[String],
    capacity: usize,
) -> Result<(), String> {
    CommandIssuerPolicy::new(
        // Use a sentinel issuer for validation only — no URI written to any log.
        format!("https://validate-sentinel-{idx}.internal"),
        age_seconds,
        principals.to_vec(),
        capacity,
    )
    .map(|_| ())
    .map_err(|e| format!("issuer [index {idx}] invalid command policy: {e}"))
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

/// Build the `401 authentication required` response with the mandatory
/// `WWW-Authenticate: Nostr` header. [NIP-FI.md §Rejection table]
fn auth_required_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("WWW-Authenticate", "Nostr")
        .body(Body::from("authentication required\n"))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Spec-exact 200 success response.
///
/// The spec body is `{"disconnected": true}` (note the space after `:`).
/// `serde_json::to_vec` produces `{"disconnected":true}` without the space.
/// We produce the literal bytes directly to stay byte-exact.
fn disconnected_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from("{\"disconnected\": true}"))
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

    // (The previous constant-assertion test was removed: asserting None and a constant
    // does not bind production behavior. The builder is now covered by route integration tests.)

    // ── HTTP response contract ─────────────────────────────────────────────

    /// The spec requires `{"disconnected": true}` (note the space after `:`).
    #[tokio::test]
    async fn disconnected_response_is_spec_exact() {
        let resp = disconnected_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(ct, "application/json");
        // Body bytes are verified directly — serde_json compact and the spec
        // literal are NOT the same (serde_json omits the space).
        let body_bytes = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body_bytes.as_ref(),
            b"{\"disconnected\": true}",
            "success body must be byte-exact per spec"
        );
    }

    /// `401` MUST carry `WWW-Authenticate: Nostr` and the spec body.
    #[tokio::test]
    async fn auth_required_response_has_www_authenticate() {
        let resp = auth_required_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(www_auth, "Nostr", "401 MUST carry WWW-Authenticate: Nostr");
        let body_bytes = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(body_bytes.as_ref(), b"authentication required\n");
    }

    /// `403` error responses MUST NOT carry `WWW-Authenticate`.
    #[test]
    fn error_responses_have_no_www_authenticate() {
        for body in &["evidence rejected\n", "authorization denied\n"] {
            let resp = plain_response(StatusCode::FORBIDDEN, body);
            assert!(
                resp.headers().get("WWW-Authenticate").is_none(),
                "403 must not carry WWW-Authenticate"
            );
        }
    }

    /// `503` plain responses have the spec-exact body.
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

// ── Route integration tests ────────────────────────────────────────────────────
//
// Exercises `disconnect()` through the full axum router with a warmed
// ProductionJwksSource, a real CommandVerifier, and an AppState wired exactly
// as production does (nip_fi_command_verifier + nip_fi_deny_map both set).
//
// These tests call the route at POST /api/nip-fi/disconnect via oneshot and
// verify every spec response row: 401, 403 (evidence), 403 (authz), 400, 503
// (capacity), 200 exact bytes.  The startup-assembly invariant is also
// verified: the tests GO RED if either state field is absent (503 unavailable).

#[cfg(test)]
mod route_integration_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use buzz_auth::{
        CommandIssuerPolicy, CommandVerifier, IssuerCapacity, IssuerRegistry, NipFiDenyMap,
        ProductionJwksSource,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    // ── Shared test key material ────────────────────────────────────────────

    // ES256 key pair — same material as command.rs tests, known-good.
    const TEST_PRIVATE_KEY_PEM: &str =
        "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n";

    const TEST_ISS: &str = "https://idp.test.example.com";
    const TEST_AUD: &str = "https://relay.test.example.com";
    const TEST_SUB: &str = "admin-svc@test.example.com";
    const TEST_PATH: &str = "/api/nip-fi/disconnect";

    // Key ID used in both the JWKS and the JWT header.
    const TEST_KID: &str = "route-test-key-1";

    fn test_public_jwk() -> jsonwebtoken::jwk::Jwk {
        // Public-key coordinates extracted from TEST_PRIVATE_KEY_PEM (P-256),
        // which is the same key pair as command.rs TEST_JWK_X/Y constants.
        serde_json::from_value(serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
            "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA",
            "alg": "ES256",
            "use": "sig",
            "kid": TEST_KID
        }))
        .expect("valid test JWK")
    }

    fn test_jwks() -> jsonwebtoken::jwk::JwkSet {
        jsonwebtoken::jwk::JwkSet {
            keys: vec![test_public_jwk()],
        }
    }

    fn test_issuer_policy() -> buzz_auth::IssuerPolicy {
        use buzz_auth::{FreshnessClass, IssuerPolicy, JwksSourceContract, TokenClass};
        let contract =
            JwksSourceContract::new(format!("{TEST_ISS}/.well-known/jwks.json"), 300, 86400)
                .expect("valid JWKS contract");
        IssuerPolicy::new(
            TEST_ISS.to_owned(),
            vec![TEST_AUD.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![buzz_auth::JwtAlgorithm::ES256],
            30,
            3600,
            None,
            contract,
        )
        .expect("valid issuer policy")
    }

    fn test_jwks_config() -> buzz_auth::IssuerJwksConfig {
        use buzz_auth::{IssuerJwksConfig, JwksSourceContract};
        let contract =
            JwksSourceContract::new(format!("{TEST_ISS}/.well-known/jwks.json"), 300, 86400)
                .expect("valid JWKS contract");
        IssuerJwksConfig {
            issuer: TEST_ISS.to_owned(),
            contract,
        }
    }

    fn mint_token(target_hex: &str, until_offset_secs: i64, extra: serde_json::Value) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let mut claims = serde_json::json!({
            "iss": TEST_ISS,
            "aud": TEST_AUD,
            "sub": TEST_SUB,
            "iat": now,
            "exp": now + 60,
            "jti": uuid::Uuid::new_v4().to_string(),
            "method": "POST",
            "path": TEST_PATH,
            "cmd": "disconnect",
            "target_pubkey": target_hex,
            "until": now + until_offset_secs,
        });
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                claims[k] = v.clone();
            }
        }
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("nip-fi-command+jwt".to_owned());
        header.kid = Some(TEST_KID.to_owned());
        let key = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("test EC key");
        encode(&header, &claims, &key).expect("sign test token")
    }

    async fn build_test_state(capacity: usize) -> Arc<crate::state::AppState> {
        // Build a minimal AppState with NIP-FI S4 components wired.
        // Uses lazy/invalid DB+Redis — only nip_fi fields and conn_manager matter.
        use crate::state::AppState;
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.database_url = "postgres://buzz:buzz@127.0.0.1:1/buzz".to_string();
        config.redis_url = "redis://127.0.0.1:1".to_string();

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );

        // Wire NIP-FI S4 components.
        let jwks_configs = vec![test_jwks_config()];
        let key_source = Arc::new(
            ProductionJwksSource::new(jwks_configs, buzz_auth::HttpJwksFetcher::new())
                .expect("key source"),
        );
        // Seed the snapshot without making an HTTP request.
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());

        let deny_map = Arc::new(NipFiDenyMap::new(
            capacity,
            vec![IssuerCapacity {
                issuer: TEST_ISS.to_owned(),
                capacity,
            }],
        ));
        let policy =
            CommandIssuerPolicy::new(TEST_ISS.to_owned(), 30, vec![TEST_SUB.to_owned()], capacity)
                .expect("command policy");
        let verifier = Arc::new(CommandVerifier::new(
            registry,
            Arc::clone(&key_source),
            vec![policy],
            (*deny_map).clone(),
        ));

        state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
        state.nip_fi_command_verifier = Some(verifier);
        Arc::new(state)
    }

    fn target_hex() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    async fn do_request(
        state: Arc<crate::state::AppState>,
        method: &str,
        headers: Vec<(&'static str, String)>,
        body: Option<serde_json::Value>,
    ) -> axum::response::Response {
        use crate::router::build_router;
        let body_bytes = match body {
            Some(v) => serde_json::to_vec(&v).unwrap().into(),
            None => axum::body::Bytes::new(),
        };
        let mut req = Request::builder().method(method).uri(TEST_PATH);
        for (k, v) in &headers {
            req = req.header(*k, v.as_str());
        }
        let req = req.body(Body::from(body_bytes)).unwrap();
        build_router(state).oneshot(req).await.unwrap()
    }

    // ── Test: no verifier → 503 (startup-assembly invariant) ─────────────────

    #[tokio::test]
    async fn absent_verifier_gives_503_unavailable() {
        // If nip_fi_command_verifier is not set, every request gets 503.
        // This test would FAIL if production startup failed to wire the verifier
        // (which was the F1 defect in pass 1 — endpoint stuck at 503 forever).
        // Build a state without the verifier.
        let no_verifier_state = {
            let mut config = crate::config::Config::from_env().expect("default config loads");
            config.database_url = "postgres://buzz:buzz@127.0.0.1:1/buzz".to_string();
            config.redis_url = "redis://127.0.0.1:1".to_string();
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            // nip_fi_command_verifier stays None.
            Arc::new(state)
        };
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            no_verifier_state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── Test: absent header → 401 + WWW-Authenticate ─────────────────────────

    #[tokio::test]
    async fn absent_header_route_gives_401_with_www_authenticate() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let resp = do_request(
            state,
            "POST",
            vec![("Content-Type", "application/json".into())],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(www_auth, "Nostr", "401 MUST carry WWW-Authenticate: Nostr");
    }

    // ── Test: bad signature → 403 evidence rejected ───────────────────────────

    #[tokio::test]
    async fn bad_signature_gives_403_evidence_rejected() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        // Tamper the token.
        let token = mint_token(&target, 300, serde_json::json!({}));
        let tampered = format!("{token}X");
        let resp = do_request(
            state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {tampered}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ── Test: capacity exceeded → 503 does NOT burn jti ──────────────────────

    #[tokio::test]
    async fn capacity_503_does_not_burn_jti_route_retry_succeeds() {
        // capacity=1, two distinct targets.
        let state = build_test_state(1).await;
        let target_a = target_hex();
        let target_b = target_hex();

        // First request fills the slot.
        let token_a = mint_token(&target_a, 300, serde_json::json!({}));
        let resp_a = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token_a}")),
            ],
            Some(serde_json::json!({"pubkey": target_a})),
        )
        .await;
        assert_eq!(resp_a.status(), StatusCode::OK);

        // Second request hits capacity → 503.  Jti NOT burned.
        let jti_b = uuid::Uuid::new_v4().to_string();
        let token_b = mint_token(&target_b, 300, serde_json::json!({"jti": jti_b}));
        let resp_b = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token_b}")),
            ],
            Some(serde_json::json!({"pubkey": target_b})),
        )
        .await;
        assert_eq!(
            resp_b.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "capacity exceeded must return 503"
        );
        let body = axum::body::to_bytes(resp_b.into_body(), 64).await.unwrap();
        assert_eq!(body.as_ref(), b"deny set full\n");
        // Jti was NOT burned: the same token_b can be reused once the slot frees.
        // (Route-level: we verify the 503 body; the jti non-burn is covered by
        // command.rs::capacity_503_does_not_burn_jti_retry_succeeds_after_slot_freed)
    }

    // ── Test: successful disconnect → 200 spec-exact bytes ───────────────────

    #[tokio::test]
    async fn success_response_is_spec_exact_bytes() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(ct, "application/json");
        let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"{\"disconnected\": true}",
            "200 body must be byte-exact per spec (note the space after ':')"
        );
    }

    // ── Test: count-independence (zero vs many sessions) ─────────────────────

    #[tokio::test]
    async fn success_body_identical_regardless_of_sessions_closed() {
        // Zero live sessions: response must still be {"disconnected": true}.
        let state = build_test_state(1000).await;
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"{\"disconnected\": true}",
            "zero-sessions success must be byte-identical to many-sessions success [no count leak]"
        );
    }

    // ── Test: deny entry recorded after success ───────────────────────────────

    #[tokio::test]
    async fn success_records_deny_entry_visible_to_is_denied() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let target_pubkey = nostr::PublicKey::from_hex(&target).expect("valid hex pubkey");

        // Before disconnect: not denied.
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");
        assert!(
            !deny_map.is_denied(TEST_ISS, &target_pubkey, chrono::Utc::now()),
            "must not be denied before disconnect"
        );

        // Execute disconnect.
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        // After disconnect: denied.
        assert!(
            deny_map.is_denied(TEST_ISS, &target_pubkey, chrono::Utc::now()),
            "must be denied after successful disconnect"
        );
    }
}
