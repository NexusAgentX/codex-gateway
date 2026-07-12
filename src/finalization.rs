use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use futures_util::FutureExt;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct FinalizationLifecycle {
    inner: Arc<Inner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalizationDrainReport {
    pub registered_tasks: u64,
    pub completed_tasks: u64,
    pub panicked_tasks: u64,
    pub attempt_persistence_tasks: u64,
    pub admission_finalization_tasks: u64,
    pub stream_finalization_tasks: u64,
    pub upstream_health_tasks: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum FinalizationTaskKind {
    AttemptPersistence,
    AdmissionFinalization,
    StreamFinalization,
    UpstreamHealth,
}

pub struct FinalizationTracker {
    inner: Arc<Inner>,
}

struct Inner {
    active_tasks: AtomicUsize,
    producers: AtomicUsize,
    registered_tasks: AtomicU64,
    completed_tasks: AtomicU64,
    panicked_tasks: AtomicU64,
    attempt_persistence_tasks: AtomicU64,
    admission_finalization_tasks: AtomicU64,
    stream_finalization_tasks: AtomicU64,
    upstream_health_tasks: AtomicU64,
    changed: Notify,
}

struct TaskRegistration {
    inner: Arc<Inner>,
}

impl FinalizationLifecycle {
    pub fn new() -> (Self, FinalizationTracker) {
        let inner = Arc::new(Inner {
            active_tasks: AtomicUsize::new(0),
            producers: AtomicUsize::new(1),
            registered_tasks: AtomicU64::new(0),
            completed_tasks: AtomicU64::new(0),
            panicked_tasks: AtomicU64::new(0),
            attempt_persistence_tasks: AtomicU64::new(0),
            admission_finalization_tasks: AtomicU64::new(0),
            stream_finalization_tasks: AtomicU64::new(0),
            upstream_health_tasks: AtomicU64::new(0),
            changed: Notify::new(),
        });
        (
            Self {
                inner: inner.clone(),
            },
            FinalizationTracker { inner },
        )
    }

    pub async fn drain(&self) -> FinalizationDrainReport {
        loop {
            let changed = self.inner.changed.notified();
            if self.inner.producers.load(Ordering::Acquire) == 0
                && self.inner.active_tasks.load(Ordering::Acquire) == 0
            {
                return FinalizationDrainReport {
                    registered_tasks: self.inner.registered_tasks.load(Ordering::Acquire),
                    completed_tasks: self.inner.completed_tasks.load(Ordering::Acquire),
                    panicked_tasks: self.inner.panicked_tasks.load(Ordering::Acquire),
                    attempt_persistence_tasks: self
                        .inner
                        .attempt_persistence_tasks
                        .load(Ordering::Acquire),
                    admission_finalization_tasks: self
                        .inner
                        .admission_finalization_tasks
                        .load(Ordering::Acquire),
                    stream_finalization_tasks: self
                        .inner
                        .stream_finalization_tasks
                        .load(Ordering::Acquire),
                    upstream_health_tasks: self.inner.upstream_health_tasks.load(Ordering::Acquire),
                };
            }
            changed.await;
        }
    }

    pub async fn wait_for_completed_tasks(&self, expected: u64) {
        loop {
            let changed = self.inner.changed.notified();
            if self.inner.completed_tasks.load(Ordering::Acquire) >= expected {
                return;
            }
            changed.await;
        }
    }
}

impl FinalizationTracker {
    pub(crate) fn spawn<F>(
        &self,
        kind: FinalizationTaskKind,
        future: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let registration = self.register(kind);
        tokio::spawn(async move {
            let _registration = registration;
            if AssertUnwindSafe(future).catch_unwind().await.is_err() {
                _registration
                    .inner
                    .panicked_tasks
                    .fetch_add(1, Ordering::Relaxed);
                tracing::error!(task_kind = kind.as_str(), "finalization task panicked");
            }
        })
    }

    fn register(&self, kind: FinalizationTaskKind) -> TaskRegistration {
        self.inner.registered_tasks.fetch_add(1, Ordering::Relaxed);
        match kind {
            FinalizationTaskKind::AttemptPersistence => &self.inner.attempt_persistence_tasks,
            FinalizationTaskKind::AdmissionFinalization => &self.inner.admission_finalization_tasks,
            FinalizationTaskKind::StreamFinalization => &self.inner.stream_finalization_tasks,
            FinalizationTaskKind::UpstreamHealth => &self.inner.upstream_health_tasks,
        }
        .fetch_add(1, Ordering::Relaxed);
        self.inner.active_tasks.fetch_add(1, Ordering::AcqRel);
        TaskRegistration {
            inner: self.inner.clone(),
        }
    }
}

impl FinalizationTaskKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::AttemptPersistence => "attempt_persistence",
            Self::AdmissionFinalization => "admission_finalization",
            Self::StreamFinalization => "stream_finalization",
            Self::UpstreamHealth => "upstream_health",
        }
    }
}

impl Clone for FinalizationTracker {
    fn clone(&self) -> Self {
        self.inner.producers.fetch_add(1, Ordering::AcqRel);
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Default for FinalizationTracker {
    fn default() -> Self {
        FinalizationLifecycle::new().1
    }
}

impl Drop for FinalizationTracker {
    fn drop(&mut self) {
        self.inner.producers.fetch_sub(1, Ordering::AcqRel);
        self.inner.changed.notify_waiters();
    }
}

impl Drop for TaskRegistration {
    fn drop(&mut self) {
        self.inner.completed_tasks.fetch_add(1, Ordering::Relaxed);
        self.inner.active_tasks.fetch_sub(1, Ordering::AcqRel);
        self.inner.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[tokio::test]
    async fn drain_waits_for_late_registered_tasks_and_survives_panics() {
        let (lifecycle, tracker) = FinalizationLifecycle::new();
        let producer = tracker.clone();
        drop(tracker);
        let drain_started = Arc::new(Notify::new());
        let drain_finished = Arc::new(AtomicBool::new(false));
        let drain = tokio::spawn({
            let lifecycle = lifecycle.clone();
            let drain_started = drain_started.clone();
            let drain_finished = drain_finished.clone();
            async move {
                drain_started.notify_one();
                lifecycle.drain().await;
                drain_finished.store(true, Ordering::Release);
            }
        });
        drain_started.notified().await;
        tokio::task::yield_now().await;
        assert!(!drain_finished.load(Ordering::Acquire));

        let release = Arc::new(Notify::new());
        let task = producer.spawn(FinalizationTaskKind::AttemptPersistence, {
            let release = release.clone();
            async move {
                release.notified().await;
                panic!("injected finalization panic");
            }
        });
        assert_eq!(lifecycle.inner.registered_tasks.load(Ordering::Acquire), 1);
        assert_eq!(lifecycle.inner.active_tasks.load(Ordering::Acquire), 1);
        drop(producer);
        release.notify_one();
        task.await.unwrap();
        drain.await.unwrap();

        assert!(drain_finished.load(Ordering::Acquire));
        assert_eq!(lifecycle.inner.completed_tasks.load(Ordering::Acquire), 1);
        assert_eq!(lifecycle.inner.panicked_tasks.load(Ordering::Acquire), 1);
    }
}
