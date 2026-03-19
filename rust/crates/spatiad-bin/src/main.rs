use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::Context;
use spatiad_api::{router, start_background_tasks, ApiState, SlidingWindowRateLimiter, WsReconnectGuard};
use spatiad_core::Engine;
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, DriverStatus};
use tokio::sync::Mutex;
use tracing::info;
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

    let mut engine = Engine::new(h3_resolution);

    let dispatch_rate_limit_per_min = std::env::var("SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(240);
    let ws_reconnect_max_per_min = std::env::var("SPATIAD_WS_RECONNECT_MAX_PER_MIN")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(30);

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

    let state = ApiState {
        dispatch: Arc::new(Mutex::new(DispatchService::new(engine))),
        webhook_url: std::env::var("SPATIAD_WEBHOOK_URL").ok(),
        webhook_secret: std::env::var("SPATIAD_WEBHOOK_SECRET").ok(),
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

    let app = router(state);

    let bind_addr = std::env::var("SPATIAD_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let addr: SocketAddr = bind_addr
        .parse()
        .context("invalid bind address")?;

    info!(%addr, "starting spatiad API");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
