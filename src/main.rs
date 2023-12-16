use std::{collections::HashMap, env, sync::OnceLock};

use anyhow::{anyhow, Result};
use axum::{extract::Query, http::StatusCode, response::IntoResponse, routing::get, Router};
use axum_auth::AuthBearer;
use tower::ServiceBuilder;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static AUTH_TOKEN: OnceLock<String> = OnceLock::new();

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
    let auth_token = env::var("AUTH_TOKEN")?;
    AUTH_TOKEN
        .set(auth_token)
        .map_err(|_| anyhow!("Auth token could not be set"))?;

    let compression_service = ServiceBuilder::new().layer(CompressionLayer::new());

    let app = Router::new()
        .route("/", get(handler))
        .fallback(handler_404)
        .layer(TraceLayer::new_for_http())
        .layer(compression_service);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handler(
    AuthBearer(token): AuthBearer,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    if &token != AUTH_TOKEN.get().unwrap() {
        return Ok((StatusCode::UNAUTHORIZED, "unauthorized".to_string()));
    }
    Ok((StatusCode::OK, params.get("url").unwrap().to_string()))
}

async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "nothing to see here")
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