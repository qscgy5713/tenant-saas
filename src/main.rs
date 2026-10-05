use std::time::Duration;

use anyhow::{Context, Result, bail};
use sqlx::postgres::PgPoolOptions;
use tenant_saas::{config::Config, dbrole, mail::Mailer, routes, worker};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_logging();

    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("migrate") => migrate().await,
        Some("healthcheck") => healthcheck().await,
        Some(other) => bail!("未知的子命令:{other}(可用:serve、migrate、healthcheck)"),
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    if std::env::var("LOG_FORMAT").is_ok_and(|v| v == "json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

/// `tenant-saas migrate`:用有建表權限的帳號套用 migration 後結束。
/// 帳號與執行階段不同:優先讀 MIGRATION_DATABASE_URL,沒有才退回 DATABASE_URL。
async fn migrate() -> Result<()> {
    let url = std::env::var("MIGRATION_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .context("缺少 MIGRATION_DATABASE_URL(或 DATABASE_URL)")?;
    let db = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .context("無法連線到資料庫")?;
    sqlx::migrate!("./migrations").run(&db).await?;
    tracing::info!("migration 完成");
    Ok(())
}

/// `tenant-saas healthcheck`:給容器的 HEALTHCHECK 用。
/// 容器內沒有 curl,這裡直接對本機的 /health 送一個最小的 HTTP 請求。
async fn healthcheck() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3001".into());
    let port = bind.rsplit(':').next().context("BIND_ADDR 格式不正確")?;
    let mut stream = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")),
    )
    .await
    .context("連線逾時")??;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await?;
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut response))
        .await
        .context("讀取逾時")??;
    if response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        bail!("健康檢查未通過:{}", response.lines().next().unwrap_or(""))
    }
}

async fn serve() -> Result<()> {
    let config = Config::from_env()?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&config.database_url)
        .await?;
    if config.auto_migrate {
        sqlx::migrate!("./migrations").run(&db).await?;
    } else {
        tracing::info!(
            "AUTO_MIGRATE 關閉:不在啟動時套用 migration(請先執行 `tenant-saas migrate`)"
        );
    }

    // 連線帳號過大會讓 RLS 形同虛設。正式環境直接拒絕啟動;開發環境(超級使用者)只警告
    let problems = dbrole::problems(&db).await?;
    if config.production && !problems.is_empty() {
        bail!(
            "APP_ENV=production 拒絕啟動,資料庫連線帳號不夠受限:\n  - {}\n參考 docs/deployment.md 建立受限的 runtime 帳號",
            problems.join("\n  - ")
        );
    }
    for problem in &problems {
        tracing::warn!("資料庫連線帳號過大:{problem}(正式環境請設定 APP_ENV=production 強制檢查)");
    }

    // 背景 worker:寄信、提醒、整理過期預約;收到關閉訊號時等它收尾
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let worker_handle = if config.worker_enabled {
        let mailer = Mailer::from_config(&config)?;
        Some(tokio::spawn(worker::run(
            db.clone(),
            mailer,
            config.public_base_url.clone(),
            Duration::from_secs(config.worker_poll_secs),
            shutdown_rx,
        )))
    } else {
        tracing::warn!("WORKER_ENABLED=false:不會寄信,也不會整理過期預約");
        None
    };

    let metrics = metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(&[
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ])?
        .install_recorder()?;
    let app = routes::router(routes::AppState::new(db, &config).with_metrics(metrics));
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
