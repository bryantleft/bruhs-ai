mod config;
mod error;
mod local_access;
mod parent_watch;
mod reactive_events;
#[cfg_attr(not(unix), path = "restore_control_unsupported.rs")]
mod restore_control;
mod routes;
mod service;
pub(crate) mod sse;
mod static_assets;
mod telemetry;
#[cfg(unix)]
mod workspace_relay;

#[cfg(test)]
mod http_performance_tests;

#[cfg(test)]
mod http_read_performance_tests;

// Exercise the desktop's production HTTP caller through this router without GPUI.
#[cfg(test)]
#[path = "../../bruhs-desktop/src/server_client.rs"]
pub mod desktop_server_client;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post, put},
    Router,
};
use bruhs_core::AppState;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;
use tower_http::trace::TraceLayer;
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
#[expect(
    clippy::significant_drop_tightening,
    reason = "RetentionHost is moved into the router Extension layer"
)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if bruhs_core::api_client_tools::is_mcp_helper() {
        bruhs_core::api_client_tools::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::workbench_tools::is_mcp_helper() {
        bruhs_core::workbench_tools::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::database_tools::is_mcp_helper() {
        bruhs_core::database_tools::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::website_reader::is_mcp_helper() {
        bruhs_core::website_reader::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::artifact_tools::is_mcp_helper() {
        bruhs_core::artifact_tools::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::reactive_tools::is_mcp_helper() {
        bruhs_core::reactive_tools::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::secrets::is_mcp_helper() {
        bruhs_core::secrets::run_mcp_helper().await?;
        return Ok(());
    }
    if bruhs_core::thread_link::is_mcp_helper() {
        bruhs_core::thread_link::run_mcp_helper().await?;
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("service") {
        let config = Config::from_env()?;
        std::process::exit(service::cli(&config.spa_dir).unwrap_or(2));
    }
    if let Some(code) = restore_control::restore_apply_cli() {
        std::process::exit(code);
    }
    tracing_subscriber::registry()
        .with(log_filter())
        .with(tracing_subscriber::fmt::layer())
        .init();
    parent_watch::exit_with_parent();

    let config = Arc::new(Config::from_env()?);
    // A restore is applied before any store opens (D16, ruling F20).
    if let Some(code) = restore_control::apply_at_start().await {
        std::process::exit(code);
    }
    let store =
        bruhs_core::SqliteMeetingStore::open(bruhs_core::SqliteMeetingStore::default_path()?)
            .await?;
    let secrets = bruhs_core::secrets::SecretChoice::from_env();
    let memory = secrets != bruhs_core::secrets::SecretChoice::Os;
    let secret_store = || secrets.store();
    // The web runs threads side by side, so an agent may wait for another
    // thread's reply inside one tool call.
    let state = AppState::builder()
        .thread_waits(true)
        .meetings(Arc::new(store));
    let state = if memory {
        state
            .secret_store(secret_store())
            .connections(bruhs_core::connections::Connections::in_memory())
    } else {
        state
    };
    let state = Arc::new(state.build());
    let reactive = bruhs_core::reactive::ReactiveRuntime::open(
        bruhs_core::reactive::ReactiveRuntime::default_path()?,
    )
    .await?;
    state.attach_reactive(reactive.clone())?;
    let mail =
        bruhs_core::mail::MailRuntime::open(bruhs_core::mail::MailRuntime::default_path()?).await?;
    state.attach_mail(mail)?;
    let gmail = Arc::new(bruhs_core::mail::gmail::GmailClient::new()?);
    let outlook = Arc::new(bruhs_core::mail::outlook::OutlookClient::new()?);
    // Claim the port before any configured source command or agent can run.
    // Owner refusals name this host (`SERVER_BIND_HOST`).
    let addr = format!("{}:{}", bruhs_models::api::SERVER_BIND_HOST, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    bruhs_core::workbench_tools::set_endpoint(format!("http://{}", listener.local_addr()?))?;
    // Bruh browsers: the host gets every bruh whose Computer is on, before
    // the worker can run one of their steps.
    let tools = bruhs_core::local_tools::LocalToolsHost::new()?;
    reactive.attach_computers(Arc::clone(&tools))?;
    let backup =
        routes::backup::start(secret_store(), Arc::clone(&tools), reactive.clone()).await?;
    let imports = routes::import::start();
    // Before the worker starts, so the first run's preamble and gate see
    // the vault.
    reactive.attach_vault(state.secrets().clone())?;
    // The process's one app client (D13), shared by the Apps routes, the
    // bruhs' calls and the polls; attached before the worker starts too.
    let apps = bruhs_core::apps::AppsHost::new(
        state.secrets().clone(),
        bruhs_core::apps::AppHttp::from_env()?,
        state.mail().cloned(),
        Arc::clone(&gmail),
        Arc::clone(&outlook),
    );
    reactive.attach_apps(apps.clone())?;
    let _reactive_worker = reactive.start(Arc::clone(&state))?;
    let _reactive_events = reactive_events::start(reactive.clone(), Arc::clone(&state));
    // Google Calendar sync (when BRUHS_GOOGLE_* is configured).
    state.start_background(&tokio::runtime::Handle::current());
    routes::uploads::schedule_capture_cleanup();
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
    let updates = bruhs_core::updates::UpdateChecker::new(
        bruhs_core::updates::BuildInfo::new(
            env!("BRUHS_BUILD_COMMIT"),
            env!("BRUHS_REPO_ROOT"),
            "scripts/dev.sh",
        )
        .with_release(env!("BRUHS_RELEASE_VERSION")),
    );
    let app = router(ServerState {
        app: state.clone(),
        config: Arc::clone(&config),
        updates: updates.clone(),
    })
    .layer(axum::Extension(reactive.clone()))
    .layer(axum::Extension(restore_control::real(backup.roots())))
    .layer(axum::Extension(backup))
    .layer(axum::Extension(apps))
    .layer(axum::Extension(gmail))
    .layer(axum::Extension(outlook));
    let app = routes::import::with_host(app, imports);
    let coordination = bruhs_core::coordination::CoordinationRuntime::open(
        bruhs_core::coordination::CoordinationRuntime::default_path()?,
        state.clone(),
    )
    .await?;
    let _reset_worker = coordination.start_reset_worker();
    let _wake_worker = coordination.start_wake_worker();
    let delivery = bruhs_core::delivery::DeliveryService::open(
        bruhs_core::delivery::DeliveryService::default_path()?,
    )
    .await?;
    let _delivery_watcher = delivery.start_watcher(state.clone(), Some(coordination.clone()))?;
    // Removing a worktree refuses one a pull-request watch still reads.
    bruhs_core::worktrees::register_watch_source(std::sync::Arc::new(delivery.clone()));
    let retention = bruhs_core::worktrees::retention::RetentionHost::new(
        state.clone(),
        coordination.clone(),
        delivery.clone(),
    );
    if let Some(file) = bruhs_core::delivery::writer::desktop_state_file() {
        tokio::spawn(retention.clone().run(file));
    }
    let board_links = bruhs_core::coordination::BoardLinks::new(delivery.clone());
    let databases = bruhs_core::database::DatabaseHost::open(
        bruhs_core::database::DatabaseHost::default_path()?,
        secret_store(),
    )
    .await?;
    let api_client = bruhs_core::api_client::ApiClientHost::with_origins(
        secret_store(),
        std::iter::once(listener.local_addr()?.port())
            .chain(
                reqwest::Url::parse(bruhs_core::capture_client::daemon_base_url())
                    .ok()
                    .and_then(|url| url.port_or_known_default()),
            )
            .collect(),
        [
            std::env::var("BRUHS_API_URL").ok(),
            std::env::var("BRUHS_WEB_ORIGIN").ok(),
            Some(bruhs_core::capture_client::daemon_base_url().into()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    )?;
    let remote_hosts = bruhs_core::remote_hosts::RemoteHostManager::open(
        bruhs_core::remote_hosts::RemoteHostManager::default_path()?,
    )
    .await?;
    let relay_mailbox = bruhs_core::relay_mailbox::RelayMailbox::open(
        bruhs_core::relay_mailbox::RelayMailbox::default_path()?,
    )
    .await?;
    let simlock_pool =
        bruhs_core::simlock::SimlockPool::open(bruhs_core::simlock::SimlockPool::default_path()?)
            .await?;
    let wsl_manager =
        bruhs_core::wsl::WslManager::open(bruhs_core::wsl::WslManager::default_path()?).await?;
    let app = app
        .merge(workbench_services(
            delivery,
            remote_hosts.clone(),
            config.access.clone(),
        ))
        .layer(axum::Extension(remote_hosts.clone()))
        .layer(axum::Extension(relay_mailbox))
        .layer(axum::Extension(simlock_pool))
        .layer(axum::Extension(wsl_manager))
        .layer(axum::Extension(retention))
        .layer(axum::Extension(coordination.clone()))
        .layer(axum::Extension(board_links.clone()))
        .layer(axum::Extension(tools.clone()))
        .layer(axum::Extension(databases))
        .layer(axum::Extension(api_client));
    let app = telemetry::attach(app, config.access.clone());
    if let Ok(bind) = std::env::var("BRUHS_TASK_BIND") {
        let address: std::net::SocketAddr = bind.parse()?;
        let remote = tokio::net::TcpListener::bind(address).await?;
        let remote_app = routes::coordination::remote_router(coordination, board_links)
            .layer(axum::Extension(updates))
            .layer(axum::Extension(reactive));
        tracing::info!(%address,"Paired listener enabled for scoped task and keybinding access");
        tokio::spawn(async move {
            if let Err(error) = axum::serve(remote, remote_app).await {
                tracing::error!(%error,"Paired task listener stopped");
            }
        });
    } else {
        drop(coordination);
    }

    tracing::info!("bruhs-server listening on http://{addr}");
    let result = serve(listener, app).await;
    remote_hosts.shutdown();
    // Bruh browsers keep profiles; a graceful close writes their cookies.
    tools.close_all().await;
    result?;
    Ok(())
}

fn workbench_services(
    delivery: bruhs_core::delivery::DeliveryService,
    hosts: bruhs_core::remote_hosts::RemoteHostManager,
    policy: local_access::Policy,
) -> Router {
    routes::delivery::router(delivery)
        .merge(routes::remote_hosts::router(hosts))
        .layer(policy.cors())
        .layer(axum::middleware::from_fn_with_state(
            policy,
            local_access::guard,
        ))
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
        () = restore_control::exit_requested() => {}
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
    router_with_capture_daemon(state, bruhs_core::capture_client::endpoint().clone())
}

fn router_with_capture_daemon(
    state: ServerState,
    capture_daemon: bruhs_core::capture_client::DaemonEndpoint,
) -> Router {
    let access = state.config.access.clone();
    let cors = access.cors().expose_headers([
        axum::http::HeaderName::from_static("x-bruhs-transcript-source"),
        axum::http::HeaderName::from_static("x-bruhs-transcript-refresh"),
        axum::http::HeaderName::from_static("x-bruhs-mail-export-size"),
    ]);
    let spa = static_assets::service(&state.config.spa_dir);
    #[cfg(unix)]
    let relay =
        workspace_relay::WorkspaceRelay::new(format!("http://127.0.0.1:{}", state.config.port));

    let router = Router::new()
        .merge(routes::mail::router())
        .route(
            "/api/agent-extensions/history/preview",
            post(routes::agent_extensions::history_preview),
        )
        .route(
            "/api/agent-extensions/history/{id}/commit",
            post(routes::agent_extensions::history_commit),
        )
        .route(
            "/api/workbench/ui",
            get(routes::agent_extensions::workbench),
        )
        .route(
            "/api/agent-extensions/ui",
            get(routes::agent_extensions::ui),
        )
        .route(
            "/api/agent-extensions/config",
            get(routes::agent_extensions::config).post(routes::agent_extensions::save),
        )
        .route(
            "/api/agent-extensions/config/agent",
            post(routes::agent_extensions::edit_agent),
        )
        .route(
            "/api/agent-extensions/pools.js",
            get(routes::agent_extensions::pools_script),
        )
        .route(
            "/api/agent-extensions/prices.js",
            get(routes::agent_extensions::prices::script),
        )
        .route(
            "/api/agent-extensions/usage/pools",
            get(routes::agent_extensions::pools::snapshot),
        )
        .route(
            "/api/agent-extensions/usage/pools/reset/{handle}",
            post(routes::agent_extensions::pools::redeem),
        )
        .route(
            "/api/agent-extensions/usage/windows/{provider}",
            get(routes::agent_extensions::quota_windows),
        )
        .route(
            "/api/agent-extensions/usage",
            get(routes::agent_extensions::usage),
        )
        .route(
            "/api/agent-extensions/usage/model-pricing",
            post(routes::agent_extensions::edit_usage_model_pricing),
        )
        .route(
            "/api/agent-extensions/usage/model-prices",
            get(routes::agent_extensions::usage_model_prices)
                .post(routes::agent_extensions::edit_usage_model_prices),
        )
        .route(
            "/api/agent-extensions/usage/price-destinations",
            get(routes::agent_extensions::prices::destinations),
        )
        .route(
            "/api/agent-extensions/usage/model-prices/{destination}",
            get(routes::agent_extensions::prices::read)
                .post(routes::agent_extensions::prices::save),
        )
        .route(
            "/api/agent-extensions/usage/refresh",
            post(routes::agent_extensions::refresh),
        )
        .route("/api/coordination", get(routes::coordination::snapshot))
        .route("/api/coordination/ui", get(routes::coordination::ui))
        .route("/api/coordination/board", get(routes::coordination::board))
        .route(
            "/api/coordination/handoffs",
            get(routes::coordination::work_item_handoffs)
                .post(routes::coordination::prepare_work_item),
        )
        .route(
            "/api/coordination/handoffs/{id}",
            get(routes::coordination::work_item_handoff)
                .delete(routes::coordination::forget_work_item_handoff),
        )
        .route(
            "/api/coordination/handoffs/{id}/start",
            post(routes::coordination::start_work_item),
        )
        .route(
            "/api/coordination/commands",
            post(routes::coordination::command),
        )
        .route("/api/worktrees", get(routes::worktrees::list))
        .route("/api/worktrees/ui", get(routes::worktrees::ui))
        .route("/api/worktrees/measure", get(routes::worktrees::measure))
        .route("/api/worktrees/remove", post(routes::worktrees::remove))
        .route(
            "/api/worktrees/cleanup-deleted-scratch",
            post(routes::worktrees::clean_deleted_scratch),
        )
        .route(
            "/api/worktrees/cleanup-deleted-thread",
            post(routes::worktrees::clean_deleted_thread),
        )
        .route(
            "/api/worktrees/cleanup-deleted-thread/with-file-policy",
            post(routes::worktrees::clean_deleted_thread),
        )
        .route(
            "/api/worktrees/start-again",
            post(routes::worktrees::start_again),
        )
        .route("/api/worktrees/forget", post(routes::worktrees::forget))
        .route(
            "/api/coordination/tasks/{id}/worktree",
            get(routes::task_worktree::get),
        )
        .route(
            "/api/coordination/wait",
            post(routes::coordination::wait_for_child),
        )
        .route(
            "/api/coordination/changes",
            get(routes::coordination::changes),
        )
        .route(
            "/api/coordination/access",
            get(routes::coordination::access_list).post(routes::coordination::issue_access),
        )
        .route(
            "/api/coordination/access/{id}",
            delete(routes::coordination::revoke_access),
        )
        .route("/api/local-tools", get(routes::local_tools::snapshot))
        .route("/api/local-tools/ui", get(routes::local_tools::ui))
        .route(
            "/api/voice/dictionaries",
            post(routes::local_tools::dictionary),
        )
        .route("/api/local-tools/grants", post(routes::local_tools::grant))
        .route(
            "/api/local-tools/grants/{id}",
            delete(routes::local_tools::revoke),
        )
        .route(
            "/api/local-tools/browser/start",
            post(routes::local_tools::browser_start),
        )
        .route(
            "/api/local-tools/browser/command",
            post(routes::local_tools::browser_command),
        )
        .route(
            "/api/local-tools/devices",
            get(routes::local_tools::devices),
        )
        .route(
            "/api/local-tools/devices/command",
            post(routes::local_tools::device_command),
        )
        .route(
            "/api/local-tools/devices/video",
            get(routes::local_tools::device_video),
        )
        .route(
            "/api/local-tools/devices/simlock",
            get(routes::environment_parity::simlock_snapshot),
        )
        .route(
            "/api/local-tools/devices/policy",
            put(routes::environment_parity::simlock_set_policy),
        )
        .route(
            "/api/local-tools/devices/simlock/hosts",
            post(routes::environment_parity::simlock_upsert_host),
        )
        .route(
            "/api/local-tools/devices/simlock/hosts/{id}",
            delete(routes::environment_parity::simlock_remove_host),
        )
        .route(
            "/api/local-tools/devices/simlock/hosts/{id}/probe",
            post(routes::environment_parity::simlock_probe_host),
        )
        .route(
            "/api/local-tools/devices/simlock/leases",
            post(routes::environment_parity::simlock_acquire_lease),
        )
        .route(
            "/api/local-tools/devices/simlock/leases/{id}",
            delete(routes::environment_parity::simlock_release_lease),
        )
        .route(
            "/api/relay",
            get(routes::environment_parity::relay_snapshot)
                .put(routes::environment_parity::relay_update),
        )
        .route(
            "/api/relay/webhooks/hold",
            post(routes::environment_parity::relay_hold_webhook),
        )
        .route(
            "/api/relay/webhooks/flush",
            post(routes::environment_parity::relay_flush_webhooks),
        )
        .route(
            "/api/environment/wsl",
            get(routes::environment_parity::wsl_status)
                .put(routes::environment_parity::wsl_update_config),
        )
        .route(
            "/api/environment/wsl/control",
            post(routes::environment_parity::wsl_control),
        )
        .route(
            "/api/local-tools/browser/{session}/screencast",
            get(routes::computer::screencast),
        )
        .route(
            "/api/local-tools/dictation",
            post(routes::local_tools::dictation).layer(DefaultBodyLimit::max(UPLOAD_BODY_BYTES)),
        )
        .route(
            "/api/service",
            get(routes::reactive::service_status).post(routes::reactive::service_apply),
        )
        .route("/api/reactive", get(routes::reactive::snapshot))
        .route("/api/reactive/commands", post(routes::reactive::command))
        .route(
            "/api/reactive/task-models",
            get(routes::reactive::task_models),
        )
        .route(
            "/api/reactive/task-threads",
            get(routes::reactive::task_threads),
        )
        .route(
            "/api/reactive/desktop-task-turns",
            get(routes::reactive::desktop_task_turns),
        )
        .route("/api/reactive/stream", get(routes::reactive::stream))
        .route(
            "/api/reactive/tasks/{task}/deliveries",
            get(routes::reactive::webhook_deliveries),
        )
        .route(
            "/api/reactive/tasks/{task}/deliveries/{delivery}",
            get(routes::reactive::webhook_delivery),
        )
        .route(
            "/api/reactive/agents/{id}/runs",
            get(routes::reactive::agent_runs),
        )
        .route(
            "/api/reactive/agents/{id}/computer",
            get(routes::computer::state).post(routes::computer::request),
        )
        .route(
            "/api/reactive/agents/{id}/computer/command",
            post(routes::computer::command),
        )
        .route(
            "/api/reactive/agents/{id}/apps",
            post(routes::bruh_apps::call)
                .layer(DefaultBodyLimit::max(routes::apps::MAX_BODY_BYTES)),
        )
        .route(
            "/api/reactive/rooms/{id}/messages",
            get(routes::reactive::room_messages),
        )
        .route(
            "/api/databases",
            get(routes::database::list).post(routes::database::save),
        )
        .route("/api/databases/grant", post(routes::database::grant))
        .route(
            "/api/api-client/collection",
            get(routes::api_client::collection).post(routes::api_client::save),
        )
        .route("/api/api-client/send", post(routes::api_client::send))
        .route(
            "/api/whiteboards/source-fingerprint",
            post(routes::whiteboard::fingerprint),
        )
        .route(
            "/api/api-client/credential",
            post(routes::api_client::credential),
        )
        .route(
            "/api/api-client/cancel/{id}",
            post(routes::api_client::cancel),
        )
        .route(
            "/api/api-client/proposals",
            get(routes::api_client::proposals),
        )
        .route("/api/api-client/decision", post(routes::api_client::decide))
        .route(
            "/api/api-client/agent/propose",
            post(routes::api_client::propose),
        )
        .route(
            "/api/api-client/agent/proposals/{id}",
            get(routes::api_client::result),
        )
        .route("/api/databases/query", post(routes::database::query))
        .route("/api/databases/browse", post(routes::database::browse))
        .route("/api/databases/describe", post(routes::database::describe))
        .route(
            "/api/databases/agent/describe",
            post(routes::database::agent_describe),
        )
        .route("/api/databases/cancel/{id}", post(routes::database::cancel))
        .route("/api/databases/{id}", delete(routes::database::remove))
        .route("/api/databases/{id}/tables", get(routes::database::tables))
        .route("/api/databases/agent", get(routes::database::agent_list))
        .route(
            "/api/databases/agent/query",
            post(routes::database::agent_query),
        )
        .route(
            "/api/databases/agent/{id}/tables",
            get(routes::database::agent_tables),
        )
        .route("/api/weekly-reports", get(routes::reactive::reports))
        .route(
            "/api/weekly-reports/commands",
            post(routes::reactive::report_command),
        )
        .route("/api/weekly-reports/{id}", get(routes::reactive::report))
        .route(
            "/api/reactive/sources/{id}/token",
            get(routes::reactive::token),
        )
        .route(
            "/api/reactive/tasks/{id}/webhook-url",
            get(routes::reactive::task_token).post(routes::reactive::rotate_task_token),
        )
        .route(
            "/api/reactive/task-hooks/{id}/{token}",
            post(routes::reactive::hook).layer(DefaultBodyLimit::max(
                bruhs_models::reactive::MAX_PAYLOAD_BYTES,
            )),
        )
        .route(
            "/api/reactive/hooks/{id}",
            post(routes::reactive::hook).layer(DefaultBodyLimit::max(
                bruhs_models::reactive::MAX_PAYLOAD_BYTES,
            )),
        )
        .route("/api/agent", post(routes::agent::post_agent))
        .route(
            "/api/agent/prompt-suggestion",
            post(routes::prompt_suggestion::post_suggestion)
                .layer(DefaultBodyLimit::max(96 * 1024)),
        )
        .route(
            "/api/agent/meeting-suggestions",
            post(routes::meeting_suggestions::post_round).layer(DefaultBodyLimit::max(64 * 1024)),
        )
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
        .route(
            "/api/agent/models/{provider}",
            get(routes::agent_auth::get_models_for_provider),
        )
        .route("/api/health", get(routes::health::get_health))
        .route("/api/updates", get(routes::updates::get_updates))
        .route(
            "/api/updates/providers/{provider}",
            post(routes::updates::post_provider_update),
        )
        .route("/api/agent/approve", post(routes::approve::post_approve))
        .route(
            "/api/agent/answer-question",
            post(routes::approve::post_answer_question),
        )
        .route(
            "/api/agent/question-files",
            post(routes::approve::post_question_file)
                .layer(DefaultBodyLimit::max(UPLOAD_BODY_BYTES))
                .delete(routes::approve::delete_question_file),
        )
        .route("/api/agent/secret", post(routes::secrets::submit))
        .route(
            bruhs_models::logic::attachments::RETIRE_QUESTION_FILES_PATH,
            post(routes::approve::retire_question_files),
        )
        .route(
            "/api/agent/thread-reply",
            post(routes::agent::post_thread_reply),
        )
        .merge(routes::backup::router())
        .route(
            "/api/vault",
            get(routes::vault::list)
                .post(routes::vault::create)
                .layer(DefaultBodyLimit::max(routes::vault::MAX_BODY_BYTES)),
        )
        .route(
            "/api/vault/{id}",
            axum::routing::put(routes::vault::update)
                .delete(routes::vault::delete)
                .layer(DefaultBodyLimit::max(routes::vault::MAX_BODY_BYTES)),
        )
        .route("/api/vault/{id}/check", post(routes::vault::check))
        .route("/api/apps", get(routes::apps::list))
        .route(
            "/api/apps/accounts",
            post(routes::apps::connect).layer(DefaultBodyLimit::max(routes::apps::MAX_BODY_BYTES)),
        )
        .route(
            "/api/apps/accounts/{id}",
            axum::routing::put(routes::apps::replace)
                .layer(DefaultBodyLimit::max(routes::apps::MAX_BODY_BYTES)),
        )
        .route("/api/apps/accounts/{id}/test", post(routes::apps::test))
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
            "/api/open-in-editor",
            post(routes::editor::post_open_in_editor),
        )
        .route(
            "/api/open-in-terminal",
            post(routes::editor::post_open_in_terminal),
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
        .merge(routes::import::router())
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
        .route(
            "/api/maintenance/browser-captures",
            get(routes::uploads::capture_retention).post(routes::uploads::set_capture_retention),
        )
        .route(
            "/api/maintenance/browser-captures/prune",
            post(routes::uploads::prune_captures),
        )
        .route(
            "/api/agent/artifacts/{id}",
            get(routes::artifacts::get_artifact),
        )
        .route(
            "/api/agent/artifacts/{id}/preview",
            get(routes::artifacts::get_preview),
        )
        .route(
            "/api/agent/artifacts/{id}/diagnostics",
            get(routes::artifacts::get_diagnostics).post(routes::artifacts::post_diagnostics),
        )
        .route("/api/agent/media", get(routes::rich_response::get_media))
        .route(
            "/api/agent/rich-response/remote-media",
            post(routes::rich_response::post_remote_media),
        )
        .route(
            "/api/agent/rich-response/state",
            get(routes::rich_response::get_state).put(routes::rich_response::put_state),
        )
        .route(
            "/api/agent/rich-response/media",
            post(routes::rich_response::post_capture),
        )
        .route(
            "/api/agent/rich-response/asset/{name}",
            get(routes::rich_response::get_asset),
        )
        .route(
            "/api/agent/rich-response/export",
            post(routes::rich_response::post_export)
                .layer(DefaultBodyLimit::max(UPLOAD_BODY_BYTES)),
        )
        .route(
            "/api/agent/rich-response/download",
            post(routes::rich_response::post_download),
        )
        .route(
            "/api/agent/rich-response",
            get(routes::rich_response::get_host),
        )
        .route(
            "/api/agent/rich-media",
            get(routes::rich_response::get_media_host),
        )
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
        .route(
            "/api/agent/prompt-repository",
            post(routes::git::prompt_repository_matches),
        )
        .route(
            "/api/agent/prompt-share-repository",
            post(routes::git::prompt_share_repository),
        )
        .route(
            "/api/agent/prompt-pull-checkout",
            post(routes::git::post_pull_checkout),
        )
        .route(
            "/api/agent/prompt-clone",
            post(routes::git::post_prompt_clone),
        )
        .route("/api/agent/git-diff", post(routes::git::get_diff))
        .route("/api/agent/list-dir", post(routes::list_dir::post_list_dir))
        .route(
            "/api/agent/validate-path",
            post(routes::validate_path::post_validate),
        )
        .route(
            bruhs_models::workspace_trust::PATH,
            get(routes::workspace_trust::get_trusted)
                .post(routes::workspace_trust::post_inspect)
                .put(routes::workspace_trust::put_trust)
                .delete(routes::workspace_trust::delete_trusted),
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
        .route("/api/tasks/{id}/stop", post(routes::tasks::stop_task))
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
            get(routes::people::get_person)
                .patch(routes::people::patch_person)
                .delete(routes::people::delete_person),
        )
        .route(
            "/api/people/{id}/restore",
            post(routes::people::restore_person),
        )
        .route(
            "/api/people/{id}/unmerge",
            post(routes::people::unmerge_person),
        )
        .route(
            "/api/people/{id}/aliases",
            axum::routing::put(routes::people::set_person_aliases),
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
            capture_daemon,
        )));
    // The MCP workspace relay to a web tab (`workspace_relay`). The live
    // events stream above reaches it through the same extension.
    #[cfg(unix)]
    let router = router
        .route(
            workspace_relay::SESSION_PATH,
            post(workspace_relay::enable_session).delete(workspace_relay::disable_session),
        )
        .route(
            workspace_relay::REPLY_PATH,
            post(workspace_relay::post_reply)
                .layer(DefaultBodyLimit::max(workspace_relay::MAX_REPLY_BODY_BYTES)),
        )
        .route(
            workspace_relay::CALL_PATH,
            post(workspace_relay::post_call).layer(DefaultBodyLimit::max(
                bruhs_models::workspace_bridge::MAX_REQUEST_BYTES,
            )),
        )
        .layer(axum::Extension(relay));
    router
        .fallback_service(spa)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(
            access,
            local_access::guard,
        ))
        .layer(TraceLayer::new_for_http().make_span_with(|req: &axum::extract::Request| {
            // Route templates keep capability URLs and sensitive query strings out of logs.
            let route = req.extensions().get::<axum::extract::MatchedPath>()
                .map_or("<unmatched>", |path| path.as_str());
            tracing::debug_span!("request", method = %req.method(), route, version = ?req.version())
        }))
        .with_state(state)
}
