/// A simple proxy that forwards requests to a given URL with a custom User-Agent.
use std::{collections::HashMap, env, net::SocketAddr, sync::OnceLock};

use anyhow::{anyhow, Result};
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_auth::AuthBearer;
use clap::Parser;
use reqwest::{header::HeaderValue, Client};
use tower::ServiceBuilder;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static AUTH_TOKEN: OnceLock<String> = OnceLock::new();

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(short, long)]
    user_agent: Option<String>,
}

#[derive(Clone)]
struct AppState {
    client: Client,
}

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

    dotenvy::dotenv().ok();
    let port = env::var("PORT").unwrap_or("7788".to_string());
    let auth_token = env::var("AUTH_TOKEN")?;
    AUTH_TOKEN
        .set(auth_token)
        .map_err(|_| anyhow!("Auth token could not be set"))?;

    let cli = Cli::parse();
    let user_agent = cli.user_agent.unwrap_or("Instagram 310.0.0.37.328 Android (31/12; 440dpi; 1080x2180; Xiaomi; M2007J3SG; apollo; qcom; de_DE; 543594164)".to_string());
    let client = Client::builder().user_agent(user_agent).build()?;
    let app_state = AppState { client };

    let compression_service = ServiceBuilder::new().layer(CompressionLayer::new());

    let app = Router::new()
        .route("/", get(handler))
        .fallback(handler_404)
        .layer(TraceLayer::new_for_http())
        .layer(compression_service)
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn handler(
    AuthBearer(token): AuthBearer,
    Query(params): Query<HashMap<String, String>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    if &token != AUTH_TOKEN.get().unwrap() {
        tracing::error!(peer = addr.to_string(), "Unauthorized access attempt");
        return Ok((
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            Bytes::from_static(b"Unauthorized"),
        ));
    }
    let Some(url) = params.get("url") else {
        tracing::error!(peer = addr.to_string(), "Missing `url` param");
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Bytes::from_static(b"Missing `url` param"),
        ));
    };
    let request = state.client.get(url).send().await?;
    match request.status() {
        reqwest::StatusCode::OK => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                request
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .unwrap_or(&HeaderValue::from_str("text/plain")?)
                    .to_str()?
                    .parse()?,
            );
            let body = request.bytes().await?;
            tracing::info!(data_len = body.len(), "Proxied request");
            Ok((StatusCode::OK, headers, body))
        }
        status => {
            let status_code = status.as_u16();
            let body = request.bytes().await?;
            tracing::error!(status_code, "Error during proxy request");
            Ok((StatusCode::from_u16(status_code)?, HeaderMap::new(), body))
        }
    }
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
