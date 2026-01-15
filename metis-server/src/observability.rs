use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn init_metrics_recorder() -> PrometheusHandle {
    PROMETHEUS_HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("failed to install metrics recorder")
        })
        .clone()
}

pub fn ensure_metrics_recorder() {
    let _ = init_metrics_recorder();
}
