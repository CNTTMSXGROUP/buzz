use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_personas,
        retention::{mark_synced, open_retention_db},
        storage::managed_agents_base_dir,
        AgentDefinition,
    },
};

use super::pending::{prepare_persona_publication, PreparedPersonaPublication};

/// Test-only observer called immediately after the `managed_agents_store_lock`
/// is acquired in `publish_and_refresh_teams_at`'s refresh section. Tests use
/// this to assert `try_lock()` fails — proving the lock is held during the
/// synchronous refresh. Moving the lock acquisition to AFTER the refresh call
/// (recreating the TOCTOU race) causes `try_lock()` to succeed, turning the
/// probe test RED.
#[cfg(test)]
type RefreshLockObserver = Box<dyn Fn(&AppState) + Send>;
#[cfg(test)]
pub(crate) static REFRESH_LOCK_OBSERVER: std::sync::Mutex<Option<RefreshLockObserver>> =
    std::sync::Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersonaSharePublicationStatus {
    Published,
    Queued,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPersonaSharedResult {
    pub persona: AgentDefinition,
    pub publication_status: PersonaSharePublicationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_message: Option<String>,
}

#[tauri::command]
pub async fn set_persona_shared(
    id: String,
    shared: bool,
    app: AppHandle,
) -> Result<SetPersonaSharedResult, String> {
    let prepared = tokio::task::spawn_blocking({
        let app = app.clone();
        move || {
            let state = app.state::<AppState>();
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let personas = load_personas(&app)?;
            let persona = personas
                .iter()
                .find(|record| record.id == id)
                .ok_or_else(|| format!("agent {id} not found"))?;

            if persona.is_builtin {
                return Err("Built-in agents cannot be shared to the catalog.".to_string());
            }

            // Strict path: unlike ordinary definition saves, an enqueue failure
            // for this privacy-sensitive toggle must reach the command/UI.
            prepare_persona_publication(&app, &state, persona, Some(shared))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // Save persona id before `prepared` is consumed by publish_prepared_persona.
    let persona_id = prepared.persona.id.clone();
    let base_dir = managed_agents_base_dir(&app)?;
    let keys = prepared.scope.owner_keys.clone();
    let db_path = prepared.scope.db_path.clone();
    let state = app.state::<AppState>();
    publish_and_refresh_teams_at(&state, prepared, &base_dir, &keys, &db_path, &persona_id).await
}

/// Save a persona edit AND publish its catalog head, returning the same
/// `published | queued` outcome as [`set_persona_shared`].
///
/// The "save and publish" affordance in the edit dialog promises the change
/// reaches the catalog on save. Plain `update_persona` only enqueues
/// best-effort, so the UI could not report whether the relay accepted it. This
/// takes the identical input and reuses the strict preparation path, then awaits
/// the relay exactly like the share toggle does — a rejection or an unreachable
/// relay stays durably queued for the flush loop and is reported as `queued`.
#[tauri::command]
pub async fn update_persona_and_publish(
    input: crate::managed_agents::UpdatePersonaRequest,
    app: AppHandle,
) -> Result<SetPersonaSharedResult, String> {
    let (_, prepared) =
        super::update::update_persona_with(input, app.clone(), |app, state, persona| {
            // Strict path: this command's contract is to report the publication
            // outcome, so an enqueue failure must reach the UI rather than being
            // logged and swallowed.
            let result = prepare_persona_publication(app, state, persona, None)?;
            // F2: refresh any shared 30178 heads that include this persona.
            crate::commands::refresh_team_catalog_heads_for_persona(app, state, &persona.id);
            Ok(result)
        })
        .await?;

    let state = app.state::<AppState>();
    publish_prepared_persona(&state, prepared).await
}

/// Publish a prepared persona head and refresh any shared 30178 team heads that
/// include this persona — the combined contract shared by the share toggle and
/// the publish-retry seam.
///
/// Extracted from [`set_persona_shared`] so this two-step sequence can be
/// tested directly through `publish_and_refresh_teams_at` without a
/// `tauri::AppHandle`. Deleting the [`refresh_for_persona_at`] call from this
/// function must cause the command-path regression to fail.
pub(crate) async fn publish_and_refresh_teams_at(
    state: &AppState,
    prepared: PreparedPersonaPublication,
    base_dir: &std::path::Path,
    keys: &nostr::Keys,
    db_path: &std::path::Path,
    persona_id: &str,
) -> Result<SetPersonaSharedResult, String> {
    let result = publish_prepared_persona(state, prepared).await?;
    // F2: refresh any shared 30178 heads that include this persona. The refresh
    // reads the current team/persona definitions and may retain a new head — it
    // must be serialized with team edit/unshare/delete operations that also hold
    // `managed_agents_store_lock` and may retain/retract the same head.
    //
    // Without the lock a concurrent `set_team_shared(false)` can:
    //   1. acquire the lock, retain unshared head T+1, release the lock;
    //   2. refresh (unlocked) reads old shared head T, rebuilds, retains T+1;
    //      `retain_event` accepts equal timestamps — the refresh wins, undoing
    //      the explicit unshare.
    //
    // Acquire AFTER the network await so the lock is never held across I/O;
    // the synchronous refresh completes entirely inside the critical section.
    {
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| format!("managed_agents_store_lock poisoned: {e}"))?;
        #[cfg(test)]
        {
            if let Ok(obs) = REFRESH_LOCK_OBSERVER.lock() {
                if let Some(ref f) = *obs {
                    f(state);
                }
            }
        }
        let _ = crate::commands::teams::refresh_for_persona_at(base_dir, keys, db_path, persona_id);
    }
    Ok(result)
}

async fn publish_prepared_persona(
    state: &AppState,
    prepared: PreparedPersonaPublication,
) -> Result<SetPersonaSharedResult, String> {
    let api_base_url = crate::relay::relay_http_base_url(&prepared.scope.relay_url);
    let publish_result = crate::relay::submit_signed_event_at_with_keys(
        &prepared.event,
        state,
        &api_base_url,
        &prepared.scope.owner_keys,
    )
    .await;

    match publish_result {
        Ok(_) => {
            let conn = open_retention_db(&prepared.scope.db_path)?;
            mark_synced(
                &conn,
                prepared.retained.kind,
                &prepared.retained.pubkey,
                &prepared.retained.d_tag,
                prepared.retained.created_at,
                &prepared.retained.content,
            )?;
            Ok(SetPersonaSharedResult {
                persona: prepared.persona,
                publication_status: PersonaSharePublicationStatus::Published,
                relay_message: None,
            })
        }
        Err(error) => Ok(SetPersonaSharedResult {
            persona: prepared.persona,
            publication_status: PersonaSharePublicationStatus::Queued,
            relay_message: Some(error),
        }),
    }
}

#[cfg(all(test, not(target_os = "windows")))]
use super::update::update_persona_with as update_persona_with_seam;

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::{update_persona_with_seam, *};
    use crate::{
        app_state::build_app_state,
        commands::personas::pending::prepare_persona_publication_at,
        managed_agents::{
            retention::{get_retained_event, open_retention_db, RetentionScope},
            save_managed_agents, save_personas, AgentDefinition, ManagedAgentRecord,
            UpdatePersonaRequest,
        },
    };
    use std::collections::BTreeMap;
    fn persona() -> AgentDefinition {
        AgentDefinition {
            description: None,
            id: "catalog-reviewer".to_string(),
            display_name: "Catalog Reviewer".to_string(),
            avatar_url: None,
            system_prompt: "Review the catalog.".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-07-27T00:00:00Z".to_string(),
            updated_at: "2026-07-27T00:00:00Z".to_string(),
        }
    }

    async fn spawn_relay(accepted: bool) -> String {
        use axum::{routing::post, Router};

        let app = Router::new().route(
            "/events",
            post(move |body: String| async move {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                serde_json::json!({
                    "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                    "accepted": accepted,
                    "message": if accepted { "" } else { "policy rejection" }
                })
                .to_string()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{addr}")
    }

    fn prepared(
        db_path: &std::path::Path,
        relay_url: String,
        keys: nostr::Keys,
        shared_override: Option<bool>,
    ) -> PreparedPersonaPublication {
        let (event, retained, persona) =
            prepare_persona_publication_at(db_path, &keys, &persona(), shared_override).unwrap();
        PreparedPersonaPublication {
            scope: RetentionScope {
                db_path: db_path.to_path_buf(),
                relay_url,
                owner_keys: keys,
            },
            event,
            retained,
            persona,
        }
    }

    #[tokio::test]
    async fn relay_rejection_stays_durably_queued() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let prepared = prepared(&db_path, spawn_relay(false).await, keys, Some(true));
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Queued
        );
        assert!(result
            .relay_message
            .as_deref()
            .is_some_and(|message| message.contains("relay rejected event")));
        assert!(
            get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    #[tokio::test]
    async fn unavailable_relay_stays_durably_queued() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let prepared = prepared(&db_path, relay_url, keys, Some(true));
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Queued
        );
        assert!(result
            .relay_message
            .as_deref()
            .is_some_and(|message| message.starts_with("relay unreachable:")));
        assert!(
            get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    #[tokio::test]
    async fn relay_acceptance_marks_the_scoped_head_synced() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let prepared = prepared(&db_path, spawn_relay(true).await, keys, Some(true));
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Published
        );
        assert!(
            !get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    /// `update_persona_and_publish` differs from the share toggle in one way:
    /// it passes no share override, so the edit must keep whatever the scoped
    /// head already says, and it reports the relay outcome to the caller.
    #[tokio::test]
    async fn test_update_and_publish_acceptance_publishes_the_edit_at_the_current_share_state() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        // The persona is already shared in this scope.
        prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true)).unwrap();
        let prepared = prepared(&db_path, spawn_relay(true).await, keys, None);
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Published
        );
        assert!(
            result.persona.shared,
            "an ordinary edit must not silently unshare the persona"
        );
        assert!(
            !get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    #[tokio::test]
    async fn test_update_and_publish_relay_rejection_reports_queued_not_failure() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true)).unwrap();
        let prepared = prepared(&db_path, spawn_relay(false).await, keys, None);
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Queued
        );
        assert!(result
            .relay_message
            .as_deref()
            .is_some_and(|message| message.contains("relay rejected event")));
        assert!(
            get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync,
            "the edit stays queued for the flush loop"
        );
    }

    /// The save path swallows enqueue failures (`retain_persona_pending` logs
    /// them). This command promises a publication outcome, so the strict
    /// preparation it uses must surface the failure instead.
    #[tokio::test]
    async fn test_update_and_publish_enqueue_failure_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();

        let error = prepare_persona_publication_at(dir.path(), &keys, &persona(), None)
            .expect_err("a directory cannot be opened as the retention database");

        assert!(error.contains("failed to open retention db"));
    }

    /// Build a headless mock app for tests that need a full `AppHandle`.
    ///
    /// Shares the same pattern used in `concurrent_edit_tests.rs`. Use
    /// `lock_path_mutex()` + `HOME`/`XDG_DATA_HOME` overrides around this in
    /// tests that touch file-backed stores.
    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        let state = build_app_state();
        tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds headless")
    }

    /// P2 relay-sync regression through the real `prepare_persona_publication_at`
    /// failure mode: when the retention DB path is unwritable, the relay kind:0
    /// profile sync for linked agents must still complete before the error propagates.
    ///
    /// Uses the same retain-closure shape as `update_persona_and_publish` (calls
    /// `prepare_persona_publication_at` with `?`), exercising the strict-preparation
    /// seam directly. The failure is induced by pre-creating the retention DB path
    /// as a directory — verified independently by
    /// `test_update_and_publish_enqueue_failure_is_returned`.
    ///
    /// Mutation acceptance: restoring `retain_result?` inside
    /// `Ok((result, retain_result?, …))` at the blocking-phase return causes
    /// phase 2 to be skipped → counter receives 0 requests → RED.
    #[test]
    fn test_update_and_publish_relay_profile_syncs_despite_preparation_failure() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tauri::Manager;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home_p2_seam");
        std::fs::create_dir_all(&home).unwrap();

        // Use a separate dir as the "bad" retention DB path (a directory that
        // SQLite cannot open as a file). This simulates `prepare_persona_publication`
        // failing via `open_retention_db`.
        let bad_db_path = temp.path().join("bad-db-dir");
        std::fs::create_dir_all(&bad_db_path).unwrap();

        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        let _path_guard = crate::managed_agents::lock_path_mutex();
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &home);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime");

        rt.block_on(async {
            let app = mock_app();

            // Spawn a local HTTP server that counts POST /events requests.
            // sync_managed_agent_profile posts kind:0 profile events here.
            let post_count = Arc::new(AtomicUsize::new(0));
            let post_count_clone = post_count.clone();
            let relay_server = {
                use axum::{routing::post, Router};
                let app_router = Router::new().route(
                    "/events",
                    post(move |_body: String| {
                        let counter = post_count_clone.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            serde_json::json!({
                                "event_id": "test-event-id",
                                "accepted": true,
                                "message": ""
                            })
                            .to_string()
                        }
                    }),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                tokio::spawn(async move {
                    axum::serve(listener, app_router).await.ok();
                });
                format!("http://{addr}")
            };

            // Point the workspace relay override at the counter server.
            {
                let state = app.state::<AppState>();
                let mut override_slot = state
                    .relay_url_override
                    .lock()
                    .expect("relay_url_override must be lockable");
                *override_slot = Some(relay_server.clone());
            }

            // Seed persona "Alice" at a known revision.
            let r1 = "2026-01-01T00:00:00Z";
            save_personas(
                app.handle(),
                &[AgentDefinition {
                    id: "p1".to_string(),
                    display_name: "Alice".to_string(),
                    updated_at: r1.to_string(),
                    created_at: r1.to_string(),
                    ..persona()
                }],
            )
            .expect("seed must succeed");

            // Seed a linked agent with valid NSEC keys so sync_managed_agent_profile
            // can sign and submit the kind:0 profile event.
            let agent_keys = nostr::Keys::generate();
            let agent_record = ManagedAgentRecord {
                pubkey: agent_keys.public_key().to_hex(),
                name: "Alice".to_string(),
                persona_id: Some("p1".to_string()),
                private_key_nsec: agent_keys.secret_key().to_secret_hex(),
                auth_tag: None,
                relay_url: String::new(),
                avatar_url: None,
                acp_command: String::new(),
                agent_command: String::new(),
                agent_command_override: None,
                agent_args: vec![],
                mcp_command: String::new(),
                turn_timeout_seconds: 0,
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
                parallelism: 1,
                system_prompt: None,
                model: None,
                provider: None,
                persona_source_version: None,
                env_vars: BTreeMap::new(),
                start_on_app_launch: false,
                auto_restart_on_config_change: false,
                runtime_pid: None,
                backend: Default::default(),
                backend_agent_id: None,
                provider_policy_pending: false,
                provider_binary_path: None,
                team_id: None,
                persona_team_dir: None,
                persona_name_in_team: None,
                created_at: String::new(),
                updated_at: String::new(),
                last_started_at: None,
                last_stopped_at: None,
                last_exit_code: None,
                last_error: None,
                last_error_code: None,
                respond_to: Default::default(),
                respond_to_allowlist: vec![],
                display_name: Some("Alice".to_string()),
                description: None,
                slug: None,
                runtime: None,
                name_pool: vec![],
                is_builtin: false,
                is_active: true,
                shared: false,
                source_team: None,
                source_team_persona_slug: None,
                catalog_source: None,
                team_catalog_source: None,
                definition_respond_to: None,
                definition_respond_to_allowlist: vec![],
                definition_parallelism: None,
                relay_mesh: None,
                effort_level: None,
            };
            save_managed_agents(app.handle(), &[agent_record])
                .expect("agent seed must succeed");

            // Submit a rename with a retain closure that mirrors the
            // `update_persona_and_publish` shape: calls `prepare_persona_publication_at`
            // with `?` — but uses `bad_db_path` (a directory) to force EISDIR failure.
            let bad_db = bad_db_path.clone();
            let result = update_persona_with_seam(
                UpdatePersonaRequest {
                    id: "p1".to_string(),
                    display_name: "Alice Renamed".to_string(),
                    avatar_url: None,
                    description: None,
                    system_prompt: "Do the work.".to_string(),
                    runtime: None,
                    model: None,
                    provider: None,
                    name_pool: Vec::new(),
                    env_vars: None,
                    behavior: None,
                    expected_updated_at: Some(r1.to_string()),
                },
                app.handle().clone(),
                move |_app, _state, persona_def| {
                    // Mirror the update_persona_and_publish retain shape:
                    // prepare_persona_publication_at with ? propagates the Err.
                    let keys = nostr::Keys::generate();
                    let (_event, retained, _def) =
                        prepare_persona_publication_at(&bad_db, &keys, persona_def, None)?;
                    Ok(retained)
                },
            )
            .await;

            // The preparation failure must propagate (coordinator sees publishFailed).
            assert!(
                result.is_err(),
                "update_persona_with must return Err when prepare_persona_publication_at fails"
            );
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .contains("failed to open retention db"),
                "error must come from prepare_persona_publication_at, got: {:?}",
                result
            );

            // Phase 2 must have run: relay sync fires despite the retain failure.
            // Before the fix (retain_result? inside the blocking Ok): count is 0 → RED.
            // After the fix (retain_result? after phase 2): count is 1 → GREEN.
            assert_eq!(
                post_count.load(Ordering::SeqCst),
                1,
                "relay kind:0 profile sync must fire despite prepare_persona_publication_at failure; \
                 restoring `retain_result?` before phase 2 turns this RED"
            );
        }); // rt.block_on

        // Cleanup
        std::env::remove_var("HOME");
        std::env::remove_var("XDG_DATA_HOME");
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    /// Lock-scope regression for P1-2: the synchronous refresh inside
    /// `publish_and_refresh_teams_at` must hold `managed_agents_store_lock`
    /// for its entire read+retain sequence so it is serialized with concurrent
    /// team edits, unshare, and delete operations.
    ///
    /// Test structure uses a `std::sync::Barrier(2)` to coordinate a racing
    /// OS thread that tries to acquire the same lock:
    ///
    /// - WITH the lock (current code): observer holds lock → barrier releases
    ///   race thread → race thread blocks on lock → observer returns → refresh
    ///   runs → lock releases → race thread acquires lock → retains unshared
    ///   T+1 → final: UNSHARED ✓
    ///
    /// - WITHOUT the lock (mutation — move/remove the lock acquisition):
    ///   observer fires, try_lock() SUCCEEDS, the first assert panics → RED.
    ///
    /// The barrier + final-state assertion confirm the complete serialization
    /// contract: only by holding the lock across the full refresh does the
    /// unshare reliably get the last write.
    #[tokio::test]
    async fn test_retry_refresh_holds_store_lock_during_refresh() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Barrier};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let relay_url = spawn_relay(true).await;
        let prepared = prepared(&db_path, relay_url, keys.clone(), Some(true));
        let state = Arc::new(build_app_state());

        // Seed a shared team head so race_task has a concrete retention target.
        // teams.json is intentionally EMPTY (no 30178 heads for refresh to
        // overwrite) — the test focuses on lock-scope, not on refresh output.
        std::fs::write(dir.path().join("teams.json"), b"[]").unwrap();

        // Seed a shared persona kind:30175 in the db so the race task can
        // retain an unshared variant at T+1 — the resource both sides contend.
        let (_, initial_retained, _) =
            prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true)).unwrap();
        let initial_created_at = initial_retained.created_at;

        // A 2-party barrier: one party is the REFRESH_LOCK_OBSERVER (on the
        // tokio thread) and the other is the race OS thread. When both call
        // barrier.wait(), both are released simultaneously. The observer fires
        // WHILE the lock is held, so the race thread will immediately contend.
        let barrier = Arc::new(Barrier::new(2));
        let barrier_obs = barrier.clone();

        let observer_fired = Arc::new(AtomicBool::new(false));
        let observer_fired_clone = observer_fired.clone();
        {
            let mut slot = REFRESH_LOCK_OBSERVER.lock().expect("observer slot");
            *slot = Some(Box::new(move |state: &AppState| {
                // Mutation target: moving the lock acquisition to AFTER this call
                // causes try_lock() to succeed — test turns RED immediately.
                assert!(
                    state.managed_agents_store_lock.try_lock().is_err(),
                    "managed_agents_store_lock must be held during the refresh — \
                     a successful try_lock recreates the concurrent-unshare race condition"
                );
                observer_fired_clone.store(true, Ordering::SeqCst);
                // Release the race thread. Since the lock is still held here
                // (we are inside `{ let _guard = ...; }`) the race thread will
                // immediately block on lock().
                barrier_obs.wait();
            }));
        }

        // Race thread: waits at the barrier (released by the observer while
        // the lock is held), then immediately tries to acquire the lock. With
        // the lock in place it blocks; after the refresh releases it, it
        // retains an unshared persona event.
        let state_race = state.clone();
        let db_path_race = db_path.clone();
        let keys_race = keys.clone();
        let persona_race = persona();
        let race_thread = std::thread::spawn(move || {
            barrier.wait(); // released by the observer while lock is held
                            // This will block until publish_and_refresh_teams_at releases the lock.
            let _guard = state_race
                .managed_agents_store_lock
                .lock()
                .expect("race lock must not be poisoned");
            // Retain an unshared persona head — this is the write that the
            // concurrent-unshare scenario must not lose.
            prepare_persona_publication_at(&db_path_race, &keys_race, &persona_race, Some(false))
                .expect("race task unshare must succeed");
        });

        let persona_id = prepared.persona.id.clone();
        let result = publish_and_refresh_teams_at(
            &state,
            prepared,
            dir.path(),
            &keys,
            &db_path,
            &persona_id,
        )
        .await;

        // Clear observer before any assertion that could panic.
        {
            let mut slot = REFRESH_LOCK_OBSERVER.lock().expect("observer slot");
            *slot = None;
        }

        // Wait for the race thread to complete its unshare write.
        race_thread.join().expect("race thread must not panic");

        result.expect("publish_and_refresh_teams_at must succeed");
        assert!(
            observer_fired.load(Ordering::SeqCst),
            "the refresh lock observer must have fired — if not, the hook is not wired"
        );

        // The race thread always runs AFTER the refresh (because the lock
        // serializes them) and retains an unshared event with a strictly
        // later monotonic timestamp. The final db state must be UNSHARED.
        let owner = keys.public_key().to_hex();
        let conn = open_retention_db(&db_path).unwrap();
        let retained = get_retained_event(
            &conn,
            buzz_core_pkg::kind::KIND_PERSONA,
            &owner,
            "catalog-reviewer",
        )
        .expect("db query must not fail")
        .expect("retained persona event must exist");
        // The race task wrote at T+1 (strictly after the initial T);
        // assert it is both later than the seed AND unshared.
        assert!(
            retained.created_at > initial_created_at,
            "retained event must be later than the seed (race task ran last)"
        );
        use buzz_core_pkg::kind::event_is_shared;
        use nostr::JsonUtil;
        let event =
            nostr::Event::from_json(&retained.raw_event).expect("must parse retained event");
        assert!(
            !event_is_shared(&event),
            "final retained persona event must be UNSHARED — \
             race task ran last and its write must not have been overwritten"
        );
    }
}
