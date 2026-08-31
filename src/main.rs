use std::net::SocketAddr;

use liteci::{
    Config, CredentialCipher, app_with_setup_token_workspace_and_cipher, connect, migrate,
    prepare_storage,
};
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "liteci=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let setup_token = config
        .setup_token
        .clone()
        .unwrap_or_else(generate_setup_token);
    if config.setup_token.is_none() {
        tracing::warn!(setup_token = %setup_token, "first-run setup token; store it securely and restart to rotate");
    }
    prepare_storage(&config.storage)?;
    let pool = connect(&config.database.url).await?;
    migrate(&pool).await?;
    let address = SocketAddr::from((config.server.host, config.server.port));
    let listener = TcpListener::bind(address).await?;
    tracing::info!(%address, "liteci server started");
    axum::serve(
        listener,
        app_with_setup_token_workspace_and_cipher(
            pool,
            setup_token,
            &config.storage.workspace,
            credential_cipher(&config)?,
        )
        .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn generate_setup_token() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn credential_cipher(config: &Config) -> Result<CredentialCipher, &'static str> {
    let value = config
        .credential_key
        .as_deref()
        .ok_or("LITECI_CREDENTIAL_KEY is required")?;
    let key = hex::decode(value).map_err(|_| "LITECI_CREDENTIAL_KEY must be 64 hex characters")?;
    CredentialCipher::from_key_bytes(&key)
        .map_err(|_| "LITECI_CREDENTIAL_KEY must be 64 hex characters")
}
