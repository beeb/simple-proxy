/// A simple proxy that forwards requests to a given URL with a custom User-Agent.
use std::{convert::Infallible, env, str::FromStr, time::Duration};

use anyhow::{Context as _, Result};
use rama::{
    http::{
        client::HttpClient,
        headers::UserAgent,
        layer::{
            proxy_auth::ProxyAuthLayer,
            remove_header::{RemoveRequestHeaderLayer, RemoveResponseHeaderLayer},
            set_header::SetRequestHeaderLayer,
            trace::TraceLayer,
            upgrade::{UpgradeLayer, Upgraded},
        },
        matcher::MethodMatcher,
        server::HttpServer,
        Body, IntoResponse as _, Request, RequestContext, Response, StatusCode,
    },
    rt::Executor,
    service::{service_fn, Context, Service as _, ServiceBuilder},
    stream::layer::http::BodyLimitLayer,
    tcp::{server::TcpListener, utils::is_connection_error},
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
            .context("bind on port")?;
        tracing::info!("listening on port {port}");

        let exec = Executor::graceful(guard.clone());
        let http_service = HttpServer::auto(exec).service(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(ProxyAuthLayer::basic(("proxy".to_string(), auth_token)))
                .layer(SetRequestHeaderLayer::overriding_typed(
                    UserAgent::from_str(&user_agent).context("decoding user agent")?,
                ))
                .layer(UpgradeLayer::new(
                    MethodMatcher::CONNECT,
                    service_fn(http_connect_accept),
                    service_fn(http_connect_proxy),
                ))
                .service(
                    ServiceBuilder::new()
                        .layer(RemoveRequestHeaderLayer::hop_by_hop())
                        .layer(RemoveResponseHeaderLayer::hop_by_hop())
                        .service_fn(http_plain_proxy),
                ),
        );

        tcp_service
            .serve_graceful(
                guard,
                ServiceBuilder::new()
                    .layer(BodyLimitLayer::symmetric(2 * 1024 * 1024))
                    .service(http_service),
            )
            .await;

        Result::<(), anyhow::Error>::Ok(())
    });

    graceful
        .shutdown_with_limit(Duration::from_secs(30))
        .await?;

    Ok(())
}

async fn http_connect_accept<S>(
    mut ctx: Context<S>,
    req: Request,
) -> Result<(Response, Context<S>, Request), Response>
where
    S: Send + Sync + 'static,
{
    tracing::debug!("connect accept: {req:?}");
    match ctx
        .get_or_insert_with::<RequestContext>(|| RequestContext::from(&req))
        .host
        .as_ref()
    {
        Some(host) => tracing::info!("accept CONNECT to {host}"),
        None => {
            tracing::error!("error extracting host");
            return Err(StatusCode::BAD_REQUEST.into_response());
        }
    }

    Ok((StatusCode::OK.into_response(), ctx, req))
}

async fn http_connect_proxy<S>(ctx: Context<S>, mut upgraded: Upgraded) -> Result<(), Infallible>
where
    S: Send + Sync + 'static,
{
    tracing::debug!("connect proxy: {upgraded:?}");
    let host = ctx
        .get::<RequestContext>()
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .clone();
    tracing::info!("CONNECT to {}", host);
    let mut stream = match tokio::net::TcpStream::connect(&host).await {
        Ok(stream) => stream,
        Err(err) => {
            tracing::error!(error = %err, "error connecting to host");
            return Ok(());
        }
    };
    if let Err(err) = tokio::io::copy_bidirectional(&mut upgraded, &mut stream).await {
        if !is_connection_error(&err) {
            tracing::error!(error = %err, "error copying data");
        }
    }
    Ok(())
}

async fn http_plain_proxy<S>(ctx: Context<S>, req: Request) -> Result<Response, Infallible>
where
    S: Send + Sync + 'static,
{
    tracing::debug!("plain proxy: {req:?}");
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
