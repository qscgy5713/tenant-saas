use std::time::Duration;

use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use tenant_saas::{config::Config, mail::Mailer, routes, worker};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env()?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    // 背景 worker:寄信、提醒、整理過期預約;收到關閉訊號時等它收尾
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let worker_handle = if config.worker_enabled {
        let mailer = Mailer::from_config(&config)?;
        Some(tokio::spawn(worker::run(
            db.clone(),
            mailer,
            Duration::from_secs(config.worker_poll_secs),
            shutdown_rx,
        )))
    } else {
        tracing::warn!("WORKER_ENABLED=false:不會寄信,也不會整理過期預約");
        None
    };

    let app = routes::router(routes::AppState::new(db, &config));
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("監聽 {}", config.bind_addr);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    let _ = shutdown_tx.send(true);
    if let Some(handle) = worker_handle {
        let _ = handle.await;
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("無法監聽 Ctrl+C");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("無法監聽 SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("收到關閉訊號,開始優雅關閉");
}
