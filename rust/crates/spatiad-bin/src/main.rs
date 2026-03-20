use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::Context;
use spatiad_api::{router, start_background_tasks, ApiState, SlidingWindowRateLimiter, WsReconnectGuard};
use spatiad_core::Engine;
use spatiad_dispatch::DispatchService;
use spatiad_storage::StorageBackend;
use spatiad_types::{Coordinates, DriverStatus};
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let log_level = std::env::var("SPATIAD_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();

    let h3_resolution = std::env::var("SPATIAD_H3_RESOLUTION")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(8);

    let storage_backend = std::env::var("SPATIAD_STORAGE_BACKEND")
        .unwrap_or_else(|_| "memory".to_string());

    let engine = match storage_backend.as_str() {
        "sqlite" => {
            let path = std::env::var("SPATIAD_SQLITE_PATH")
                .unwrap_or_else(|_| "spatiad.db".to_string());
            info!(path = %path, "opening SQLite storage backend");
            let backend = spatiad_storage::SqliteBackend::open(&path)
                .context("failed to open SQLite database")?;
            let storage: Box<dyn StorageBackend> = Box::new(backend);
            Engine::recover(h3_resolution, storage)
                .map_err(|e| anyhow::anyhow!("failed to recover engine state: {e}"))?
        }
        "postgres" => {
            let url = std::env::var("SPATIAD_POSTGRES_URL")
                .unwrap_or_else(|_| "host=localhost dbname=spatiad".to_string());
            info!(url = %url, "opening PostgreSQL storage backend");
            let backend = spatiad_storage::PostgresBackend::open(&url)
                .context("failed to connect to PostgreSQL")?;
            let storage: Box<dyn StorageBackend> = Box::new(backend);
            Engine::recover(h3_resolution, storage)
                .map_err(|e| anyhow::anyhow!("failed to recover engine state: {e}"))?
        }
        _ => {
            if storage_backend != "memory" {
                warn!(backend = %storage_backend, "unknown storage backend, falling back to in-memory");
            }
            let mut engine = Engine::new(h3_resolution);
            // Seed one driver for immediate manual tests against /dispatch/offer.
            engine.upsert_driver_location(
                Uuid::parse_str("11111111-1111-1111-1111-111111111111")?,
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
            engine
        }
    };

    let dispatch_rate_limit_per_min = std::env::var("SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(240);
    let ws_reconnect_max_per_min = std::env::var("SPATIAD_WS_RECONNECT_MAX_PER_MIN")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(30);
    let webhook_timeout_ms = std::env::var("SPATIAD_WEBHOOK_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(100, 60_000))
        .unwrap_or(3_000);

    let snapshot_interval_secs = std::env::var("SPATIAD_SNAPSHOT_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);

    let state = ApiState {
        dispatch: Arc::new(Mutex::new(DispatchService::new(engine))),
        webhook_url: std::env::var("SPATIAD_WEBHOOK_URL").ok(),
        webhook_secret: std::env::var("SPATIAD_WEBHOOK_SECRET").ok(),
        webhook_timeout_ms,
        driver_token: std::env::var("SPATIAD_DRIVER_TOKEN").ok(),
        dispatcher_token: std::env::var("SPATIAD_DISPATCHER_TOKEN").ok(),
        dispatch_rate_limiter: Arc::new(Mutex::new(SlidingWindowRateLimiter::new(
            dispatch_rate_limit_per_min,
            60,
        ))),
        ws_reconnect_guard: Arc::new(Mutex::new(WsReconnectGuard::new(
            ws_reconnect_max_per_min,
            60,
        ))),
        sessions: Arc::new(Mutex::new(HashMap::new())),
    };

    start_background_tasks(state.clone());

    // Start snapshot background task when using persistent storage.
    if storage_backend == "sqlite" || storage_backend == "postgres" {
        let dispatch = state.dispatch.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                tokio::time::Duration::from_secs(snapshot_interval_secs),
            );
            // Skip the immediate first tick.
            interval.tick().await;
            loop {
                interval.tick().await;
                let service = dispatch.lock().await;
                if let Err(e) = service.engine.create_snapshot() {
                    warn!(error = %e, "periodic snapshot failed");
                } else {
                    info!("periodic snapshot created");
                }
            }
        });
    }

    let app = router(state);

    let bind_addr = std::env::var("SPATIAD_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let addr: SocketAddr = bind_addr
        .parse()
        .context("invalid bind address")?;

    info!(%addr, %storage_backend, webhook_timeout_ms, snapshot_interval_secs, "starting spatiad API");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            warn!(%error, "failed to listen for ctrl_c signal");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                warn!(%error, "failed to listen for terminate signal");
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received");
}
