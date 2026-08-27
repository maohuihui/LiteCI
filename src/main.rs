use std::net::SocketAddr;

use autoci::{Config, app, connect, migrate, prepare_storage};
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "autoci=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    prepare_storage(&config.storage)?;
    let pool = connect(&config.database.url).await?;
    migrate(&pool).await?;
    let address = SocketAddr::from((config.server.host, config.server.port));
    let listener = TcpListener::bind(address).await?;
    tracing::info!(%address, "autoci server started");
    axum::serve(listener, app()).await?;
    Ok(())
}
