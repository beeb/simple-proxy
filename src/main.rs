/// A simple proxy that forwards requests to a given URL with a custom User-Agent.
use std::{convert::Infallible, env, time::Duration};

use anyhow::{Context as _, Result};
use rama::{
    http::{
        client::HttpClient,
        layer::{
            remove_header::{RemoveRequestHeaderLayer, RemoveResponseHeaderLayer},
            set_header::SetRequestHeaderLayer,
            trace::TraceLayer,
            validate_request::ValidateRequestHeaderLayer,
        },
        server::HttpServer,
        Body, HeaderName, HeaderValue, Request, Response, StatusCode,
    },
    rt::Executor,
    service::{Context, Service as _, ServiceBuilder},
    stream::layer::http::BodyLimitLayer,
    tcp::server::TcpListener,
};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::DEBUG.into())
                .from_env_lossy(),
        )
        .init();

    dotenvy::dotenv().ok();
    let port = env::var("PORT").unwrap_or("7788".to_string());
    let auth_token = env::var("PROXY_AUTH_TOKEN").context("PROXY_AUTH_TOKEN not provided")?;

    let user_agent = env::var("PROXY_USER_AGENT").unwrap_or("Instagram 310.0.0.37.328 Android (31/12; 440dpi; 1080x2180; Xiaomi; M2007J3SG; apollo; qcom; de_DE; 543594164)".to_string());

    let graceful = rama::utils::graceful::Shutdown::default();

    graceful.spawn_task_fn(|guard| async move {
        let tcp_service = TcpListener::build()
            .bind(format!("0.0.0.0:{port}"))
            .await
            .expect("bind on port");

        let exec = Executor::graceful(guard.clone());
        let http_service = HttpServer::auto(exec).service(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(ValidateRequestHeaderLayer::bearer(&auth_token))
                .layer(RemoveRequestHeaderLayer::exact("Authorization"))
                // .layer(SetRequestHeaderLayer::overriding(
                //     HeaderName::from_static("user-agent"),
                //     HeaderValue::from_str(&user_agent),
                // ))
                .layer(RemoveRequestHeaderLayer::hop_by_hop())
                .layer(RemoveResponseHeaderLayer::hop_by_hop())
                .service_fn(http_plain_proxy),
        );

        tcp_service
            .serve_graceful(
                guard,
                ServiceBuilder::new()
                    .layer(BodyLimitLayer::symmetric(2 * 1024 * 1024))
                    .service(http_service),
            )
            .await;
    });

    graceful
        .shutdown_with_limit(Duration::from_secs(30))
        .await?;

    // let client = Client::builder().user_agent(user_agent).build()?;
    // let app_state = AppState { client };

    // let compression_service = ServiceBuilder::new().layer(CompressionLayer::new());

    // let app = Router::new()
    //     .route("/", get(handler))
    //     .fallback(handler_404)
    //     .layer(TraceLayer::new_for_http())
    //     .layer(compression_service)
    //     .with_state(app_state);

    // let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    // axum::serve(
    //     listener,
    //     app.into_make_service_with_connect_info::<SocketAddr>(),
    // )
    // .await?;
    Ok(())
}

async fn http_plain_proxy<S>(ctx: Context<S>, req: Request) -> Result<Response, Infallible>
where
    S: Send + Sync + 'static,
{
    let client = HttpClient::default();
    match client.serve(ctx, req).await {
        Ok(resp) => Ok(resp),
        Err(err) => {
            tracing::error!(error = %err, "error in client request");
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap())
        }
    }
}

// async fn handler(
//     AuthBearer(token): AuthBearer,
//     Query(params): Query<HashMap<String, String>>,
//     ConnectInfo(addr): ConnectInfo<SocketAddr>,
//     State(state): State<AppState>,
// ) -> Result<impl IntoResponse, AppError> {
//     if &token != AUTH_TOKEN.get().unwrap() {
//         tracing::error!(peer = addr.to_string(), "Unauthorized access attempt");
//         return Ok((
//             StatusCode::UNAUTHORIZED,
//             HeaderMap::new(),
//             Bytes::from_static(b"Unauthorized"),
//         ));
//     }
//     let Some(url) = params.get("url") else {
//         tracing::error!(peer = addr.to_string(), "Missing `url` param");
//         return Ok((
//             StatusCode::BAD_REQUEST,
//             HeaderMap::new(),
//             Bytes::from_static(b"Missing `url` param"),
//         ));
//     };
//     let request = state.client.get(url).send().await?;
//     match request.status() {
//         reqwest::StatusCode::OK => {
//             let mut headers = HeaderMap::new();
//             headers.insert(
//                 header::CONTENT_TYPE,
//                 request
//                     .headers()
//                     .get(reqwest::header::CONTENT_TYPE)
//                     .unwrap_or(&HeaderValue::from_str("text/plain")?)
//                     .to_str()?
//                     .parse()?,
//             );
//             let body = request.bytes().await?;
//             tracing::info!(data_len = body.len(), "Proxied request");
//             Ok((StatusCode::OK, headers, body))
//         }
//         status => {
//             let status_code = status.as_u16();
//             let body = request.bytes().await?;
//             tracing::error!(status_code, "Error during proxy request");
//             Ok((StatusCode::from_u16(status_code)?, HeaderMap::new(), body))
//         }
//     }
// }

// async fn handler_404() -> impl IntoResponse {
//     (StatusCode::NOT_FOUND, "nothing to see here")
// }

// struct AppError(anyhow::Error);

// impl IntoResponse for AppError {
//     fn into_response(self) -> axum::response::Response {
//         (
//             StatusCode::INTERNAL_SERVER_ERROR,
//             format!("Something went wrong: {}", self.0),
//         )
//             .into_response()
//     }
// }

// impl<E> From<E> for AppError
// where
//     E: Into<anyhow::Error>,
// {
//     fn from(err: E) -> Self {
//         Self(err.into())
//     }
// }
