use crate::cli::Args;
use crate::state::ProxyState;
use axum::body::Body;
use axum::extract::{Host, State};
use axum::http::Request;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;
use clap::Parser;
use hyper::client::connect::dns::Name;
use hyper::client::HttpConnector;
use hyper::{Client, Server, StatusCode, Uri};
use metrics::{counter, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::convert::Infallible;
use std::future::{ready, Ready};
use std::iter::{once, Once};
use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::Service;
use tracing::level_filters::LevelFilter;

mod cli;

mod state;

async fn fallback(
    Host(hostname): Host,
    State(state): State<ProxyState>,
    mut request: Request<Body>,
) -> Response<Body> {
    let uri = request.uri().clone();

    *request.uri_mut() = Uri::builder()
        .authority(format!("{hostname}:{}", state.remote.port()))
        .scheme("http")
        .path_and_query(uri.path_and_query().unwrap().clone())
        .build()
        .unwrap();

    match state.client.request(request).await {
        Ok(r) => r,
        Err(error) => {
            tracing::error!(message = "Internal error when talking to upstream", ?error);

            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap()
        }
    }
}

async fn observe<B>(request: Request<B>, next: Next<B>) -> Response {
    let uri = request.uri().clone();
    let method = request.method().clone();

    let start = Instant::now();

    let response = next.run(request).await;

    let delta = start.elapsed();

    if response.status().is_client_error() || response.status().is_server_error() {
        counter!(
            "error_responses", 1,
            "status" => response.status().to_string(),
            "method" => method.to_string()
        );

        tracing::error!(
            message = "Serving error",
            "status" = response.status().as_str(),
            "uri" = uri.to_string(),
            "method" = method.as_str(),
            "time" = delta.as_secs_f64()
        )
    } else {
        if response.status().is_redirection() {
            tracing::warn!(
                message = "Serving redirect",
                "status" = response.status().as_str(),
                "uri" = uri.to_string(),
                "method" = method.as_str(),
                "time" = delta.as_secs_f64()
            );

            counter!(
                "redirect_responses", 1,
                "status" => response.status().to_string(),
                "method" => method.to_string()
            );
        }

        counter!(
            "responses", 1,
            "status" => response.status().to_string(),
            "method" => method.to_string()
        );

        tracing::info!(
            message = "Serving response",
            "status" = response.status().as_str(),
            "uri" = uri.to_string(),
            "method" = method.as_str(),
            "time" = delta.as_secs_f64()
        )
    }

    histogram!(
        "response_processing_time_seconds", delta,
        "status" => response.status().to_string(),
        "method" => method.to_string(),
    );

    response
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt::fmt()
        .with_max_level(LevelFilter::INFO)
        .init();

    PrometheusBuilder::new()
        .add_global_label("service", "mastodon")
        .with_http_listener(args.metrics)
        .set_buckets(&[
            1e-3, 2e-3, 3e-3, 4e-3, 5e-3, 6e-3, 7e-3, 8e-3, 9e-3, // ms
            1e-2, 2e-2, 3e-2, 4e-2, 5e-2, 6e-2, 7e-2, 8e-2, 9e-2, // 10s ms
            1e-1, 2e-1, 3e-1, 4e-1, 5e-1, 6e-1, 7e-1, 8e-1, 9e-1, // 100s ms
            1e0, 2e0, 3e0, 4e0, 5e0, 6e0, 7e0, 8e0, 9e0, // s
        ])
        .unwrap()
        .install()
        .unwrap();

    let client_builder = Client::builder();

    let remote = args.remote;

    let client =
        client_builder.build::<_, Body>(HttpConnector::new_with_resolver(Resolver { remote }));

    let state = ProxyState { client, remote };

    Server::bind(&args.bind)
        .serve(
            Router::new()
                .fallback(fallback)
                .with_state(state)
                .layer(from_fn(observe))
                .into_make_service(),
        )
        .await
        .unwrap();
}

#[derive(Clone)]
pub struct Resolver {
    pub remote: SocketAddr,
}

impl Service<Name> for Resolver {
    type Response = Once<SocketAddr>;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Name) -> Self::Future {
        ready(Ok::<_, Infallible>(once(self.remote)))
    }
}
