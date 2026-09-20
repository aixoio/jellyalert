#![forbid(unsafe_code)]

use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use server_core::{
    api, cli::Cli, config::ServerConfig, database::Database, discord::Discord, service::Service,
    sonarr::Sonarr,
};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = ServerConfig::read_from_path(&cli.config_path)?;
    let db = Database::connect(&config.sqlite_database_path).await?;
    let service = Service::new(
        db.clone(),
        Sonarr::new(&config.sonarr_url, &config.sonarr_api_key)?,
        Discord::new(&config.discord_webhook_url)?,
    );
    // Bind before starting workers so an accidental second process on the same address fails early.
    let listener = TcpListener::bind(config.listen_address)
        .await
        .context("cannot bind API listener")?;
    let (shutdown, receiver) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let scanner = service.clone();
    let scan_shutdown = receiver.clone();
    tasks.spawn(async move {
        scanner
            .scan_loop(
                Duration::from_secs(config.scan_interval_seconds),
                scan_shutdown,
            )
            .await;
        Ok::<_, anyhow::Error>(())
    });
    let worker = service.clone();
    let delivery_shutdown = receiver.clone();
    tasks.spawn(async move {
        worker.delivery_loop(delivery_shutdown).await;
        Ok(())
    });
    let mut api_shutdown = receiver;
    tasks.spawn(async move {
        axum::serve(listener, api::router(service))
            .with_graceful_shutdown(async move {
                if !*api_shutdown.borrow() {
                    let _ = api_shutdown.changed().await;
                }
            })
            .await?;
        Ok(())
    });
    eprintln!("Jelly Alert listening on {}", config.listen_address);
    let result = tokio::select! {
        result = shutdown_signal() => result,
        result = tasks.join_next() => match result {
            Some(Ok(Err(error))) => Err(error),
            Some(Err(error)) => Err(error.into()),
            _ => Err(anyhow::anyhow!("a backend worker exited unexpectedly")),
        }
    };
    let _ = shutdown.send(true);
    // Let an in-flight webhook finish and persist its result before closing SQLite.
    let drain = async {
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                eprintln!("Worker failed during shutdown: {error}");
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(45), drain)
        .await
        .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    db.pool().close().await;
    result
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
