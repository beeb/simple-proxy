use std::{collections::HashMap, env};

use anyhow::Result;
use axum::{extract::Query, http::StatusCode, response::IntoResponse, routing::get, Router};
use tower::ServiceBuilder;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "simple_proxy=debug,tower_http=debug,axum::rejection=trace".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let _ = dotenvy::dotenv();
    let port = env::var("PORT").unwrap_or("7788".to_string());

    let compression_service = ServiceBuilder::new().layer(CompressionLayer::new());

    let app = Router::new()
        .route("/", get(handler))
        .layer(TraceLayer::new_for_http())
        .layer(compression_service);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handler(Query(params): Query<HashMap<String, String>>) -> Result<&'static str, AppError> {
    Ok("hi")
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
