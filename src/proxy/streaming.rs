use std::convert::Infallible;

use async_stream::stream;
use axum::{
    body::Body,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;

use crate::usage::SseUsageScanner;

use super::{
    attempt::StreamingAttempt,
    headers,
    settlement::{self, AdmissionSettlement, AttemptRecord},
};

pub(super) fn response(
    mut stream_attempt: StreamingAttempt,
    admission: AdmissionSettlement,
    expose_debug_headers: bool,
    route_strategy: &str,
) -> Response {
    headers::set_request_id(
        &mut stream_attempt.headers,
        &stream_attempt.record.base.request_id,
    );
    headers::set_debug_headers(
        &mut stream_attempt.headers,
        expose_debug_headers,
        route_strategy,
        &stream_attempt.upstream_id,
    );
    let status = stream_attempt.status;
    let response_headers = stream_attempt.headers;
    let db = admission.pool().clone();
    let mut upstream_stream = stream_attempt.upstream_response.bytes_stream();
    let guard = StreamingFinalizationGuard::new(db, stream_attempt.record, admission);
    let body_stream = stream! {
        let mut guard = guard;
        while let Some(item) = upstream_stream.next().await {
            match item {
                Ok(chunk) => {
                    guard.observe(&chunk);
                    if guard.completed()
                        && let Some(task) = guard.spawn_finalization(None, "completed")
                        && let Err(error) = task.await
                    {
                        tracing::warn!(?error, "stream finalization task failed");
                    }
                    yield Ok::<Bytes, Infallible>(chunk);
                }
                Err(error) => {
                    tracing::warn!(?error, "upstream SSE stream failed");
                    guard.set_error("upstream_error");
                    break;
                }
            }
        }
        if let Some(task) = guard.spawn_finalization(None, "eof")
            && let Err(error) = task.await
        {
            tracing::warn!(?error, "stream finalization task failed");
        }
    };
    (status, response_headers, Body::from_stream(body_stream)).into_response()
}

struct StreamingFinalizationGuard {
    db: sqlx::SqlitePool,
    record: Option<AttemptRecord>,
    admission: Option<AdmissionSettlement>,
    scanner: SseUsageScanner,
    output_chars: i64,
    error_code: Option<&'static str>,
}

impl StreamingFinalizationGuard {
    fn new(db: sqlx::SqlitePool, record: AttemptRecord, admission: AdmissionSettlement) -> Self {
        Self {
            db,
            record: Some(record),
            admission: Some(admission),
            scanner: SseUsageScanner::default(),
            output_chars: 0,
            error_code: None,
        }
    }

    fn observe(&mut self, chunk: &[u8]) {
        self.output_chars += chunk.len() as i64;
        self.scanner.push(chunk);
    }

    fn completed(&self) -> bool {
        self.scanner.completed()
    }

    fn set_error(&mut self, error_code: &'static str) {
        self.error_code = Some(error_code);
    }

    fn spawn_finalization(
        &mut self,
        forced_error_code: Option<&'static str>,
        reason: &'static str,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let mut record = self.record.take()?;
        let admission = self
            .admission
            .take()
            .expect("stream admission accompanies its attempt record");
        let error_code = forced_error_code.or(self.error_code);
        record.status = stream_log_status(record.status, error_code);
        record.error_code = match forced_error_code {
            Some(code) => Some(code.to_string()),
            None => self.error_code.map(str::to_string).or(record.error_code),
        };
        record.usage = self.scanner.snapshot();
        record.output_chars = self.output_chars;
        Some(settlement::spawn_stream_finalization(
            self.db.clone(),
            record,
            admission,
            reason,
        ))
    }
}

impl Drop for StreamingFinalizationGuard {
    fn drop(&mut self) {
        let _ = self.spawn_finalization(Some("client_disconnected"), "disconnected");
    }
}

fn stream_log_status(
    status: axum::http::StatusCode,
    error_code: Option<&str>,
) -> axum::http::StatusCode {
    match error_code {
        Some("client_disconnected") => axum::http::StatusCode::from_u16(499)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        Some("upstream_error") if status.is_success() => axum::http::StatusCode::BAD_GATEWAY,
        _ => status,
    }
}
