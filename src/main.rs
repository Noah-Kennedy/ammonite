use crate::cli::Args;
use crate::state::ProxyState;
use axum::body::Body;
use axum::extract::{Host, State};
use axum::http::Request;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;
use clap::Parser;
use hyper::{Client, Server, StatusCode, Uri};
use metrics::histogram;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::time::Instant;
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
        .authority(hostname)
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
        tracing::error!(
            message = "Serving error",
            "status" = response.status().as_str(),
            "uri" = uri.to_string(),
            "method" = method.as_str(),
            "time" = delta.as_secs_f64()
        )
    } else {
        tracing::info!(
            message = "Serving response",
            "status" = response.status().as_str(),
            "uri" = uri.to_string(),
            "method" = method.as_str(),
            "time" = delta.as_secs_f64()
        )
    }

    histogram!("response_processing_time_seconds", delta);

    response
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt::fmt()
        .with_max_level(LevelFilter::TRACE)
        .init();

    PrometheusBuilder::new()
        .add_global_label("service", "freighter")
        .with_http_listener(args.metrics)
        .set_buckets(&[
            100e-6, 500e-6, 1e-3, 5e-3, 1e-2, 5e-2, 1e-1, 2e-1, 3e-1, 4e-1, 5e-1, 6e-1, 7e-1, 8e-1,
            9e-1, 1.0, 5.0, 10.0,
        ])
        .unwrap()
        .install()
        .unwrap();

    let state = ProxyState {
        client: Client::new(),
        remote: args.remote,
    };

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
