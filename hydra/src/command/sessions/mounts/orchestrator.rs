//! Drives one mount phase: wraps it in an optional timeout, routes the
//! result through the tracked / fatal / Ok policy, and emits the
//! phase-bracketed status log lines.

use super::{MountError, MountResult, Phase};
use anyhow::{anyhow, Result};
use std::future::Future;
use std::time::Instant;

/// Run a single mount phase.
///
/// - Returns `Ok(())` on success and on tracked failure (the orchestrator
///   pushes the failure onto `errors` and the caller continues).
/// - Returns `Err(source)` only on a fatal failure — the caller should
///   abort the worker run.
/// - A timeout is mapped to [`MountError::tracked`].
pub async fn run_phase<F, Fut>(phase: Phase, call: F, errors: &mut Vec<anyhow::Error>) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = MountResult>,
{
    log_status(format_args!("Phase: {} — starting", phase.label));
    let start = Instant::now();
    let result = match phase.timeout {
        Some(t) => match tokio::time::timeout(t, call()).await {
            Ok(r) => r,
            Err(_) => Err(MountError::tracked(anyhow!(
                "{} timed out after {}s",
                phase.label,
                t.as_secs()
            ))),
        },
        None => call().await,
    };
    let elapsed = start.elapsed().as_secs_f64();
    match result {
        Ok(()) => {
            log_status(format_args!(
                "Phase: {} — completed ({elapsed:.2}s)",
                phase.label
            ));
            Ok(())
        }
        Err(MountError {
            source,
            fatal: false,
        }) => {
            log_status(format_args!(
                "Phase: {} — failed ({elapsed:.2}s): {source}",
                phase.label
            ));
            errors.push(source);
            Ok(())
        }
        Err(MountError {
            source,
            fatal: true,
        }) => {
            log_status(format_args!(
                "Phase: {} — fatal ({elapsed:.2}s): {source}",
                phase.label
            ));
            Err(source)
        }
    }
}

/// Emit a status line for a phase event. Uses `tracing::info!` so tests
/// can capture log output via a `tracing` subscriber, and production
/// runs route the line through the worker's existing fmt subscriber
/// (initialized in `worker_run::run`) and into the session log file.
fn log_status(args: std::fmt::Arguments<'_>) {
    tracing::info!(target: "hydra::mounts::orchestrator", "{}", args);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tracing::field::{Field, Visit};
    use tracing::subscriber::{set_default, DefaultGuard};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::Registry;
    use tracing_subscriber::Layer;

    /// Captures the `message` field of every event emitted while
    /// installed as the default subscriber.
    #[derive(Clone, Default)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl CaptureLayer {
        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl<S: Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = MessageVisitor(String::new());
            event.record(&mut visitor);
            self.events.lock().unwrap().push(visitor.0);
        }
    }

    struct MessageVisitor(String);

    impl Visit for MessageVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                use std::fmt::Write;
                let _ = write!(self.0, "{value:?}");
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.0.push_str(value);
            }
        }
    }

    fn install_capture() -> (CaptureLayer, DefaultGuard) {
        let layer = CaptureLayer::default();
        let subscriber = Registry::default().with(layer.clone());
        let guard = set_default(subscriber);
        (layer, guard)
    }

    fn phase(label: &'static str, timeout: Option<Duration>) -> Phase {
        Phase { label, timeout }
    }

    #[tokio::test]
    async fn happy_path_returns_ok_and_emits_completed_log() {
        let (capture, _guard) = install_capture();
        let mut errors = Vec::new();

        let result = run_phase(phase("happy", None), || async { Ok(()) }, &mut errors).await;

        assert!(result.is_ok());
        assert!(errors.is_empty());
        let events = capture.events();
        assert!(
            events.iter().any(|e| e == "Phase: happy — starting"),
            "missing starting line, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("Phase: happy — completed (")),
            "missing completed line, got {events:?}"
        );
    }

    #[tokio::test]
    async fn tracked_error_is_pushed_and_caller_continues() {
        let (capture, _guard) = install_capture();
        let mut errors = Vec::new();

        let result = run_phase(
            phase("tracky", None),
            || async { Err(MountError::tracked(anyhow!("boom"))) },
            &mut errors,
        )
        .await;

        assert!(result.is_ok(), "tracked errors must not abort the caller");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].to_string(), "boom");
        let events = capture.events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("Phase: tracky — failed (") && e.ends_with(": boom")),
            "missing failed line, got {events:?}"
        );
    }

    #[tokio::test]
    async fn fatal_error_aborts_caller() {
        let (capture, _guard) = install_capture();
        let mut errors = Vec::new();

        let result = run_phase(
            phase("doomed", None),
            || async { Err(MountError::fatal(anyhow!("kaboom"))) },
            &mut errors,
        )
        .await;

        let err = result.expect_err("fatal errors must abort the caller");
        assert_eq!(err.to_string(), "kaboom");
        assert!(
            errors.is_empty(),
            "fatal failures must not also push onto the errors vec"
        );
        let events = capture.events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("Phase: doomed — fatal (") && e.ends_with(": kaboom")),
            "missing fatal line, got {events:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_is_mapped_to_tracked() {
        let (capture, _guard) = install_capture();
        let mut errors = Vec::new();

        let result = run_phase(
            phase("slow", Some(Duration::from_millis(10))),
            || async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(())
            },
            &mut errors,
        )
        .await;

        assert!(
            result.is_ok(),
            "timeouts are tracked, not fatal — caller continues"
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].to_string(), "slow timed out after 0s");
        let events = capture.events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("Phase: slow — failed (")
                    && e.contains("slow timed out after 0s")),
            "missing failed line, got {events:?}"
        );
    }

    #[tokio::test]
    async fn label_appears_in_every_log_line() {
        let (capture, _guard) = install_capture();
        let mut errors = Vec::new();

        let _ = run_phase(phase("labeled", None), || async { Ok(()) }, &mut errors).await;

        let events = capture.events();
        assert!(!events.is_empty(), "expected at least one log event");
        for event in &events {
            assert!(
                event.contains("labeled"),
                "log line missing label: {event:?}"
            );
        }
    }
}
