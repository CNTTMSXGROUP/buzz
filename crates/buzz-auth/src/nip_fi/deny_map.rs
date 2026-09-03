//! In-memory NIP-FI deny set.
//!
//! Holds `(iss, pubkey) → until` entries.  No persistence — a relay restart
//! forgets active entries (Option B, as decided).  The issuer re-push path is
//! documented as the mitigation but is not implemented here.
//!
//! ## Invariants
//!
//! * **Merge rule**: inserting a new `until` for an existing key retains
//!   `max(existing_until, new_until)` — an accepted disconnect MUST NOT shorten
//!   an active deny. [FI-TRACE-DENY-SET]
//! * **Past-`until` commands**: close sessions but MUST NOT create or shorten
//!   entries.  Concretely, when `new_until < now` the merge still applies the
//!   `max` rule, which preserves any active entry and lets a "no active entry"
//!   case insert with an already-expired value (immediately inactive). [FI-TRACE-DENY-SET]
//! * **Per-issuer capacity cap**: each issuer has a hard ceiling on live entries.
//!   A capacity failure returns `Err(DenySetFull)` without inserting anything —
//!   the spec requires `503` here and neither the jti nor the deny entry is
//!   recorded. [FI-TRACE-DENY-SET]
//! * **Cross-issuer isolation**: capacity of issuer A MUST NOT affect issuer B.
//! * **jti reservation** and **deny-entry insertion** are performed atomically
//!   in one lock scope (both or neither). [VerifyCommandJwt step 7]
//! * **Issuer-global scope**: the deny applies across all communities served
//!   under that issuer. [FI-TRACE-DENY-SET]
//! * **Self-eviction**: expired entries are pruned lazily on each mutation and
//!   on read (deny check), so the map does not grow without bound.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use nostr::PublicKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Error type ────────────────────────────────────────────────────────────────

/// Returned when the per-issuer deny-set capacity is exhausted.
///
/// The caller MUST respond `503` and MUST NOT record the jti; the same signed
/// command remains replayable (the command identity was not consumed).
/// [VerifyCommandJwt step 7]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("deny set full for issuer")]
pub struct DenySetFull;

// ── Per-issuer shard ──────────────────────────────────────────────────────────

/// One issuer's worth of deny entries and jti deduplication state.
///
/// The shard mutex is acquired once per `AtomicReserveJtiAndDenyEntry` call
/// so both mutations happen under the same lock (both-or-neither atomicity).
struct IssuerShard {
    /// Active deny entries: hex-encoded pubkey → until.
    entries: HashMap<String, DateTime<Utc>>,
    /// Reserved jtis: jti string → effective_expiry.  Expired jtis are evicted
    /// lazily on each write so the map never grows to replay-corpus size.
    jtis: HashMap<String, DateTime<Utc>>,
    /// Maximum number of live entries for this issuer.
    capacity: usize,
}

impl IssuerShard {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            jtis: HashMap::new(),
            capacity,
        }
    }

    /// Evict expired entries and jtis.  Called inside the lock on every write.
    fn evict_expired(&mut self, now: DateTime<Utc>) {
        self.entries.retain(|_, until| *until > now);
        self.jtis.retain(|_, exp| *exp > now);
    }

    /// True if `(iss, pubkey_hex)` has an active deny entry (`now < until`).
    fn is_denied(&self, pubkey_hex: &str, now: DateTime<Utc>) -> bool {
        self.entries
            .get(pubkey_hex)
            .map(|until| now < *until)
            .unwrap_or(false)
    }

    /// Attempt the atomic jti-reservation + deny-entry insertion.
    ///
    /// **Atomicity**: both HashMap inserts are precomputed before any write.
    /// Eviction is done first (pure mutation of existing map, always safe),
    /// then all fallible pre-conditions are checked, then both inserts happen
    /// under the same lock scope.  An unwind before the inserts leaves the
    /// shard unchanged; an unwind mid-insert is not possible because HashMap
    /// insert is infallible after capacity reservation.
    fn atomic_reserve_and_insert(
        &mut self,
        jti: &str,
        jti_effective_expiry: DateTime<Utc>,
        pubkey_hex: &str,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReserveError> {
        self.evict_expired(now);

        // Replay check: jti already in set → AuthorizationDenied.
        if self.jtis.contains_key(jti) {
            return Err(ReserveError::JtiAlreadyReserved);
        }

        // Capacity check: only count an insert if this pubkey has no active
        // entry already.  The merge rule never increases live entry count.
        let is_update = self
            .entries
            .get(pubkey_hex)
            .map(|existing| now < *existing)
            .unwrap_or(false);
        if !is_update && self.entries.len() >= self.capacity {
            return Err(ReserveError::CapacityExceeded);
        }

        // Prebuild both values before writing anything.
        let jti_key = jti.to_owned();
        let entry_key = pubkey_hex.to_owned();
        let effective_until = match self.entries.get(pubkey_hex) {
            Some(&existing) => existing.max(until),
            None => until,
        };

        // Both mutations are infallible HashMap inserts; executed together
        // so no intermediate observable state exists.
        self.jtis.insert(jti_key, jti_effective_expiry);
        self.entries.insert(entry_key, effective_until);

        Ok(())
    }

    /// Merge a remote deny entry without consuming a jti.
    ///
    /// Used for cross-pod propagation where replay idempotency is achieved by
    /// the max(until) merge rule alone — no jti tracking needed.
    /// Returns `Err(CapacityExceeded)` if the entry is new and the shard is full.
    fn remote_merge(
        &mut self,
        pubkey_hex: &str,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReserveError> {
        self.evict_expired(now);

        // Capacity check: only count as new if there is no active entry.
        let is_update = self
            .entries
            .get(pubkey_hex)
            .map(|existing| now < *existing)
            .unwrap_or(false);
        if !is_update && self.entries.len() >= self.capacity {
            return Err(ReserveError::CapacityExceeded);
        }

        // max(existing_until, until) merge.
        let effective_until = match self.entries.get(pubkey_hex) {
            Some(&existing) => existing.max(until),
            None => until,
        };
        self.entries.insert(pubkey_hex.to_owned(), effective_until);
        Ok(())
    }
}

/// Reasons an atomic reserve can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReserveError {
    /// The jti was already reserved — replay attempt.
    JtiAlreadyReserved,
    /// Per-issuer capacity ceiling reached.
    CapacityExceeded,
}

/// Outcome of a cross-pod deny merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossPodMergeResult {
    /// Entry was inserted or updated (max-merge applied).
    Merged,
    /// Issuer is not locally configured; message rejected.
    UnknownIssuer,
    /// Per-issuer capacity ceiling reached; issuer is fail-closed.
    CapacityExceeded,
    /// Shard mutex is poisoned; issuer is fail-closed.
    ShardPoisoned,
}

// ── Public map ────────────────────────────────────────────────────────────────

/// Relay-wide in-memory NIP-FI deny set.
///
/// One `Arc<NipFiDenyMap>` is held in `AppState`; the HTTP disconnect endpoint
/// and the WS admission check share it.
///
/// The deny-check interface is intentionally transport-agnostic — S5 (HTTP
/// enforcement) calls `is_denied` from HTTP admission without any WS coupling.
#[derive(Clone)]
pub struct NipFiDenyMap {
    /// Per-issuer shards.  Each shard owns its own Mutex so cross-issuer
    /// capacity exhaustion is impossible to cause cross-issuer denial.
    shards: Arc<DashMap<String, Mutex<IssuerShard>>>,
    /// Default per-issuer capacity, used when no issuer-specific override exists.
    default_capacity: usize,
}

/// A per-issuer capacity override supplied at construction time.
#[derive(Debug, Clone)]
pub struct IssuerCapacity {
    /// The exact issuer URI this capacity applies to.
    pub issuer: String,
    /// Maximum number of live deny entries for this issuer.
    pub capacity: usize,
}

impl NipFiDenyMap {
    /// Construct a new deny map.
    ///
    /// `default_capacity` is the per-issuer entry ceiling used for any issuer
    /// not listed in `issuer_capacities`.  Must be > 0.
    ///
    /// A zero capacity would make every command a 503; callers must validate
    /// before construction.
    pub fn new(default_capacity: usize, issuer_capacities: Vec<IssuerCapacity>) -> Self {
        let shards: DashMap<String, Mutex<IssuerShard>> = DashMap::new();
        for ic in issuer_capacities {
            shards.insert(ic.issuer, Mutex::new(IssuerShard::new(ic.capacity)));
        }
        Self {
            shards: Arc::new(shards),
            default_capacity,
        }
    }

    /// Returns `true` when `(iss, pubkey)` has an active deny entry at `now`.
    ///
    /// Used by S4 (WS admission step 6) and S5 (HTTP admission step 5).
    /// [FI-TRACE-DENY-SET]
    ///
    /// Fails **closed**: a poisoned shard lock returns `true` (deny) so that a
    /// damaged shard cannot silently admit a denied pubkey.
    pub fn is_denied(&self, issuer: &str, pubkey: &PublicKey, now: DateTime<Utc>) -> bool {
        let pubkey_hex = pubkey.to_hex();
        match self.shards.get(issuer) {
            Some(shard) => shard
                .lock()
                .map(|guard| guard.is_denied(&pubkey_hex, now))
                .unwrap_or(true), // poisoned shard → fail closed (deny)
            None => false,
        }
    }

    /// Atomically reserve `(iss, jti)` and insert/merge the deny entry.
    ///
    /// Both mutations happen under the same per-issuer lock (both-or-neither).
    ///
    /// * `Ok(())` — success.
    /// * `Err(ReserveError::JtiAlreadyReserved)` — replay; map is unchanged,
    ///   caller responds `AuthorizationDenied`.
    /// * `Err(ReserveError::CapacityExceeded)` — full; map is unchanged,
    ///   caller responds `503 deny set full`.
    ///
    /// [VerifyCommandJwt step 7]
    pub(crate) fn atomic_reserve_and_insert(
        &self,
        issuer: &str,
        jti: &str,
        jti_effective_expiry: DateTime<Utc>,
        pubkey: &PublicKey,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReserveError> {
        let pubkey_hex = pubkey.to_hex();
        let shard = self
            .shards
            .entry(issuer.to_owned())
            .or_insert_with(|| Mutex::new(IssuerShard::new(self.default_capacity)));
        shard
            .lock()
            .map_err(|_| ReserveError::CapacityExceeded) // poisoned = fail closed
            .and_then(|mut guard| {
                guard.atomic_reserve_and_insert(jti, jti_effective_expiry, &pubkey_hex, until, now)
            })
    }

    /// Merge a cross-pod deny entry (e.g. from Redis propagation).
    ///
    /// Idempotent: repeated delivery of the same `(issuer, pubkey, until)` is
    /// a no-op due to the `max(until)` merge rule.  No synthetic jti is
    /// allocated — replay idempotency is structural, not tracked.
    ///
    /// Only merges into **locally-configured** issuer shards.  An unknown
    /// issuer returns [`CrossPodMergeResult::UnknownIssuer`] so the consumer
    /// can reject without allocating state.
    ///
    /// Capacity exhaustion and shard poisoning both return fail-closed results
    /// so the caller can transition the issuer to a deny-all posture.
    pub fn merge_cross_pod_deny(
        &self,
        issuer: &str,
        pubkey: &PublicKey,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> CrossPodMergeResult {
        let pubkey_hex = pubkey.to_hex();
        // Only operate on pre-configured shards — never allocate for unknown issuers.
        match self.shards.get(issuer) {
            None => CrossPodMergeResult::UnknownIssuer,
            Some(shard) => match shard.lock() {
                Err(_) => CrossPodMergeResult::ShardPoisoned,
                Ok(mut guard) => match guard.remote_merge(&pubkey_hex, until, now) {
                    Ok(()) => CrossPodMergeResult::Merged,
                    Err(ReserveError::CapacityExceeded) => CrossPodMergeResult::CapacityExceeded,
                    Err(ReserveError::JtiAlreadyReserved) => {
                        // remote_merge never touches jtis; this arm is unreachable.
                        unreachable!("remote_merge does not use jti tracking")
                    }
                },
            },
        }
    }

    /// Close all sessions whose proven pubkey is `pubkey` for any issuer and
    /// any community.  This is the issuer-global close scan.
    ///
    /// Returns the pubkey_hex for downstream use.
    pub fn pubkey_hex(pubkey: &PublicKey) -> String {
        pubkey.to_hex()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use nostr::Keys;

    fn key() -> PublicKey {
        Keys::generate().public_key()
    }

    fn map() -> NipFiDenyMap {
        NipFiDenyMap::new(100, vec![])
    }

    fn iss() -> &'static str {
        "https://issuer.example.com"
    }

    // ── FI-TRACE-DENY-SET: basic admit/deny ──────────────────────────────────

    #[test]
    fn not_denied_when_no_entry() {
        let m = map();
        assert!(
            !m.is_denied(iss(), &key(), Utc::now()),
            "no entry → admitted"
        );
    }

    #[test]
    fn denied_when_active_entry() {
        let m = map();
        let k = key();
        let until = Utc::now() + Duration::seconds(300);
        m.atomic_reserve_and_insert(iss(), "jti-1", until, &k, until, Utc::now())
            .expect("first insert");
        assert!(m.is_denied(iss(), &k, Utc::now()), "active entry → denied");
    }

    #[test]
    fn admitted_after_until_expires() {
        let m = map();
        let k = key();
        let until = Utc::now() - Duration::seconds(1); // already expired
        m.atomic_reserve_and_insert(iss(), "jti-exp", until, &k, until, Utc::now())
            .expect("insert with past-until");
        // is_denied with `now` past `until` → not denied
        assert!(
            !m.is_denied(iss(), &k, Utc::now()),
            "expired entry → admitted"
        );
    }

    // ── FI-TRACE-DENY-SET: merge rule ────────────────────────────────────────

    #[test]
    fn merge_rule_longer_command_wins() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let longer = now + Duration::seconds(600);
        let shorter = now + Duration::seconds(300);

        // Insert longer first.
        m.atomic_reserve_and_insert(iss(), "jti-A", longer, &k, longer, now)
            .expect("insert longer");
        // Insert shorter — must not shorten.
        m.atomic_reserve_and_insert(iss(), "jti-B", shorter, &k, shorter, now)
            .expect("insert shorter");

        // Check just before shorter would expire (still in longer window).
        let check_time = now + Duration::seconds(400);
        assert!(
            m.is_denied(iss(), &k, check_time),
            "merge rule: longer deny survives shorter command"
        );
    }

    #[test]
    fn merge_rule_longer_command_second_wins() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let shorter = now + Duration::seconds(300);
        let longer = now + Duration::seconds(600);

        // Insert shorter first, then longer.
        m.atomic_reserve_and_insert(iss(), "jti-A", shorter, &k, shorter, now)
            .expect("insert shorter");
        m.atomic_reserve_and_insert(iss(), "jti-B", longer, &k, longer, now)
            .expect("insert longer");

        let check_time = now + Duration::seconds(400);
        assert!(
            m.is_denied(iss(), &k, check_time),
            "delivery order does not matter — longer wins regardless"
        );
    }

    #[test]
    fn past_until_command_over_active_entry_leaves_active_unchanged() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let active_until = now + Duration::seconds(600);
        let past_until = now - Duration::seconds(60);

        // Active entry first.
        m.atomic_reserve_and_insert(iss(), "jti-A", active_until, &k, active_until, now)
            .expect("insert active");

        // Past-until command: max(active_until, past_until) = active_until.
        m.atomic_reserve_and_insert(iss(), "jti-B", past_until, &k, past_until, now)
            .expect("past-until insert");

        // Active entry unchanged.
        let check_time = now + Duration::seconds(400);
        assert!(
            m.is_denied(iss(), &k, check_time),
            "past-until command must not shorten active deny"
        );
    }

    #[test]
    fn past_until_command_absent_entry_inserts_expired() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let past_until = now - Duration::seconds(60);

        // Past-until, no existing entry → insert with expired value → immediately inactive.
        m.atomic_reserve_and_insert(iss(), "jti-A", past_until, &k, past_until, now)
            .expect("past-until on absent entry");

        // Not denied (entry is immediately expired).
        assert!(
            !m.is_denied(iss(), &k, now),
            "past-until with no prior entry creates no future denial"
        );
    }

    // ── Replay prevention ────────────────────────────────────────────────────

    #[test]
    fn jti_replay_is_rejected() {
        let m = map();
        let k = key();
        let until = Utc::now() + Duration::seconds(300);

        m.atomic_reserve_and_insert(iss(), "jti-same", until, &k, until, Utc::now())
            .expect("first use");
        let result = m.atomic_reserve_and_insert(iss(), "jti-same", until, &k, until, Utc::now());
        assert_eq!(
            result,
            Err(ReserveError::JtiAlreadyReserved),
            "replayed jti must be rejected"
        );
    }

    // ── Capacity ─────────────────────────────────────────────────────────────

    #[test]
    fn capacity_exceeded_returns_error_without_inserting() {
        // Capacity = 2, three distinct pubkeys.
        let m = NipFiDenyMap::new(2, vec![]);
        let now = Utc::now();
        let until = now + Duration::seconds(300);

        let k1 = key();
        let k2 = key();
        let k3 = key();

        m.atomic_reserve_and_insert(iss(), "jti-1", until, &k1, until, now)
            .expect("k1");
        m.atomic_reserve_and_insert(iss(), "jti-2", until, &k2, until, now)
            .expect("k2");
        let result = m.atomic_reserve_and_insert(iss(), "jti-3", until, &k3, until, now);
        assert_eq!(
            result,
            Err(ReserveError::CapacityExceeded),
            "third distinct key must be rejected when cap=2"
        );
        // k3 is NOT denied (entry was not inserted).
        assert!(!m.is_denied(iss(), &k3, now), "k3 must not be denied");
    }

    #[test]
    fn update_to_existing_key_does_not_count_against_capacity() {
        let m = NipFiDenyMap::new(1, vec![]);
        let now = Utc::now();
        let k = key();
        let until_a = now + Duration::seconds(300);
        let until_b = now + Duration::seconds(600);

        m.atomic_reserve_and_insert(iss(), "jti-a", until_a, &k, until_a, now)
            .expect("first insert");
        // Same key, longer until — should succeed even though capacity=1.
        m.atomic_reserve_and_insert(iss(), "jti-b", until_b, &k, until_b, now)
            .expect("update same key at capacity");

        assert!(
            m.is_denied(iss(), &k, now + Duration::seconds(400)),
            "updated entry is active"
        );
    }

    #[test]
    fn cross_issuer_capacity_is_independent() {
        let iss_a = "https://a.example.com";
        let iss_b = "https://b.example.com";
        // issuer A has capacity 1
        let m = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: iss_a.to_owned(),
                capacity: 1,
            }],
        );

        let now = Utc::now();
        let until = now + Duration::seconds(300);
        let k1 = key();
        let k2 = key();
        let k3 = key();

        // Fill issuer A.
        m.atomic_reserve_and_insert(iss_a, "jti-a1", until, &k1, until, now)
            .expect("iss_a k1");
        // Issuer B is at default capacity (100) → must accept.
        m.atomic_reserve_and_insert(iss_b, "jti-b1", until, &k2, until, now)
            .expect("iss_b k2 must succeed independent of iss_a capacity");
        // Issuer A is at capacity 1 → must reject.
        let result = m.atomic_reserve_and_insert(iss_a, "jti-a2", until, &k3, until, now);
        assert_eq!(
            result,
            Err(ReserveError::CapacityExceeded),
            "iss_a capacity exhaustion must not affect iss_b, and vice versa"
        );
    }

    // ── Poison-path: is_denied must fail closed ───────────────────────────────

    #[test]
    fn poisoned_shard_is_denied_fails_closed() {
        let iss = "https://poison.example.com";
        // Construct the map with a pre-registered shard for this issuer.
        let m = std::sync::Arc::new(NipFiDenyMap::new(
            10,
            vec![IssuerCapacity {
                issuer: iss.to_owned(),
                capacity: 10,
            }],
        ));
        let m_clone = std::sync::Arc::clone(&m);
        let k = key();

        // Poison the real IssuerShard by spawning a thread that acquires the
        // shard Mutex (which wraps a real IssuerShard) and then panics.
        // A thread panic while holding a Mutex guard poisons the mutex.
        let _ = std::thread::spawn(move || {
            let shard_ref = m_clone.shards.get(iss).expect("shard must exist");
            let _guard = shard_ref.lock().expect("lock acquired");
            panic!("intentional poison");
        })
        .join(); // Err(_) expected — that's the proof the thread panicked.

        // The shard is now poisoned.  is_denied must return true (fail closed).
        // Mutation anchor: reverting unwrap_or(true) → unwrap_or(false) makes
        // this assertion fail — that is the defect Thufir identified in pass 1.
        assert!(
            m.is_denied(iss, &k, Utc::now()),
            "poisoned shard must return true from is_denied (fail closed)"
        );

        // Confirm the normal path still works on a clean map.
        let clean = std::sync::Arc::new(NipFiDenyMap::new(
            10,
            vec![IssuerCapacity {
                issuer: iss.to_owned(),
                capacity: 10,
            }],
        ));
        let k2 = key();
        let until2 = Utc::now() + Duration::seconds(300);
        clean
            .atomic_reserve_and_insert(iss, "jti-clean", until2, &k2, until2, Utc::now())
            .expect("insert on clean map");
        assert!(
            clean.is_denied(iss, &k2, Utc::now()),
            "active entry on clean map must return true"
        );
    }

    // ── remote_merge: idempotent cross-pod semantics ─────────────────────────

    #[test]
    fn remote_merge_shorter_after_longer_does_not_shorten() {
        // Map with iss() pre-registered so remote_merge can operate on it.
        let m = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 100,
            }],
        );
        let k = key();
        let now = Utc::now();
        let longer = now + Duration::seconds(600);
        let shorter = now + Duration::seconds(300);

        // First merge: longer.
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, longer, now),
            CrossPodMergeResult::Merged
        );
        // Second merge: shorter — must not shorten.
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, shorter, now),
            CrossPodMergeResult::Merged
        );
        // At 400s: still denied (longer wins).
        assert!(
            m.is_denied(iss(), &k, now + Duration::seconds(400)),
            "shorter-after-longer remote merge must not shorten the deny"
        );
    }

    #[test]
    fn remote_merge_replay_is_idempotent() {
        // Map with iss() pre-registered so remote_merge can operate on it.
        let m = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 100,
            }],
        );
        let k = key();
        let now = Utc::now();
        let until = now + Duration::seconds(300);

        // Deliver twice.
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, until, now),
            CrossPodMergeResult::Merged
        );
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, until, now),
            CrossPodMergeResult::Merged
        );
        // Still denied at 200s (no spurious second-insert count growth).
        assert!(
            m.is_denied(iss(), &k, now + Duration::seconds(200)),
            "replay must be idempotent"
        );
    }

    #[test]
    fn remote_merge_unknown_issuer_rejected() {
        let m = map(); // default issuer is "https://issuer.example.com", not "unknown"
        let k = key();
        let until = Utc::now() + Duration::seconds(300);
        assert_eq!(
            m.merge_cross_pod_deny("https://unknown.example.com", &k, until, Utc::now()),
            CrossPodMergeResult::UnknownIssuer,
            "unknown issuer must be rejected without allocating state"
        );
        // No shard was created for the unknown issuer.
        assert!(
            m.shards.get("https://unknown.example.com").is_none(),
            "no shard must be allocated for unknown issuer"
        );
    }

    #[test]
    fn remote_merge_capacity_exceeded_returns_correct_result() {
        // Capacity = 1, two distinct keys.
        let m = NipFiDenyMap::new(
            1,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 1,
            }],
        );
        let now = Utc::now();
        let until = now + Duration::seconds(300);
        let k1 = key();
        let k2 = key();

        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k1, until, now),
            CrossPodMergeResult::Merged
        );
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k2, until, now),
            CrossPodMergeResult::CapacityExceeded,
            "second key with cap=1 must return CapacityExceeded"
        );
        // k2 is NOT denied (entry was not inserted).
        assert!(
            !m.is_denied(iss(), &k2, now),
            "k2 must not be denied after CapacityExceeded"
        );
    }

    #[test]
    fn remote_merge_poisoned_shard_returns_shard_poisoned() {
        let iss = "https://poison-remote.example.com";
        let m = std::sync::Arc::new(NipFiDenyMap::new(
            10,
            vec![IssuerCapacity {
                issuer: iss.to_owned(),
                capacity: 10,
            }],
        ));
        let m_clone = std::sync::Arc::clone(&m);
        let k = key();
        let until = Utc::now() + Duration::seconds(300);

        // Poison the shard.
        let _ = std::thread::spawn(move || {
            let shard_ref = m_clone.shards.get(iss).expect("shard must exist");
            let _guard = shard_ref.lock().expect("lock acquired");
            panic!("intentional poison for remote_merge test");
        })
        .join();

        assert_eq!(
            m.merge_cross_pod_deny(iss, &k, until, Utc::now()),
            CrossPodMergeResult::ShardPoisoned,
            "poisoned shard must return ShardPoisoned"
        );
    }
}
