use super::*;
use axum::body::{Body, Bytes};
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_stream::StreamExt;

pub(crate) enum RoutedResponse {
    Json(Value),
    Sse(reqwest::Response),
}

pub(crate) fn sse_keepalive_ms() -> u64 {
    std::env::var("LLMHOTSPOT_SSE_KEEPALIVE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 250)
        .unwrap_or(15_000)
}

fn sse_idle_timeout_ms() -> u64 {
    std::env::var("LLMHOTSPOT_SSE_IDLE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 5_000)
        .unwrap_or(120_000)
}

pub(crate) fn sse_response(upstream: reqwest::Response) -> axum::response::Response {
    let content_type = upstream
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("text/event-stream")
        .to_string();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    tokio::spawn(async move {
        let mut done_seen = false;
        let mut tail = String::new();
        let mut interval = tokio::time::interval(Duration::from_millis(sse_keepalive_ms()));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // `interval` ticks immediately once; consume that so we don't emit a
        // leading `: ping` before any real SSE data.
        interval.tick().await;
        let idle_timeout = Duration::from_millis(sse_idle_timeout_ms());
        let mut last_activity = Instant::now();
        let mut upstream_stream = upstream.bytes_stream();

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if last_activity.elapsed() >= idle_timeout {
                        let _ = tx
                            .send(Ok(Bytes::from_static(
                                br#"data: {"error":{"type":"upstream_timeout","message":"upstream stream idle timeout"}}\n\n"#,
                            )))
                            .await;
                        break;
                    }
                    if tx.send(Ok(Bytes::from_static(b": ping\n\n"))).await.is_err() {
                        return;
                    }
                }
                next = upstream_stream.next() => {
                    match next {
                        Some(Ok(chunk)) => {
                            last_activity = Instant::now();
                            if !done_seen {
                                let s = String::from_utf8_lossy(&chunk);
                                tail.push_str(&s);
                                if tail.len() > 4096 {
                                    tail = tail[tail.len().saturating_sub(4096)..].to_string();
                                }
                                if tail.contains("data: [DONE]") {
                                    done_seen = true;
                                }
                            }
                            if tx.send(Ok(chunk)).await.is_err() {
                                return;
                            }
                        }
                        Some(Err(e)) => {
                            let _ = tx
                                .send(Err(std::io::Error::other(e)))
                                .await;
                            break;
                        }
                        None => break,
                    }
                }
            }
        }

        if !done_seen {
            let _ = tx
                .send(Ok(Bytes::from_static(b"\n\ndata: [DONE]\n\n")))
                .await;
        }
    });

    let body = Body::from_stream(stream);
    let mut resp = (StatusCode::OK, body).into_response();
    if let Ok(header) = content_type.parse() {
        resp.headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, header);
    }
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        "no-cache".parse().expect("valid header"),
    );
    resp
}

pub(crate) fn anthropic_sse_passthrough_response(
    upstream: reqwest::Response,
) -> axum::response::Response {
    let content_type = upstream
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("text/event-stream")
        .to_string();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(sse_keepalive_ms()));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // Avoid an immediate keepalive comment before upstream data.
        interval.tick().await;
        let idle_timeout = Duration::from_millis(sse_idle_timeout_ms());
        let mut last_activity = Instant::now();
        let mut upstream_stream = upstream.bytes_stream();
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if last_activity.elapsed() >= idle_timeout {
                        let _ = tx
                            .send(Ok(Bytes::from_static(
                                br#"event: error\ndata: {"type":"upstream_timeout","message":"upstream stream idle timeout"}\n\n"#,
                            )))
                            .await;
                        return;
                    }
                    if tx.send(Ok(Bytes::from_static(b": ping\n\n"))).await.is_err() {
                        return;
                    }
                }
                next = upstream_stream.next() => {
                    match next {
                        Some(Ok(chunk)) => {
                            last_activity = Instant::now();
                            if tx.send(Ok(chunk)).await.is_err() {
                                return;
                            }
                        }
                        Some(Err(e)) => {
                            let _ = tx
                                .send(Err(std::io::Error::other(e)))
                                .await;
                            return;
                        }
                        None => return,
                    }
                }
            }
        }
    });
    let body = Body::from_stream(stream);
    let mut resp = (StatusCode::OK, body).into_response();
    if let Ok(header) = content_type.parse() {
        resp.headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, header);
    }
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        "no-cache".parse().expect("valid header"),
    );
    resp
}
