//! Test-only barriers for NIP-FI B2 witness tests.
//!
//! Each function is a named production hook that is inert in production
//! (`#[cfg(test)]` guards ensure zero-cost at runtime) but acts as a
//! deterministic barrier in tests. A test arms the gate, dispatches work,
//! waits for the arrived notification, fires expiry, then releases the gate.
//!
//! Pattern (same as `publish_test_hooks` in `side_effects.rs`):
//! - `arm(community)` → `(arrived_rx, release_notify)`
//! - Production code calls `before_X(community).await`
//! - Test awaits `arrived_rx.await` → knows production reached the hook
//! - Test fires expiry
//! - Test calls `release_notify.notify_one()` → production proceeds
//!
//! Only one gate per slot is supported at a time (static Mutex). Tests are
//! sequential per community; concurrent tests use different communities.

use buzz_core::CommunityId;
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, Notify};

struct Gate {
    community: CommunityId,
    arrived: oneshot::Sender<()>,
    release: Arc<Notify>,
}

macro_rules! make_hook {
    ($mod_name:ident, $fn_name:ident) => {
        pub(crate) mod $mod_name {
            use super::*;

            static GATE: Mutex<Option<Gate>> = Mutex::new(None);

            /// Arm a one-shot barrier for `community`.
            ///
            /// Returns `(arrived_rx, release)`. Await `arrived_rx` to know when
            /// the production code has reached this hook; call `release.notify_one()`
            /// to let it continue.
            pub(crate) fn arm(community: CommunityId) -> (oneshot::Receiver<()>, Arc<Notify>) {
                let (tx, rx) = oneshot::channel();
                let release = Arc::new(Notify::new());
                *GATE.lock().unwrap() = Some(Gate {
                    community,
                    arrived: tx,
                    release: release.clone(),
                });
                (rx, release)
            }

            pub(crate) async fn trigger(community: CommunityId) {
                let gate = {
                    let mut slot = GATE.lock().unwrap();
                    match slot.as_ref() {
                        Some(g) if g.community == community => slot.take(),
                        _ => None,
                    }
                };
                if let Some(g) = gate {
                    let _ = g.arrived.send(());
                    g.release.notified().await;
                }
            }
        }

        pub(crate) async fn $fn_name(community: CommunityId) {
            $mod_name::trigger(community).await;
        }
    };
}

make_hook!(auth_commit_hook, before_auth_commit);
make_hook!(event_ingest_hook, before_event_ingest);
make_hook!(req_registration_hook, before_req_registration);
make_hook!(count_query_hook, before_count_query);

// ── Audio B1 hooks ─────────────────────────────────────────────────────────
// `before_membership_check`: fires between NIP-42 pairing and the membership
// DB read inside `check_membership_for_admission`. Arms expiry here → proves
// that a cancellation before membership check produces zero DB side effects.
//
// `before_participant_commit`: fires between the 48101 insert and the
// `acquire_effect()` + `tx.commit()` inside `commit_participant_join`. Arms
// expiry here → proves that a cancellation before the permit acquisition
// rolls back the transaction and produces zero post-expiry 48101/membership
// writes.
make_hook!(audio_membership_check_hook, before_membership_check);
make_hook!(audio_participant_commit_hook, before_participant_commit);
