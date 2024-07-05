use axum::response::IntoResponse;
use prometheus::{Encoder, TextEncoder};

fn encode_metrics() -> Vec<u8> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    buffer
}

pub async fn metrics_handler() -> impl IntoResponse {
    let buffer = encode_metrics();
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buffer,
    )
}
