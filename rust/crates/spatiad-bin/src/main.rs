use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use spatiad_api::{router, ApiState};
use spatiad_core::Engine;
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, DriverStatus};
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let mut engine = Engine::new(8);

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
    };

    let app = router(state);

    let addr: SocketAddr = "0.0.0.0:3000"
        .parse()
        .context("invalid bind address")?;

    info!(%addr, "starting spatiad API");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
