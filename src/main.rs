mod browser;
mod error;
mod handlers;
mod llm;
mod models;

use axum::{routing::{get, post}, Router};
use std::sync::Arc;
use tokio::signal;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::browser::BrowserManager;
use crate::handlers::{health_handler, scrape_handler, AppState};
use crate::llm::GeminiClient;

const DEFAULT_PORT: u16 = 3000;
const DEFAULT_MAX_CONCURRENT_TABS: usize = 50;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "distill=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let api_key = std::env::var("API_KEY").unwrap_or_else(|_| {
        warn!("API_KEY not set, using default");
        "changeme".to_string()
    });

    let max_concurrent_tabs = std::env::var("MAX_CONCURRENT_TABS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENT_TABS);

    let browser = BrowserManager::new(max_concurrent_tabs)?;
    let llm_client = GeminiClient::new();

    info!(
        port = DEFAULT_PORT,
        max_tabs = max_concurrent_tabs,
        gemini = llm_client.is_configured(),
        "Distill starting"
    );

    let state = Arc::new(AppState {
        browser,
        llm_client,
        api_key,
    });

    let app = Router::new()
        .route("/scrape", post(scrape_handler))
        .route("/health", get(health_handler))
        .layer(build_cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!(port, "Server ready");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Shutdown complete");
    Ok(())
}

fn build_cors_layer() -> CorsLayer {
    match std::env::var("ALLOWED_ORIGINS") {
        Ok(origins) if !origins.is_empty() && origins != "*" => {
            let origins: Vec<_> = origins
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            if origins.is_empty() {
                warn!("ALLOWED_ORIGINS invalid, allowing all");
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any)
            } else {
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(origins))
                    .allow_methods(Any)
                    .allow_headers(Any)
            }
        }
        _ => {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C"),
        _ = terminate => info!("Received SIGTERM"),
    }
}
