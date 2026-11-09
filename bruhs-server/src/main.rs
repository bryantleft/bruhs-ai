mod config;
mod error;
mod local_access;
mod parent_watch;
mod routes;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post},
    Router,
};
use bruhs_core::AppState;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{Config, ServerState};

/// Default body cap for API requests. 1 MiB is generous for JSON payloads.
const MAX_BODY_BYTES: usize = 1 << 20;
/// Larger cap for voice-enrollment WAV uploads (5s @ 16kHz mono i16 ≈ 160 KB;
/// 4 MiB gives comfortable headroom without being a footgun).
const ENROLL_BODY_BYTES: usize = 4 << 20;
/// One attached file. Images are rejected above 10 MiB after the body arrives.
const UPLOAD_BODY_BYTES: usize = 50 << 20;

/// How long open connections (SSE streams never end on their own) may run
/// after a shutdown signal before the server exits anyway.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if bruhs_core::secrets::is_mcp_helper() {
        bruhs_core::secrets::run_mcp_helper().await?;
        return Ok(());
    }
    tracing_subscriber::registry()
        .with(log_filter())
        .with(tracing_subscriber::fmt::layer())
        .init();
    parent_watch::exit_with_parent();

    let config = Arc::new(Config::from_env()?);
    let store =
        bruhs_core::SqliteMeetingStore::open(bruhs_core::SqliteMeetingStore::default_path()?)
            .await?;
    let state = AppState::builder().meetings(Arc::new(store)).build();
    let state = Arc::new(state);
    // Google Calendar sync (when BRUHS_GOOGLE_* is configured).
    state.start_background(&tokio::runtime::Handle::current());
    // One upstream subscription and bounded linking worker for the process,
    // including when no browser/native SSE clients are connected.
    let _capture_pump = routes::meetings_stream::start(state.clone());

    // On boot: reconcile any meetings that were Live when the server last
    // stopped. If the capture daemon is reachable, meetings that are Live
    // but not currently recording are flipped to Failed. If the daemon is
    // unreachable the reconcile is skipped silently (daemon may be starting
    // up alongside the server).
    {
        let s = Arc::clone(&state);
        tokio::spawn(async move {
            s.reconcile_orphaned_meetings().await;
        });
    }

    if config.spa_dir.is_dir() {
        tracing::info!("serving the web app from {}", config.spa_dir.display());
    } else {
        tracing::warn!(
            "web app not found at {} (run `trunk build` in bruhs-web or set BRUHS_WEB_DIST); \
             only the API is served",
            config.spa_dir.display()
        );
    }
    let app = router(ServerState {
        app: state,
        config: Arc::clone(&config),
        updates: bruhs_core::updates::UpdateChecker::new(
            bruhs_core::updates::BuildInfo::new(
                env!("BRUHS_BUILD_COMMIT"),
                env!("BRUHS_REPO_ROOT"),
                "scripts/dev.sh",
            )
            .with_release(env!("BRUHS_RELEASE_VERSION")),
        ),
    });

    let addr = format!("127.0.0.1:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("bruhs-server listening on http://{addr}");
    serve(listener, app).await?;
    Ok(())
}

/// `RUST_LOG` as `target=level` directives (e.g. `info,tower_http=debug`),
/// defaulting to `info`.
fn log_filter() -> Targets {
    // Unset (or not UTF-8) reads as the default filter.
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| match value.parse::<Targets>() {
            Ok(targets) => Some(targets),
            Err(error) => {
                eprintln!("bruhs-server: ignoring RUST_LOG ({error})");
                None
            }
        })
        .unwrap_or_else(|| Targets::new().with_default(tracing::Level::INFO))
}

/// Serve until SIGINT/SIGTERM, then stop accepting connections and give
/// open ones [`SHUTDOWN_GRACE`] to finish.
async fn serve(listener: tokio::net::TcpListener, app: Router) -> std::io::Result<()> {
    let (stop, mut stopped) = tokio::sync::watch::channel(());
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            // An error means the sender is gone, which is a stop as well.
            if stopped.changed().await.is_err() {
                tracing::debug!("shutdown sender dropped");
            }
        })
        .into_future();
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => return result,
        () = shutdown_signal() => {}
    }
    tracing::info!("shutting down");
    // `stopped` is alive inside the graceful-shutdown future, so this
    // cannot fail; nothing to do if it somehow did.
    if stop.send(()).is_err() {
        tracing::debug!("server already stopped");
    }
    match tokio::time::timeout(SHUTDOWN_GRACE, server).await {
        Ok(result) => result,
        Err(_) => {
            tracing::info!("connections still open after {SHUTDOWN_GRACE:?}; exiting");
            Ok(())
        }
    }
}

async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!("cannot listen for Ctrl-C: {error}");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!("cannot listen for SIGTERM: {error}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

fn router(state: ServerState) -> Router {
    let access = state.config.access.clone();
    let cors = access.cors().expose_headers([
        axum::http::HeaderName::from_static("x-bruhs-transcript-source"),
        axum::http::HeaderName::from_static("x-bruhs-transcript-refresh"),
    ]);
    let spa = ServeDir::new(&state.config.spa_dir);

    Router::new()
        .route("/api/agent", post(routes::agent::post_agent))
        .route("/api/agent/steer", post(routes::agent::post_steer))
        .route(
            "/api/agent/second-opinion",
            post(routes::agent::post_second_opinion),
        )
        .route("/api/agent/auth", get(routes::agent_auth::get_auth))
        .route(
            "/api/agent/auth/{provider}",
            get(routes::agent_auth::get_auth_for_provider),
        )
        .route("/api/health", get(routes::health::get_health))
        .route("/api/updates", get(routes::updates::get_updates))
        .route(
            "/api/updates/providers/{provider}",
            post(routes::updates::post_provider_update),
        )
        .route("/api/agent/approve", post(routes::approve::post_approve))
        .route("/api/agent/secret", post(routes::secrets::submit))
        .route("/api/credentials", get(routes::secrets::list))
        .route("/api/connections", get(routes::connections::list))
        .route(
            "/api/connections/{provider}/start",
            post(routes::connections::start),
        )
        .route(
            "/api/connections/{provider}",
            get(routes::connections::get).delete(routes::connections::disconnect),
        )
        .route(
            "/api/connections/attempts/{id}",
            get(routes::connections::attempt).delete(routes::connections::cancel),
        )
        .route(
            "/api/credentials/{id}",
            axum::routing::put(routes::secrets::replace).delete(routes::secrets::delete),
        )
        .route(
            "/api/open-in-editor",
            post(routes::editor::post_open_in_editor),
        )
        .route("/api/terminal/ws", get(routes::terminal::ws_terminal))
        .route(
            "/api/terminal/sessions",
            get(routes::terminal::list_sessions).post(routes::terminal::create_session),
        )
        .route(
            "/api/terminal/sessions/{id}",
            delete(routes::terminal::delete_session),
        )
        .route(
            "/api/preview/servers",
            get(routes::preview::get_local_servers),
        )
        .route("/api/import/sessions", get(routes::import::get_sessions))
        .route("/api/import/thread", get(routes::import::get_thread))
        .route("/api/export/t3", post(routes::import::post_export_t3))
        .route("/api/agent/files", post(routes::files::post_files))
        .route(
            "/api/agent/files/children",
            post(routes::files::post_children),
        )
        .route("/api/agent/pulls", post(routes::pulls::post_pulls))
        .route(
            "/api/agent/branch-pull",
            post(routes::pulls::post_branch_pull),
        )
        .route("/api/agent/skills", post(routes::skills::post_skills))
        .route(
            "/api/agent/uploads",
            post(routes::uploads::post_upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_BYTES)),
        )
        .route("/api/agent/uploads/open", get(routes::uploads::get_open))
        .route("/api/agent/git-branches", post(routes::git::get_branches))
        .route("/api/agent/git-checkout", post(routes::git::post_checkout))
        .route("/api/agent/worktree", post(routes::git::post_worktree))
        .route(
            "/api/agent/worktree-snapshot",
            post(routes::git::post_snapshot),
        )
        .route(
            "/api/agent/worktree-restore",
            post(routes::git::post_restore),
        )
        .route("/api/agent/git-status", post(routes::git::get_status))
        .route("/api/agent/git-diff", post(routes::git::get_diff))
        .route("/api/agent/list-dir", post(routes::list_dir::post_list_dir))
        .route(
            "/api/agent/validate-path",
            post(routes::validate_path::post_validate),
        )
        .route("/api/agent/login", post(routes::login::post_login))
        .route(
            "/api/agent/login/{provider}",
            post(routes::login::post_login_for_provider),
        )
        .route("/api/tasks", get(routes::tasks::get_tasks))
        .route(
            bruhs_models::api::live_events::PATH,
            get(routes::events_stream::get_events_stream),
        )
        .route("/api/tasks/stream", get(routes::tasks::get_tasks_stream))
        .route("/api/tasks/{id}", delete(routes::tasks::delete_task))
        .route("/api/meetings", get(routes::meetings::list_meetings))
        .route(
            "/api/meetings/{id}/local-focus",
            get(routes::local_focus::list).post(routes::local_focus::append),
        )
        .route("/api/meetings", post(routes::meetings::create_meeting))
        .route("/api/meetings/{id}", get(routes::meetings::get_meeting))
        .route("/api/meetings/{id}", patch(routes::meetings::patch_meeting))
        .route(
            "/api/meetings/{id}",
            delete(routes::meetings::delete_meeting),
        )
        .route(
            "/api/meetings/{id}/stop",
            post(routes::meetings::stop_meeting),
        )
        .route(
            "/api/meetings/{id}/speakers/{speaker_id}",
            patch(routes::meetings::patch_speaker),
        )
        .route(
            "/api/meetings/{id}/speakers/{speaker_id}/link/{person_id}",
            post(routes::meetings::link_speaker),
        )
        .route(
            "/api/meetings/{id}/segments",
            get(routes::meetings::list_segments),
        )
        .route(
            "/api/meetings/{id}/segments/{segment_id}",
            delete(routes::meetings::remove_segment),
        )
        .route(
            "/api/meetings/{id}/segment-removals",
            get(routes::meetings::list_segment_removals),
        )
        .route(
            "/api/meetings/{id}/chat",
            post(routes::meetings::create_or_get_chat),
        )
        .route(
            "/api/meetings/{id}/summarize",
            post(routes::meetings_summary::post_summarize),
        )
        .route(
            "/api/meetings/{id}/generations/{request_id}",
            get(routes::meetings_summary::get_generation)
                .delete(routes::meetings_summary::cancel_generation),
        )
        .route(
            "/api/meetings/{id}/generations",
            get(routes::meetings_summary::get_pending_generation),
        )
        .route(
            "/api/meetings/{id}/skip_observations",
            axum::routing::patch(routes::meetings::patch_meeting_skip_observations),
        )
        .route(
            "/api/meetings/stream",
            get(routes::meetings_stream::get_meetings_stream),
        )
        .route(
            "/api/people",
            get(routes::people::list_people).post(routes::people::create_person),
        )
        .route(
            "/api/people/{id}",
            get(routes::people::get_person).patch(routes::people::patch_person),
        )
        .route(
            "/api/people/resolve-merges",
            post(routes::people::resolve_merges),
        )
        .route("/api/people/versions", get(routes::people::person_versions))
        .route("/api/people/profiles", post(routes::people::get_profiles))
        .route(
            "/api/people/{id}/tracked",
            axum::routing::patch(routes::people::patch_person_tracked),
        )
        .route(
            "/api/people/{id}/rename",
            axum::routing::patch(routes::people::rename_person),
        )
        .route(
            "/api/people/{source_id}/merge_into/{target_id}",
            post(routes::people::merge_person_into),
        )
        .route(
            "/api/people/{id}/enroll",
            post(routes::people::enroll_voice).layer(DefaultBodyLimit::max(ENROLL_BODY_BYTES)),
        )
        .route(
            "/api/calendar/google/oauth/start",
            post(routes::calendar_google::oauth_start),
        )
        .route(
            "/api/calendar/google/oauth/callback",
            get(routes::calendar_google::oauth_callback),
        )
        .route(
            "/api/calendar/google/disconnect",
            post(routes::calendar_google::disconnect),
        )
        .route(
            "/api/calendar/google/state",
            get(routes::calendar_google::get_state),
        )
        .route(
            "/api/calendar/google/sync",
            post(routes::calendar_google::force_sync),
        )
        .route(
            "/api/calendar/google/calendars",
            get(routes::calendar_google::list_calendars),
        )
        .route(
            "/api/calendar/google/calendars/{id}",
            axum::routing::patch(routes::calendar_google::patch_calendar),
        )
        .route(
            "/api/calendar/events/stream",
            get(routes::calendar_events_stream::get_events_stream),
        )
        .route(
            "/api/capture/status",
            get(routes::capture_status::get_capture_status),
        )
        // The browser drives the capture daemon only through these; the
        // server attaches the daemon's bearer token.
        .route("/api/capture/start", post(routes::capture_proxy::start))
        .route("/api/capture/stop", post(routes::capture_proxy::stop))
        .route(
            "/api/capture/activity-monitor",
            get(routes::capture_proxy::activity_monitor_status)
                .post(routes::capture_proxy::activity_monitor_command),
        )
        .layer(axum::extract::Extension(std::sync::Arc::new(
            bruhs_core::capture_client::endpoint().clone(),
        )))
        .fallback_service(spa)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(
            access,
            local_access::guard,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
