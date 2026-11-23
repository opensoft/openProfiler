mod renewal;

use opensoft_open_profiler_account_status::{
    auth::{Auth, IdentityConfig},
    http,
    store::{Engine, Store},
};
use std::{env, net::SocketAddr};
use tokio::fs;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    let listen: SocketAddr = env::var("STATUS_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .map_err(|_| "invalid listener configuration")?;
    let fixture = env::var("STATUS_LOCAL_FIXTURE").as_deref() == Ok("1");
    if fixture && !listen.ip().is_loopback() {
        return Err("fixture mode requires loopback listener");
    }
    let engine =
        Engine::parse(&env::var("STATUS_DB_ENGINE").map_err(|_| "missing database engine")?)
            .ok_or("invalid database engine")?;
    let database_file = env::var_os("STATUS_DB_URL_FILE").ok_or("missing database secret file")?;
    let database = fs::read_to_string(database_file)
        .await
        .map_err(|_| "database secret unavailable")?;
    let store = Store::connect(engine, database.trim(), fixture)
        .await
        .map_err(|_| "database writer unavailable")?;
    store
        .migrate()
        .await
        .map_err(|_| "database schema unavailable")?;
    let identity =
        env::var_os("STATUS_IDENTITY_FILE").ok_or("missing identity configuration file")?;
    let file = fs::read(identity)
        .await
        .map_err(|_| "identity configuration unavailable")?;
    let config: IdentityConfig =
        serde_json::from_slice(&file).map_err(|_| "invalid identity configuration")?;
    let auth = Auth::connect(config.clone(), fixture).await?;
    auth.refresh_in_background();
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|_| "listener unavailable")?;
    renewal::start_if_configured(&store, &config)?;
    axum::serve(listener, http::router(store, auth))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|_| "service stopped")
}

async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = terminate.recv() => {},
            _ = tokio::signal::ctrl_c() => {},
        }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
