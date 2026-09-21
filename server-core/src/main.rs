#![forbid(unsafe_code)]

use std::{fs::OpenOptions, path::PathBuf, sync::Mutex, time::Duration};

use anyhow::Context;
use clap::Parser;
use server_core::{
    api,
    cli::Cli,
    config::{LogLevel, ServerConfig},
    database::Database,
    discord::Discord,
    service::Service,
    sonarr::Sonarr,
};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};
use tracing::{Instrument, error, info, info_span, warn};
use tracing_subscriber::{EnvFilter, fmt::writer::MakeWriterExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = ServerConfig::read_from_path(&cli.config_path)?;
    init_tracing(config.log_level)?;
    info!("starting Jelly Alert server core");
    info!(config_path = %cli.config_path.display(), listen_address = %config.listen_address, scan_interval_seconds = config.scan_interval_seconds, log_level = config.log_level.as_str(), "configuration loaded");
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
    tasks.spawn(
        async move {
            scanner
                .scan_loop(
                    Duration::from_secs(config.scan_interval_seconds),
                    scan_shutdown,
                )
                .await;
            Ok::<_, anyhow::Error>(())
        }
        .instrument(info_span!("worker", kind = "scanner")),
    );
    let worker = service.clone();
    let delivery_shutdown = receiver.clone();
    tasks.spawn(
        async move {
            worker.delivery_loop(delivery_shutdown).await;
            Ok(())
        }
        .instrument(info_span!("worker", kind = "delivery")),
    );
    let mut api_shutdown = receiver;
    tasks.spawn(
        async move {
            axum::serve(listener, api::router(service))
                .with_graceful_shutdown(async move {
                    if !*api_shutdown.borrow() {
                        let _ = api_shutdown.changed().await;
                    }
                })
                .await?;
            Ok(())
        }
        .instrument(info_span!("worker", kind = "api")),
    );
    info!(listen_address = %config.listen_address, "Jelly Alert listening");
    let result = tokio::select! {
        result = shutdown_signal() => result,
        result = tasks.join_next() => match result {
            Some(Ok(Err(error))) => Err(error),
            Some(Err(error)) => Err(error.into()),
            _ => Err(anyhow::anyhow!("a backend worker exited unexpectedly")),
        }
    };
    info!("shutdown requested");
    let _ = shutdown.send(true);
    // Let an in-flight webhook finish and persist its result before closing SQLite.
    let drain = async {
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                error!(error = %error, "worker failed during shutdown");
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(45), drain)
        .await
        .is_err()
    {
        warn!("graceful shutdown timed out; aborting remaining workers");
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    db.pool().close().await;
    info!("shutdown complete");
    result
}

fn init_tracing(log_level: LogLevel) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("server_core={}", log_level.as_str())));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true);
    if let Some(directory) = std::env::var_os("LOG_DIRECTORY") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).context("cannot create log directory")?;
        let filename = format!(
            "backend-{}-{}.log",
            chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S%.9fZ"),
            std::process::id()
        );
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join(filename))
            .context("cannot create backend log file")?;
        subscriber
            .with_ansi(false)
            .with_writer(std::io::stdout.and(Mutex::new(file)))
            .init();
    } else {
        subscriber.init();
    }
    Ok(())
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result?;
                info!(signal = "SIGINT", "received shutdown signal");
            },
            _ = terminate.recv() => info!(signal = "SIGTERM", "received shutdown signal"),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
