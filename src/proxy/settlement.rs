use std::time::Instant;

use axum::http::StatusCode;

use crate::{
    FinalizationTracker, clock::SharedClock, finalization::FinalizationTaskKind, storage,
    usage::UsageSnapshot,
};

#[derive(Clone)]
pub(super) struct AttemptLogBase {
    pub(super) clock: SharedClock,
    pub(super) request_id: String,
    pub(super) user_id: String,
    pub(super) api_key_id: String,
    pub(super) model_id: Option<String>,
    pub(super) upstream_id: Option<String>,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) stream: bool,
    pub(super) started_at: String,
    pub(super) user_agent: Option<String>,
    pub(super) input_chars: i64,
    pub(super) client_metadata_sanitized: Option<String>,
    pub(super) route_strategy: Option<String>,
    pub(super) route_decision_json: Option<String>,
}

#[derive(Clone)]
pub(super) struct HealthUpdate {
    pub(super) upstream_id: String,
    pub(super) status: &'static str,
    pub(super) error_sample: Option<&'static str>,
}

pub(super) struct AttemptRecord {
    pub(super) base: AttemptLogBase,
    pub(super) status: StatusCode,
    pub(super) error_code: Option<String>,
    pub(super) usage: UsageSnapshot,
    pub(super) output_chars: i64,
    pub(super) started: Instant,
    pub(super) health: Option<HealthUpdate>,
}

impl AttemptRecord {
    fn into_log(self) -> storage::RequestLogInsert {
        let finished_at = self
            .base
            .clock
            .now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        storage::RequestLogInsert {
            request_id: self.base.request_id,
            user_id: self.base.user_id,
            api_key_id: self.base.api_key_id,
            model_id: self.base.model_id,
            upstream_id: self.base.upstream_id,
            method: self.base.method,
            path: self.base.path,
            status_code: Some(i64::from(self.status.as_u16())),
            error_code: self.error_code,
            stream: self.base.stream,
            usage: self.usage,
            input_chars: self.base.input_chars,
            output_chars: self.output_chars,
            latency_ms: self.started.elapsed().as_millis() as i64,
            started_at: self.base.started_at,
            finished_at,
            client_ip_hash: None,
            user_agent: self.base.user_agent,
            client_metadata_sanitized: self.base.client_metadata_sanitized,
            route_strategy: self.base.route_strategy,
            route_decision_json: self.base.route_decision_json,
        }
    }
}

pub(super) struct AttemptCancellationGuard {
    db: sqlx::SqlitePool,
    finalizations: FinalizationTracker,
    base: Option<AttemptLogBase>,
    started: Instant,
}

impl AttemptCancellationGuard {
    pub(super) fn new(
        db: sqlx::SqlitePool,
        finalizations: FinalizationTracker,
        base: AttemptLogBase,
        started: Instant,
    ) -> Self {
        Self {
            db,
            finalizations,
            base: Some(base),
            started,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.base = None;
    }
}

impl Drop for AttemptCancellationGuard {
    fn drop(&mut self) {
        let Some(base) = self.base.take() else {
            return;
        };
        let record = AttemptRecord {
            base,
            status: StatusCode::from_u16(499).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            error_code: Some("client_disconnected".to_string()),
            usage: UsageSnapshot::default(),
            output_chars: 0,
            started: self.started,
            health: None,
        };
        spawn_attempt_persistence(
            &self.finalizations,
            self.db.clone(),
            record,
            "request_cancelled",
        );
    }
}

pub(super) struct AdmissionSettlement {
    db: sqlx::SqlitePool,
    finalizations: FinalizationTracker,
    clock: SharedClock,
    admission: Option<storage::LimitAdmission>,
    total_tokens: i64,
}

impl AdmissionSettlement {
    pub(super) fn new(
        db: sqlx::SqlitePool,
        finalizations: FinalizationTracker,
        clock: SharedClock,
        admission: storage::LimitAdmission,
    ) -> Self {
        Self {
            db,
            finalizations,
            clock,
            admission: Some(admission),
            total_tokens: 0,
        }
    }

    pub(super) fn pool(&self) -> &sqlx::SqlitePool {
        &self.db
    }

    pub(super) fn set_total_tokens(&mut self, total_tokens: i64) {
        self.total_tokens = total_tokens.max(0);
    }

    pub(super) async fn finalize(&mut self, total_tokens: i64) {
        self.set_total_tokens(total_tokens);
        let Some(admission) = self.admission.as_ref() else {
            return;
        };
        match storage::finalize_limit_admission_with_clock(
            &self.db,
            admission,
            self.total_tokens,
            self.clock.clone(),
        )
        .await
        {
            Ok(()) => self.admission = None,
            Err(error) => tracing::warn!(?error, "failed to finalize limit admission"),
        }
    }
}

impl Drop for AdmissionSettlement {
    fn drop(&mut self) {
        let Some(admission) = self.admission.take() else {
            return;
        };
        spawn_limit_finalization(
            &self.finalizations,
            self.db.clone(),
            self.clock.clone(),
            admission,
            self.total_tokens,
            "request_cancelled",
        );
    }
}

pub(super) async fn persist_attempt(
    db: &sqlx::SqlitePool,
    finalizations: &FinalizationTracker,
    record: AttemptRecord,
) {
    let task = spawn_attempt_persistence(finalizations, db.clone(), record, "attempt_complete");
    if let Err(error) = task.await {
        tracing::warn!(?error, "attempt persistence task failed");
    }
}

pub(super) async fn persist_pre_attempt_health(
    db: &sqlx::SqlitePool,
    finalizations: &FinalizationTracker,
    request_id: &str,
    health: Option<HealthUpdate>,
) {
    let Some(health) = health else {
        return;
    };
    persist_health(db, finalizations, request_id, health).await;
}

pub(super) async fn persist_response_health(
    db: &sqlx::SqlitePool,
    finalizations: &FinalizationTracker,
    request_id: &str,
    health: HealthUpdate,
) {
    persist_health(db, finalizations, request_id, health).await;
}

pub(super) fn spawn_stream_finalization(
    db: sqlx::SqlitePool,
    record: AttemptRecord,
    mut admission: AdmissionSettlement,
    reason: &'static str,
) -> tokio::task::JoinHandle<()> {
    let finalizations = admission.finalizations.clone();
    finalizations.spawn(FinalizationTaskKind::StreamFinalization, async move {
        let total_tokens = record.usage.total_tokens;
        persist_attempt_inner(&db, record, reason).await;
        admission.finalize(total_tokens).await;
    })
}

fn spawn_attempt_persistence(
    finalizations: &FinalizationTracker,
    db: sqlx::SqlitePool,
    record: AttemptRecord,
    reason: &'static str,
) -> tokio::task::JoinHandle<()> {
    finalizations.spawn(FinalizationTaskKind::AttemptPersistence, async move {
        persist_attempt_inner(&db, record, reason).await;
    })
}

async fn persist_attempt_inner(db: &sqlx::SqlitePool, record: AttemptRecord, reason: &'static str) {
    if let Some(health) = record.health.clone() {
        persist_health_inner(db, &record.base.request_id, health).await;
    }
    let status = record.status;
    let log = record.into_log();
    let request_id = log.request_id.clone();
    if let Err(error) = storage::insert_request_log(db, log).await {
        tracing::warn!(?error, %reason, "failed to write request attempt log");
    } else {
        tracing::debug!(%request_id, status = status.as_u16(), %reason, "request attempt log written");
    }
}

async fn persist_health(
    db: &sqlx::SqlitePool,
    finalizations: &FinalizationTracker,
    request_id: &str,
    health: HealthUpdate,
) {
    let db = db.clone();
    let request_id = request_id.to_string();
    let task = finalizations.spawn(FinalizationTaskKind::UpstreamHealth, async move {
        persist_health_inner(&db, &request_id, health).await;
    });
    if let Err(error) = task.await {
        tracing::warn!(?error, "upstream health task failed");
    }
}

async fn persist_health_inner(db: &sqlx::SqlitePool, request_id: &str, health: HealthUpdate) {
    if let Err(error) =
        storage::record_upstream_health(db, &health.upstream_id, health.status, health.error_sample)
            .await
    {
        tracing::warn!(
            ?error,
            %request_id,
            upstream_id = %health.upstream_id,
            "failed to update upstream health"
        );
    }
}

fn spawn_limit_finalization(
    finalizations: &FinalizationTracker,
    db: sqlx::SqlitePool,
    clock: SharedClock,
    admission: storage::LimitAdmission,
    total_tokens: i64,
    reason: &'static str,
) {
    finalizations.spawn(FinalizationTaskKind::AdmissionFinalization, async move {
        if let Err(error) =
            storage::finalize_limit_admission_with_clock(&db, &admission, total_tokens, clock).await
        {
            tracing::warn!(?error, %reason, "failed to finalize dropped limit admission");
        }
    });
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };

    use tracing::instrument::WithSubscriber;

    use super::*;

    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufferWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn health_write_failure_logs_request_and_upstream_ids_without_payloads() {
        let pool = storage::connect_and_migrate("sqlite://:memory:")
            .await
            .unwrap();
        pool.close().await;
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || BufferWriter(writer_output.clone()))
            .finish();

        persist_health_inner(
            &pool,
            "request-diagnostic-id",
            HealthUpdate {
                upstream_id: "upstream-diagnostic-id".into(),
                status: "degraded",
                error_sample: Some("upstream_error"),
            },
        )
        .with_subscriber(subscriber)
        .await;

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(output.contains("request_id=request-diagnostic-id"));
        assert!(output.contains("upstream_id=upstream-diagnostic-id"));
        assert!(output.contains("failed to update upstream health"));
        assert!(!output.contains("authorization"));
        assert!(!output.contains("body"));
    }
}
