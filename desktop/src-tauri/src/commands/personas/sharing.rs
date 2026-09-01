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
    update_persona_and_publish_inner(input, app).await
}

/// Generic core of [`update_persona_and_publish`], testable with
/// `tauri::test::MockRuntime` as well as the production `Wry` runtime.
///
/// Extracted so tests can invoke the real command path — including the
/// `prepare_persona_publication` scope resolver — through a mock `AppHandle`
/// without the `#[tauri::command]` signature binding to `AppHandle<Wry>`.
pub(crate) async fn update_persona_and_publish_inner<R: tauri::Runtime>(
    input: crate::managed_agents::UpdatePersonaRequest,
    app: AppHandle<R>,
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
mod tests {
    use super::*;
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

    /// P2 relay-sync regression through the real `update_persona_and_publish`
    /// command path: when the active retention DB path is unwritable, the relay
    /// kind:0 profile sync for linked agents must still complete before the
    /// error propagates.
    ///
    /// Drives `update_persona_and_publish_inner` — the generic core of the
    /// exported `update_persona_and_publish` command — through a mock AppHandle,
    /// exercising the full wiring: `prepare_persona_publication` scope resolver,
    /// the strict-preparation `?`-propagation, and the phase-2 relay sync. The
    /// failure is induced by replacing the active retention DB path with a
    /// directory (EISDIR) after the relay override is set so the scope hash is
    /// stable.
    ///
    /// Mutation acceptance: restoring `retain_result?` inside
    /// `Ok((result, retain_result?, …))` at the blocking-phase return causes
    /// phase 2 to be skipped → counter receives 0 requests → RED.
    #[test]
    fn test_update_and_publish_relay_profile_syncs_despite_preparation_failure() {
        use crate::managed_agents::retention::active_retention_scope;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tauri::Manager;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home_p2_seam");
        std::fs::create_dir_all(&home).unwrap();

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
            // Must be set BEFORE computing the active retention scope so the
            // scope hash is stable for the sabotage step.
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

            // Resolve the active retention scope (relay + owner → DB path) and
            // sabotage it: replace the .db file path with a directory so that
            // `open_retention_db` inside `prepare_persona_publication` returns
            // "failed to open retention db". This exercises the production scope
            // resolver rather than injecting an arbitrary bad path.
            {
                let state = app.state::<AppState>();
                let scope = active_retention_scope(app.handle(), &state)
                    .expect("active_retention_scope must resolve with relay override set");
                // Remove the file if it was created by scope resolution, then
                // create a directory at the same path so SQLite cannot open it.
                std::fs::remove_file(&scope.db_path).ok();
                std::fs::create_dir_all(&scope.db_path)
                    .expect("must be able to create directory at db path for sabotage");
            }

            // Drive the REAL command path through a mock AppHandle. This exercises
            // prepare_persona_publication (scope resolver + ?-propagation) and the
            // phase-2 relay sync, verifying that wiring drift at the command
            // boundary — e.g. update_persona_and_publish stopping to call
            // update_persona_with — is caught.
            let result = update_persona_and_publish_inner(
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
            )
            .await;

            // The preparation failure must propagate (coordinator sees publishFailed).
            assert!(
                result.is_err(),
                "update_persona_and_publish_inner must return Err when prepare_persona_publication fails"
            );
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .contains("failed to open retention db"),
                "error must come from prepare_persona_publication scope resolver, got: {:?}",
                result
            );

            // Phase 2 must have run: relay sync fires despite the retain failure.
            // Before the fix (retain_result? inside the blocking Ok): count is 0 → RED.
            // After the fix (retain_result? after phase 2): count is 1 → GREEN.
            assert_eq!(
                post_count.load(Ordering::SeqCst),
                1,
                "relay kind:0 profile sync must fire despite prepare_persona_publication failure; \
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
    /// Test structure:
    ///
    /// 1. Seeds a shared kind:30178 team head T in the retention DB.
    /// 2. A `REFRESH_READ_OBSERVER` fires inside `refresh_or_retract_shared_head_at`
    ///    AFTER it reads the shared head T but BEFORE it retains the refreshed
    ///    head T+1. The observer uses a `Barrier(2)` to synchronize with a
    ///    racing OS thread that tries to write an unshared T+1 through the same
    ///    `managed_agents_store_lock`.
    /// 3. WITH the lock held (current code): the race thread blocks on the lock
    ///    while refresh retains T+1 (shared), then the race thread acquires the
    ///    lock and retains its unshared T+1 — last writer wins → UNSHARED ✓.
    /// 4. WITHOUT the lock (mutation: move refresh outside `{ let _guard = ... }`):
    ///    the race thread is not blocked; it writes its unshared T+1 first, then
    ///    refresh retains its shared T+1 — last writer wins → SHARED ✗ → RED.
    ///
    /// The `retain_event` function accepts equal timestamps (monotonic_created_at
    /// for both sides yields the same T+1), so the last caller always wins the
    /// UPSERT — which is exactly what the lock is supposed to prevent.
    #[tokio::test]
    async fn test_retry_refresh_holds_store_lock_during_refresh() {
        use crate::commands::teams::{prepare_team_publication_at, REFRESH_READ_OBSERVER};
        use crate::managed_agents::{
            retention::{get_retained_event, open_retention_db},
            TeamRecord,
        };
        use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
        use nostr::JsonUtil;
        use std::collections::BTreeMap;
        use std::sync::{Arc, Barrier};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let relay_url = spawn_relay(true).await;
        let state = Arc::new(build_app_state());

        // The team must include our persona so refresh_for_persona_at picks it up.
        let persona_id = "catalog-reviewer";
        let team = TeamRecord {
            id: "team-abc".to_string(),
            name: "Test Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: vec![persona_id.to_string()],
            is_builtin: false,
            shared: true,
            catalog_source: None,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let persona_member = AgentDefinition {
            id: persona_id.to_string(),
            display_name: "Catalog Reviewer".to_string(),
            description: None,
            avatar_url: None,
            system_prompt: "Review.".to_string(),
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
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        // Seed a real shared kind:30178 head T in the retention DB.
        prepare_team_publication_at(
            &db_path,
            &keys,
            &team,
            std::slice::from_ref(&persona_member),
            Some(true),
        )
        .expect("seed shared team head must succeed");

        // Write teams.json with a REAL shared team entry so refresh_for_persona_at
        // finds a team containing our persona and calls refresh_or_retract_shared_head_at.
        let teams_json = serde_json::to_string(&[&team]).expect("serialize team");
        std::fs::write(dir.path().join("teams.json"), teams_json.as_bytes()).unwrap();

        // Write personas.json with a MODIFIED persona (different system_prompt) so
        // refresh_or_retract_shared_head_at rebuilds different content than the seed T
        // and does NOT hit the idempotency skip — ensuring it actually calls retain_event
        // (which is necessary for the race to matter).
        let persona_updated = AgentDefinition {
            id: persona_id.to_string(),
            display_name: "Catalog Reviewer".to_string(),
            description: None,
            avatar_url: None,
            system_prompt: "Review catalog entries carefully.".to_string(), // differs from seed
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
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let personas_json =
            serde_json::to_string(&[&persona_updated]).expect("serialize persona_updated");
        std::fs::write(dir.path().join("personas.json"), personas_json.as_bytes())
            .expect("write personas.json must succeed");

        // Read the seed T so we can assert the final head is strictly newer.
        let initial_created_at = {
            let conn = open_retention_db(&db_path).unwrap();
            get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc")
                .unwrap()
                .expect("seed head must exist")
                .created_at
        };

        // Barrier: two parties — the REFRESH_READ_OBSERVER (tokio task) and the
        // race OS thread. Both call barrier.wait() to synchronize.
        let barrier = Arc::new(Barrier::new(2));
        let barrier_obs = barrier.clone();
        let state_obs = state.clone();

        {
            let mut slot = REFRESH_READ_OBSERVER.lock().expect("observer slot");
            *slot = Some(Box::new(move || {
                // We are inside refresh_or_retract_shared_head_at, after the
                // shared-T read, before the retain. The caller holds
                // managed_agents_store_lock. Prove it directly:
                //
                // Mutation target: moving `refresh_for_persona_at` outside the
                // lock block causes try_lock() to SUCCEED here — the test turns
                // RED immediately at this assertion, proving the lock is not held
                // at the read+retain gap.
                assert!(
                    state_obs.managed_agents_store_lock.try_lock().is_err(),
                    "managed_agents_store_lock must be held while refresh_or_retract_shared_head_at \
                     holds the shared-T read open — a successful try_lock means the lock was \
                     released before refresh finished, recreating the concurrent-unshare race"
                );
                // Signal the race thread to proceed. Since the lock IS held here
                // (assertion above passed), the race thread will immediately block
                // at lock(), keeping the ordered-write guarantee.
                barrier_obs.wait();
                // Return — refresh proceeds to retain T+1, then releases the lock,
                // then the race thread unblocks and retains its UNSHARED T+1 last.
            }));
        }

        // Race thread: waits at the barrier (released by the observer WHILE the
        // lock is held), then acquires the same lock and retains an unshared T+1.
        // With the lock: blocked until refresh retains T+1, then it retains its
        // own unshared T+1 — final = UNSHARED ✓.
        // Without the lock (mutation): races freely, retains unshared first, then
        // refresh retains shared T+1 over it — final = SHARED ✗.
        let state_race = state.clone();
        let db_path_race = db_path.clone();
        let keys_race = keys.clone();
        let team_race = team.clone();
        let member_race = AgentDefinition {
            id: persona_id.to_string(),
            display_name: "Catalog Reviewer".to_string(),
            description: None,
            avatar_url: None,
            system_prompt: "Review.".to_string(),
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
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let race_thread = std::thread::spawn(move || {
            barrier.wait(); // released by observer while lock is held
                            // Acquire the same store lock — this blocks until publish_and_refresh_teams_at
                            // releases it, ensuring the race write comes AFTER refresh's retain.
            let _guard = state_race
                .managed_agents_store_lock
                .lock()
                .expect("race lock must not be poisoned");
            // Retain an unshared team head T+1 — the write that must win.
            prepare_team_publication_at(
                &db_path_race,
                &keys_race,
                &team_race,
                &[member_race],
                Some(false), // unshare
            )
            .expect("race unshare must succeed");
        });

        // Run publish_and_refresh_teams_at using a prepared shared persona head.
        let prepared = prepared(&db_path, relay_url, keys.clone(), Some(true));
        let persona_id_str = prepared.persona.id.clone();
        let result = publish_and_refresh_teams_at(
            &state,
            prepared,
            dir.path(),
            &keys,
            &db_path,
            &persona_id_str,
        )
        .await;

        // Clear observer before any assertion that could panic.
        {
            let mut slot = REFRESH_READ_OBSERVER.lock().expect("observer slot");
            *slot = None;
        }

        // Wait for the race thread to complete its unshare write.
        race_thread.join().expect("race thread must not panic");

        result.expect("publish_and_refresh_teams_at must succeed");

        // The race thread always runs AFTER refresh (because the lock serializes
        // them) and retains an unshared head at T+1. Since retain_event accepts
        // equal timestamps, the last writer wins — and the race thread is last.
        // Mutation (move refresh outside lock): race thread writes first, refresh
        // writes over it → SHARED → test turns RED.
        let conn = open_retention_db(&db_path).unwrap();
        let retained = get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc")
            .expect("db query must not fail")
            .expect("retained team catalog head must exist after refresh + race");
        assert!(
            retained.created_at >= initial_created_at,
            "retained head must be at or after the seed timestamp"
        );
        let event =
            nostr::Event::from_json(&retained.raw_event).expect("must parse retained event");
        assert!(
            !event_is_shared(&event),
            "final retained kind:30178 head must be UNSHARED — \
             race thread ran last (lock held during refresh) and its write must not \
             have been overwritten by the refreshed shared head; \
             moving refresh outside the lock causes the shared head to win → RED"
        );
    }
}
