//! Fixed-schema evidence for the relay's earliest startup steps.
//!
//! These events are written directly to stderr because crypto, tracing,
//! configuration, and metrics setup can fail before the normal telemetry
//! stack exists. Values are closed enums; raw errors and secrets never enter
//! the lifecycle schema.

use std::{
    io::Write as _,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use uuid::Uuid;

const EVENT_NAME: &str = "buzz_process_lifecycle";
const SCHEMA_VERSION: u8 = 1;

/// A bounded early-startup phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPhase {
    /// Process entry through a usable metrics listener.
    ProcessTelemetry,
    /// Install the process-wide rustls provider.
    CryptoInit,
    /// Install structured logging and optional OTLP tracing.
    TracingInit,
    /// Parse environment-backed configuration.
    ConfigLoad,
    /// Load and validate relay key material.
    KeyLoad,
    /// Install the Prometheus recorder and bind its listener.
    MetricsBind,
}

impl StartupPhase {
    /// The complete wire vocabulary.
    pub const ALL: [Self; 6] = [
        Self::ProcessTelemetry,
        Self::CryptoInit,
        Self::TracingInit,
        Self::ConfigLoad,
        Self::KeyLoad,
        Self::MetricsBind,
    ];

    /// Stable wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessTelemetry => "process_telemetry",
            Self::CryptoInit => "crypto_init",
            Self::TracingInit => "tracing_init",
            Self::ConfigLoad => "config_load",
            Self::KeyLoad => "key_load",
            Self::MetricsBind => "metrics_bind",
        }
    }
}

/// A bounded terminal status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleStatus {
    /// Required work completed.
    Succeeded,
    /// Optional work failed and startup may continue.
    Degraded,
    /// Optional work was not configured.
    Skipped,
    /// Required work failed.
    Failed,
    /// The phase was cancelled.
    Cancelled,
    /// The phase exceeded its deadline.
    TimedOut,
    /// Control flow dropped the phase without an explicit terminal.
    Abandoned,
}

impl LifecycleStatus {
    #[cfg(test)]
    const ALL: [Self; 7] = [
        Self::Succeeded,
        Self::Degraded,
        Self::Skipped,
        Self::Failed,
        Self::Cancelled,
        Self::TimedOut,
        Self::Abandoned,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Degraded => "degraded",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Abandoned => "abandoned",
        }
    }

    const fn metric_result(self) -> &'static str {
        match self {
            Self::Succeeded => "success",
            Self::Degraded => "degraded",
            Self::Skipped | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Abandoned => {
                "failure"
            }
        }
    }
}

/// A secret-safe terminal reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleReason {
    /// Tokio runtime construction failed.
    RuntimeBuild,
    /// Another rustls provider was already installed.
    ProviderConflict,
    /// The optional OTLP exporter could not be built.
    ExporterBuild,
    /// Optional telemetry was not configured.
    NotConfigured,
    /// A configured value could not be parsed.
    Parse,
    /// A required value was missing.
    Missing,
    /// A required value was invalid.
    RequiredInvalid,
    /// A required listener could not bind.
    Bind,
    /// A global metrics recorder already existed.
    RecorderConflict,
    /// A phase owner disappeared without a terminal.
    OwnerDropped,
    /// A panic unwound through the phase.
    Panic,
    /// No narrower safe classification exists.
    Unknown,
}

impl LifecycleReason {
    #[cfg(test)]
    const ALL: [Self; 12] = [
        Self::RuntimeBuild,
        Self::ProviderConflict,
        Self::ExporterBuild,
        Self::NotConfigured,
        Self::Parse,
        Self::Missing,
        Self::RequiredInvalid,
        Self::Bind,
        Self::RecorderConflict,
        Self::OwnerDropped,
        Self::Panic,
        Self::Unknown,
    ];

    /// Stable wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBuild => "runtime_build",
            Self::ProviderConflict => "provider_conflict",
            Self::ExporterBuild => "exporter_build",
            Self::NotConfigured => "not_configured",
            Self::Parse => "parse",
            Self::Missing => "missing",
            Self::RequiredInvalid => "required_invalid",
            Self::Bind => "bind",
            Self::RecorderConflict => "recorder_conflict",
            Self::OwnerDropped => "owner_dropped",
            Self::Panic => "panic",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct LifecycleEvent {
    event_name: &'static str,
    schema_version: u8,
    process_boot_id: Uuid,
    sequence: u64,
    track: &'static str,
    phase: &'static str,
    edge: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    process_started_at_unix_ms: u64,
    observed_at_unix_ms: u64,
    process_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase_elapsed_ms: Option<u64>,
}

trait EventWriter: Send + Sync {
    fn emit(&self, event: &LifecycleEvent);
}

struct StderrWriter;

impl EventWriter for StderrWriter {
    fn emit(&self, event: &LifecycleEvent) {
        // Best effort: reporting a startup error must never create another
        // panic. This sink intentionally ignores RUST_LOG filters.
        let mut stderr = std::io::stderr().lock();
        if serde_json::to_writer(&mut stderr, event).is_ok() {
            let _ = stderr.write_all(b"\n");
        }
    }
}

struct ProcessLifecycle {
    boot_id: Uuid,
    sequence: AtomicU64,
    wall_origin: SystemTime,
    monotonic_origin: Instant,
    writer: Arc<dyn EventWriter>,
}

impl ProcessLifecycle {
    fn new(writer: Arc<dyn EventWriter>) -> Arc<Self> {
        let wall_origin = SystemTime::now();
        let monotonic_origin = Instant::now();
        Arc::new(Self {
            boot_id: Uuid::new_v4(),
            sequence: AtomicU64::new(1),
            wall_origin,
            monotonic_origin,
            writer,
        })
    }

    fn start(self: &Arc<Self>, phase: StartupPhase) -> PhaseGuard {
        let started_at = if phase == StartupPhase::ProcessTelemetry {
            self.monotonic_origin
        } else {
            Instant::now()
        };
        self.emit(phase, "started", None, None, None);
        PhaseGuard {
            lifecycle: Arc::clone(self),
            phase,
            started_at,
            finished: false,
        }
    }

    fn emit(
        &self,
        phase: StartupPhase,
        edge: &'static str,
        status: Option<LifecycleStatus>,
        reason: Option<LifecycleReason>,
        elapsed: Option<Duration>,
    ) {
        self.writer.emit(&LifecycleEvent {
            event_name: EVENT_NAME,
            schema_version: SCHEMA_VERSION,
            process_boot_id: self.boot_id,
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            track: "startup",
            phase: phase.as_str(),
            edge,
            status: status.map(LifecycleStatus::as_str),
            reason: reason.map(LifecycleReason::as_str),
            process_started_at_unix_ms: millis_since_epoch(self.wall_origin),
            observed_at_unix_ms: millis_since_epoch(SystemTime::now()),
            process_elapsed_ms: saturating_millis(self.monotonic_origin.elapsed()),
            phase_elapsed_ms: elapsed.map(saturating_millis),
        });
    }
}

/// Owns one phase from its start event through exactly one terminal.
pub struct PhaseGuard {
    lifecycle: Arc<ProcessLifecycle>,
    phase: StartupPhase,
    started_at: Instant,
    finished: bool,
}

impl PhaseGuard {
    /// Record successful completion.
    pub fn succeed(self) {
        self.finish(LifecycleStatus::Succeeded, None);
    }

    /// Record an intentional skip.
    pub fn skip(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::Skipped, Some(reason));
    }

    /// Record an allowed degradation.
    pub fn degrade(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::Degraded, Some(reason));
    }

    /// Record a fatal failure.
    pub fn fail(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::Failed, Some(reason));
    }

    /// Record cancellation.
    pub fn cancel(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::Cancelled, Some(reason));
    }

    /// Record a timeout.
    pub fn time_out(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::TimedOut, Some(reason));
    }

    fn finish(mut self, status: LifecycleStatus, reason: Option<LifecycleReason>) -> Duration {
        let elapsed = self.started_at.elapsed();
        self.lifecycle
            .emit(self.phase, "terminal", Some(status), reason, Some(elapsed));
        self.finished = true;
        elapsed
    }
}

impl Drop for PhaseGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let (status, reason) = if std::thread::panicking() {
            (LifecycleStatus::Failed, LifecycleReason::Panic)
        } else {
            (LifecycleStatus::Abandoned, LifecycleReason::OwnerDropped)
        };
        self.lifecycle.emit(
            self.phase,
            "terminal",
            Some(status),
            Some(reason),
            Some(self.started_at.elapsed()),
        );
        self.finished = true;
    }
}

/// Immutable per-process receipt exposed by the two startup gauges.
#[derive(Clone, Copy, Debug)]
pub struct StartupReceipt {
    result: &'static str,
    duration: Duration,
}

impl StartupReceipt {
    /// Set the current startup receipt gauges.
    pub fn emit(self) {
        metrics::gauge!(
            "buzz_startup_phase_terminal",
            "phase" => StartupPhase::ProcessTelemetry.as_str(),
            "result" => self.result,
        )
        .set(1.0);
        metrics::gauge!(
            "buzz_startup_phase_duration_seconds",
            "phase" => StartupPhase::ProcessTelemetry.as_str(),
        )
        .set(self.duration.as_secs_f64());
    }

    /// Keep the receipt present beyond the exporter's gauge idle timeout.
    pub fn spawn_refresh(self, idle_timeout_secs: u64) -> StartupReceiptHeartbeat {
        self.emit();
        let period = Duration::from_secs((idle_timeout_secs / 3).max(1));
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                self.emit();
            }
        });
        StartupReceiptHeartbeat { task: Some(task) }
    }

    #[cfg(test)]
    fn test_value() -> Self {
        Self {
            result: "success",
            duration: Duration::from_millis(12),
        }
    }
}

/// Owns the one Package 1 heartbeat task.
pub struct StartupReceiptHeartbeat {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl StartupReceiptHeartbeat {
    /// Stop and join the heartbeat during normal relay teardown.
    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for StartupReceiptHeartbeat {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Tracks the aggregate early-startup phase and its fixed subphases.
pub struct BootTracker {
    lifecycle: Arc<ProcessLifecycle>,
    headline: PhaseGuard,
    degraded: Option<LifecycleReason>,
}

impl BootTracker {
    /// Start lifecycle accounting before constructing Tokio.
    pub fn start_before_runtime<Runtime, Error>(
        build: impl FnOnce() -> Result<Runtime, Error>,
    ) -> Result<(Runtime, Self), Error> {
        Self::start_before_runtime_with_writer(Arc::new(StderrWriter), build)
    }

    fn start_before_runtime_with_writer<Runtime, Error>(
        writer: Arc<dyn EventWriter>,
        build: impl FnOnce() -> Result<Runtime, Error>,
    ) -> Result<(Runtime, Self), Error> {
        let lifecycle = ProcessLifecycle::new(writer);
        let boot = Self {
            headline: lifecycle.start(StartupPhase::ProcessTelemetry),
            lifecycle,
            degraded: None,
        };
        match build() {
            Ok(runtime) => Ok((runtime, boot)),
            Err(error) => {
                boot.fail(LifecycleReason::RuntimeBuild);
                Err(error)
            }
        }
    }

    /// Start a fixed early-startup subphase.
    #[must_use = "dropping a phase guard emits an abandoned terminal"]
    pub fn start(&self, phase: StartupPhase) -> PhaseGuard {
        assert_ne!(phase, StartupPhase::ProcessTelemetry);
        self.lifecycle.start(phase)
    }

    /// Run a required phase and atomically terminalize both it and startup on failure.
    pub fn run_required<T, Error>(
        self,
        phase: StartupPhase,
        work: impl FnOnce() -> Result<T, Error>,
        classify: impl FnOnce(&Error) -> LifecycleReason,
    ) -> Result<(Self, T), Error> {
        let phase_guard = self.start(phase);
        match work() {
            Ok(value) => {
                phase_guard.succeed();
                Ok((self, value))
            }
            Err(error) => {
                let reason = classify(&error);
                phase_guard.fail(reason);
                self.fail(reason);
                Err(error)
            }
        }
    }

    /// Preserve the first optional degradation for the aggregate receipt.
    pub fn mark_degraded(&mut self, reason: LifecycleReason) {
        self.degraded.get_or_insert(reason);
    }

    /// Finish early startup and return its immutable metric receipt.
    pub fn finish(self) -> StartupReceipt {
        let status = if self.degraded.is_some() {
            LifecycleStatus::Degraded
        } else {
            LifecycleStatus::Succeeded
        };
        let duration = self.headline.finish(status, self.degraded);
        StartupReceipt {
            result: status.metric_result(),
            duration,
        }
    }

    fn fail(self, reason: LifecycleReason) {
        self.headline.fail(reason);
    }
}

fn millis_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(saturating_millis)
        .unwrap_or(0)
}

fn saturating_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{panic::AssertUnwindSafe, sync::Mutex};

    #[derive(Default)]
    struct CapturingWriter(Mutex<Vec<LifecycleEvent>>);

    impl EventWriter for CapturingWriter {
        fn emit(&self, event: &LifecycleEvent) {
            self.0.lock().expect("capturing writer").push(event.clone());
        }
    }

    fn recorder() -> (Arc<ProcessLifecycle>, Arc<CapturingWriter>) {
        let writer = Arc::new(CapturingWriter::default());
        (ProcessLifecycle::new(writer.clone()), writer)
    }

    fn events(writer: &CapturingWriter) -> Vec<LifecycleEvent> {
        writer.0.lock().expect("capturing writer").clone()
    }

    #[test]
    fn explicit_and_dropped_terminals_are_exactly_once() {
        let (lifecycle, writer) = recorder();
        lifecycle.start(StartupPhase::ConfigLoad).succeed();
        drop(lifecycle.start(StartupPhase::KeyLoad));

        let events = events(&writer);
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].status, Some("succeeded"));
        assert_eq!(events[3].status, Some("abandoned"));
        assert_eq!(events[3].reason, Some("owner_dropped"));
    }

    #[test]
    fn panic_unwind_is_bounded() {
        let (lifecycle, writer) = recorder();
        let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _phase = lifecycle.start(StartupPhase::CryptoInit);
            panic!("controlled test panic");
        }));
        assert!(panic.is_err());
        let events = events(&writer);
        assert_eq!(events[1].status, Some("failed"));
        assert_eq!(events[1].reason, Some("panic"));
    }

    #[test]
    fn runtime_failure_terminalizes_the_headline() {
        let writer = Arc::new(CapturingWriter::default());
        let result = BootTracker::start_before_runtime_with_writer(
            writer.clone(),
            || -> Result<(), &'static str> { Err("controlled") },
        );
        assert!(matches!(result, Err("controlled")));
        let events = events(&writer);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].phase, "process_telemetry");
        assert_eq!(events[1].reason, Some("runtime_build"));
    }

    #[test]
    fn aggregate_preserves_optional_degradation() {
        let (lifecycle, writer) = recorder();
        let mut boot = BootTracker {
            headline: lifecycle.start(StartupPhase::ProcessTelemetry),
            lifecycle,
            degraded: None,
        };
        boot.mark_degraded(LifecycleReason::ExporterBuild);
        let receipt = boot.finish();
        assert_eq!(receipt.result, "degraded");
        let events = events(&writer);
        assert_eq!(events[1].status, Some("degraded"));
        assert_eq!(events[1].reason, Some("exporter_build"));
    }

    #[test]
    fn required_failure_terminalizes_subphase_and_headline() {
        let (lifecycle, writer) = recorder();
        let boot = BootTracker {
            headline: lifecycle.start(StartupPhase::ProcessTelemetry),
            lifecycle,
            degraded: None,
        };
        let result = boot.run_required(
            StartupPhase::MetricsBind,
            || -> Result<(), &'static str> { Err("controlled") },
            |_error| LifecycleReason::RecorderConflict,
        );
        assert!(matches!(result, Err("controlled")));

        let events = events(&writer);
        assert_eq!(events.len(), 4);
        assert_eq!(events[2].phase, "metrics_bind");
        assert_eq!(events[2].status, Some("failed"));
        assert_eq!(events[2].reason, Some("recorder_conflict"));
        assert_eq!(events[3].phase, "process_telemetry");
        assert_eq!(events[3].status, Some("failed"));
        assert_eq!(events[3].reason, Some("recorder_conflict"));
    }

    #[test]
    fn schema_and_vocabulary_are_frozen() {
        assert_eq!(
            StartupPhase::ALL.map(StartupPhase::as_str),
            [
                "process_telemetry",
                "crypto_init",
                "tracing_init",
                "config_load",
                "key_load",
                "metrics_bind",
            ]
        );
        let (lifecycle, writer) = recorder();
        drop(lifecycle.start(StartupPhase::ConfigLoad));
        let values: Vec<_> = events(&writer)
            .iter()
            .map(|event| serde_json::to_value(event).expect("serialize lifecycle event"))
            .collect();
        assert_eq!(values[0]["schema_version"], SCHEMA_VERSION);
        assert_eq!(values[0]["event_name"], EVENT_NAME);
        assert_eq!(values[1]["status"], "abandoned");
        let mut started_keys: Vec<_> = values[0]
            .as_object()
            .expect("started event object")
            .keys()
            .map(String::as_str)
            .collect();
        started_keys.sort_unstable();
        assert_eq!(
            started_keys,
            [
                "edge",
                "event_name",
                "observed_at_unix_ms",
                "phase",
                "process_boot_id",
                "process_elapsed_ms",
                "process_started_at_unix_ms",
                "schema_version",
                "sequence",
                "track",
            ]
        );
        let mut terminal_keys: Vec<_> = values[1]
            .as_object()
            .expect("terminal event object")
            .keys()
            .map(String::as_str)
            .collect();
        terminal_keys.sort_unstable();
        assert_eq!(
            terminal_keys,
            [
                "edge",
                "event_name",
                "observed_at_unix_ms",
                "phase",
                "phase_elapsed_ms",
                "process_boot_id",
                "process_elapsed_ms",
                "process_started_at_unix_ms",
                "reason",
                "schema_version",
                "sequence",
                "status",
                "track",
            ]
        );
        assert_eq!(
            LifecycleStatus::ALL.map(LifecycleStatus::as_str),
            [
                "succeeded",
                "degraded",
                "skipped",
                "failed",
                "cancelled",
                "timed_out",
                "abandoned",
            ]
        );
        assert_eq!(
            LifecycleReason::ALL.map(LifecycleReason::as_str),
            [
                "runtime_build",
                "provider_conflict",
                "exporter_build",
                "not_configured",
                "parse",
                "missing",
                "required_invalid",
                "bind",
                "recorder_conflict",
                "owner_dropped",
                "panic",
                "unknown",
            ]
        );
    }

    #[test]
    fn receipt_exposes_exactly_two_bounded_gauge_series() {
        let (recorder, handle) = crate::metrics::startup_test_recorder();
        metrics::with_local_recorder(&recorder, || {
            crate::metrics::describe_startup_metrics();
            StartupReceipt {
                result: "success",
                duration: Duration::from_millis(12),
            }
            .emit();
        });

        let rendered = handle.render();
        assert!(rendered.contains("# TYPE buzz_startup_phase_terminal gauge"));
        assert!(rendered.contains("# TYPE buzz_startup_phase_duration_seconds gauge"));
        assert!(rendered.contains(
            "buzz_startup_phase_terminal{phase=\"process_telemetry\",result=\"success\"} 1"
        ));
        assert!(rendered.lines().any(|line| line
            .starts_with("buzz_startup_phase_duration_seconds{phase=\"process_telemetry\"} ")));
        assert!(rendered
            .contains("buzz_startup_phase_duration_seconds{phase=\"process_telemetry\"} 0.012"));
        assert_eq!(
            rendered
                .lines()
                .filter(|line| !line.starts_with('#') && line.starts_with("buzz_startup_phase_"))
                .count(),
            2
        );
        assert!(!rendered.contains("process_boot_id"));
        assert!(!rendered.contains("reason="));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn heartbeat_survives_real_idle_eviction() {
        const CHILD_ENV: &str = "BUZZ_TEST_STARTUP_RECEIPT_IDLE_CHILD";
        if std::env::var_os(CHILD_ENV).is_some() {
            let listener =
                std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve metrics listener");
            let port = listener.local_addr().expect("metrics address").port();
            drop(listener);

            crate::metrics::install(port, 1).expect("install real exporter");
            let heartbeat = StartupReceipt::test_value().spawn_refresh(1);
            let url = format!("http://127.0.0.1:{port}/metrics");
            let first = reqwest::get(&url)
                .await
                .expect("first scrape")
                .text()
                .await
                .expect("first scrape body");
            assert!(first.contains(
                "buzz_startup_phase_terminal{phase=\"process_telemetry\",result=\"success\"} 1"
            ));
            let first_duration = first
                .lines()
                .find(|line| {
                    line.starts_with(
                        "buzz_startup_phase_duration_seconds{phase=\"process_telemetry\"} ",
                    )
                })
                .expect("first duration receipt")
                .to_owned();

            tokio::time::sleep(Duration::from_millis(1500)).await;
            let second = reqwest::get(&url)
                .await
                .expect("second scrape")
                .text()
                .await
                .expect("second scrape body");
            assert!(second.contains(
                "buzz_startup_phase_terminal{phase=\"process_telemetry\",result=\"success\"} 1"
            ));
            let second_duration = second
                .lines()
                .find(|line| {
                    line.starts_with(
                        "buzz_startup_phase_duration_seconds{phase=\"process_telemetry\"} ",
                    )
                })
                .expect("second duration receipt");
            assert_eq!(second_duration, first_duration);
            heartbeat.shutdown().await;

            tokio::time::sleep(Duration::from_millis(1500)).await;
            let after_shutdown = reqwest::get(&url)
                .await
                .expect("post-shutdown scrape")
                .text()
                .await
                .expect("post-shutdown scrape body");
            assert!(!after_shutdown.lines().any(|line| {
                !line.starts_with('#') && line.starts_with("buzz_startup_phase_terminal{")
            }));
            assert!(!after_shutdown.lines().any(|line| {
                !line.starts_with('#') && line.starts_with("buzz_startup_phase_duration_seconds{")
            }));
            return;
        }

        crate::test_support::run_exact_test_child(
            "lifecycle::tests::heartbeat_survives_real_idle_eviction",
            CHILD_ENV,
        );
    }
}
