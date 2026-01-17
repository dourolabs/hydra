#[cfg(target_arch = "wasm32")]
mod client;

#[cfg(any(target_arch = "wasm32", test))]
use metis_common::MetisId;

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, PartialEq)]
struct JobSummary {
    id: MetisId,
    name: String,
    status: String,
    queue: String,
    last_run: String,
}

#[cfg(any(target_arch = "wasm32", test))]
fn filter_jobs(
    jobs: &[JobSummary],
    agent_filter: Option<&str>,
    status_filter: Option<&str>,
) -> Vec<JobSummary> {
    jobs.iter()
        .filter(|job| {
            agent_filter.is_none_or(|agent| job.queue == agent)
                && status_filter.is_none_or(|status| job.status == status)
        })
        .cloned()
        .collect()
}

#[cfg(target_arch = "wasm32")]
#[allow(non_snake_case)]
mod web_app {
    use crate::{client, filter_jobs, JobSummary};
    use dioxus::prelude::*;
    use metis_common::{
        jobs::JobRecord,
        task_status::{Status, TaskStatusLog},
        MetisId,
    };
    use std::collections::{BTreeSet, HashMap};

    pub fn launch() {
        dioxus_web::launch(App);
    }

    fn App(cx: Scope) -> Element {
        let dashboard_state = use_state(&cx, || DashboardState::Loading);

        use_effect(&cx, (), |_| {
            to_owned![dashboard_state];
            async move {
                let next_state = match client::load_dashboard().await {
                    Ok(data) => DashboardState::Loaded(map_dashboard(data)),
                    Err(err) => DashboardState::Error(err.to_string()),
                };
                dashboard_state.set(next_state);
            }
        });

        let selected_agent = use_state(&cx, || None::<String>);
        let selected_status = use_state(&cx, || None::<String>);

        let agent_filter = selected_agent.get().clone();
        let status_filter = selected_status.get().clone();
        let filters_active = agent_filter.is_some() || status_filter.is_some();

        let (metrics, jobs, queues, workers, status_message, agent_names) =
            match dashboard_state.get() {
                DashboardState::Loading => (
                    loading_metrics(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Some("Loading live data...".to_string()),
                    Vec::new(),
                ),
                DashboardState::Error(err) => (
                    loading_metrics(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Some(format!("Unable to load dashboard data: {err}")),
                    Vec::new(),
                ),
                DashboardState::Loaded(data) => {
                    let filtered_jobs = filter_jobs(
                        &data.jobs,
                        agent_filter.as_deref(),
                        status_filter.as_deref(),
                    );
                    let (metrics, queues, workers) =
                        build_panels(&filtered_jobs, &data.agent_names, !filters_active);
                    (
                        metrics,
                        filtered_jobs,
                        queues,
                        workers,
                        None,
                        data.agent_names.clone(),
                    )
                }
            };

        let selected_agent_handle = selected_agent.clone();
        let selected_agent_clear = selected_agent.clone();
        let selected_status_handle = selected_status.clone();
        let selected_status_clear = selected_status.clone();
        let selected_reset_agent = selected_agent.clone();
        let selected_reset_status = selected_status.clone();

        cx.render(rsx!(
            style { include_str!("../assets/app.css") }
            div { class: "app-shell",
                header { class: "app-header",
                    div { class: "brand",
                        div { class: "brand-mark", "M" }
                        div {
                            h1 { "Metis" }
                            p { "Orchestration control room" }
                        }
                    }
                    div { class: "header-actions",
                        button { class: "ghost", "Live" }
                        button { class: "primary", "Create run" }
                    }
                }
                div { class: "app-body",
                    nav { class: "side-nav",
                        p { class: "nav-section", "Core" }
                        a { class: "nav-item active", href: "#", "Dashboard" }
                        a { class: "nav-item", href: "#", "Jobs" }
                        a { class: "nav-item", href: "#", "Queues" }
                        a { class: "nav-item", href: "#", "Workers" }
                        p { class: "nav-section", "Operations" }
                        a { class: "nav-item", href: "#", "Schedules" }
                        a { class: "nav-item", href: "#", "Runs" }
                        a { class: "nav-item", href: "#", "Audit log" }
                    }
                    main { class: "main-content",
                        section { class: "overview",
                            h2 { "Mission pulse" }
                            div { class: "overview-grid",
                                metrics.iter().map(|metric| rsx!(
                                    MetricCard {
                                        title: metric.title.clone(),
                                        value: metric.value.clone(),
                                        trend: metric.trend.clone(),
                                    }
                                ))
                            }
                        }
                        section { class: "panel-grid",
                            div { class: "filter-bar",
                                div { class: "filter-group",
                                    label { "Agent" }
                                    select {
                                        value: "{agent_filter.clone().unwrap_or_default()}",
                                        onchange: move |event| {
                                            let value = event.value.clone();
                                            selected_agent_handle.set(if value.is_empty() { None } else { Some(value) });
                                        },
                                        option { value: "", "All agents" }
                                        agent_names.iter().map(|name| rsx!(
                                            option { value: "{name}", "{name}" }
                                        ))
                                    }
                                    button {
                                        class: "ghost filter-clear",
                                        disabled: agent_filter.is_none(),
                                        onclick: move |_| selected_agent_clear.set(None),
                                        "Clear"
                                    }
                                }
                                div { class: "filter-group",
                                    label { "Status" }
                                    select {
                                        value: "{status_filter.clone().unwrap_or_default()}",
                                        onchange: move |event| {
                                            let value = event.value.clone();
                                            selected_status_handle.set(if value.is_empty() { None } else { Some(value) });
                                        },
                                        option { value: "", "All statuses" }
                                        option { value: "pending", "Pending" }
                                        option { value: "running", "Running" }
                                        option { value: "complete", "Complete" }
                                        option { value: "failed", "Failed" }
                                    }
                                    button {
                                        class: "ghost filter-clear",
                                        disabled: status_filter.is_none(),
                                        onclick: move |_| selected_status_clear.set(None),
                                        "Clear"
                                    }
                                }
                                button {
                                    class: "ghost filter-reset",
                                    disabled: !filters_active,
                                    onclick: move |_| {
                                        selected_reset_agent.set(None);
                                        selected_reset_status.set(None);
                                    },
                                    "Reset all"
                                }
                            }
                            JobPanel { jobs: jobs.clone(), status: status_message.clone() }
                            QueuePanel { queues: queues.clone(), status: status_message.clone() }
                            WorkerPanel { workers: workers.clone(), status: status_message.clone() }
                        }
                    }
                }
            }
        ))
    }

    #[derive(Clone, PartialEq)]
    enum DashboardState {
        Loading,
        Loaded(DashboardViewModel),
        Error(String),
    }

    #[derive(Clone, PartialEq)]
    struct DashboardViewModel {
        jobs: Vec<JobSummary>,
        agent_names: Vec<String>,
    }

    #[derive(Clone, PartialEq)]
    struct MetricCardData {
        title: String,
        value: String,
        trend: String,
    }

    #[derive(Clone, PartialEq)]
    struct QueueSummary {
        id: String,
        name: String,
        depth: u32,
        active_workers: u32,
        sla: String,
    }

    #[derive(Clone, PartialEq)]
    struct WorkerSummary {
        id: String,
        name: String,
        state: String,
        active_jobs: u32,
        heartbeat: String,
    }

    #[derive(Clone, PartialEq, Default)]
    struct QueueStats {
        pending: u32,
        running: u32,
        total: u32,
    }

    #[derive(Props, PartialEq)]
    struct MetricCardProps {
        title: String,
        value: String,
        trend: String,
    }

    fn MetricCard(cx: Scope<MetricCardProps>) -> Element {
        cx.render(rsx!(
            div { class: "metric-card",
                p { class: "metric-title", "{cx.props.title}" }
                h3 { class: "metric-value", "{cx.props.value}" }
                p { class: "metric-trend", "{cx.props.trend}" }
            }
        ))
    }

    #[derive(Props, PartialEq)]
    struct JobPanelProps {
        jobs: Vec<JobSummary>,
        status: Option<String>,
    }

    fn JobPanel(cx: Scope<JobPanelProps>) -> Element {
        cx.render(rsx!(
            section { class: "panel",
                header {
                    h3 { "Jobs" }
                    button { class: "ghost", "View all" }
                }
                div { class: "panel-table",
                    if let Some(message) = &cx.props.status {
                        p { class: "panel-status", "{message}" }
                    } else if cx.props.jobs.is_empty() {
                        p { class: "panel-status", "No jobs found" }
                    } else {
                        div { class: "table-row table-head",
                            span { "Job" }
                            span { "Queue" }
                            span { "Status" }
                            span { "Last run" }
                        }
                        cx.props.jobs.iter().map(|job| rsx!(
                            div { class: "table-row",
                                span { class: "table-title",
                                    "{job.name}"
                                    small { "{job.id}" }
                                }
                                span { "{job.queue}" }
                                span { class: "status-pill", "{job.status}" }
                                span { "{job.last_run}" }
                            }
                        ))
                    }
                }
            }
        ))
    }

    #[derive(Props, PartialEq)]
    struct QueuePanelProps {
        queues: Vec<QueueSummary>,
        status: Option<String>,
    }

    fn QueuePanel(cx: Scope<QueuePanelProps>) -> Element {
        cx.render(rsx!(
            section { class: "panel",
                header {
                    h3 { "Queues" }
                    button { class: "ghost", "Tune" }
                }
                div { class: "panel-table",
                    if let Some(message) = &cx.props.status {
                        p { class: "panel-status", "{message}" }
                    } else if cx.props.queues.is_empty() {
                        p { class: "panel-status", "No queues configured" }
                    } else {
                        div { class: "table-row table-head",
                            span { "Queue" }
                            span { "Depth" }
                            span { "Workers" }
                            span { "SLA" }
                        }
                        cx.props.queues.iter().map(|queue| rsx!(
                            div { class: "table-row",
                                span { class: "table-title",
                                    "{queue.name}"
                                    small { "{queue.id}" }
                                }
                                span { "{queue.depth}" }
                                span { "{queue.active_workers}" }
                                span { "{queue.sla}" }
                            }
                        ))
                    }
                }
            }
        ))
    }

    #[derive(Props, PartialEq)]
    struct WorkerPanelProps {
        workers: Vec<WorkerSummary>,
        status: Option<String>,
    }

    fn WorkerPanel(cx: Scope<WorkerPanelProps>) -> Element {
        cx.render(rsx!(
            section { class: "panel",
                header {
                    h3 { "Workers" }
                    button { class: "ghost", "Scale" }
                }
                div { class: "panel-table",
                    if let Some(message) = &cx.props.status {
                        p { class: "panel-status", "{message}" }
                    } else if cx.props.workers.is_empty() {
                        p { class: "panel-status", "No workers configured" }
                    } else {
                        div { class: "table-row table-head",
                            span { "Worker" }
                            span { "State" }
                            span { "Active" }
                            span { "Heartbeat" }
                        }
                        cx.props.workers.iter().map(|worker| rsx!(
                            div { class: "table-row",
                                span { class: "table-title",
                                    "{worker.name}"
                                    small { "{worker.id}" }
                                }
                                span { class: "status-pill", "{worker.state}" }
                                span { "{worker.active_jobs}" }
                                span { "{worker.heartbeat}" }
                            }
                        ))
                    }
                }
            }
        ))
    }

    fn map_dashboard(data: client::DashboardResponse) -> DashboardViewModel {
        let jobs = data.jobs.jobs;
        let agents = data.agents.agents;

        let mut agent_names: BTreeSet<String> =
            agents.iter().map(|agent| agent.name.clone()).collect();
        for job in &jobs {
            agent_names.insert(job_queue_name(job));
        }

        DashboardViewModel {
            jobs: jobs.iter().map(job_summary).collect(),
            agent_names: agent_names.into_iter().collect(),
        }
    }

    fn build_panels(
        jobs: &[JobSummary],
        agent_names: &[String],
        include_idle_agents: bool,
    ) -> (Vec<MetricCardData>, Vec<QueueSummary>, Vec<WorkerSummary>) {
        let mut queue_stats: HashMap<String, QueueStats> = HashMap::new();
        let mut running_jobs = 0;
        let mut pending_jobs = 0;

        for job in jobs {
            let stats = queue_stats.entry(job.queue.clone()).or_default();
            stats.total += 1;

            match job.status.as_str() {
                "pending" => {
                    stats.pending += 1;
                    pending_jobs += 1;
                }
                "running" => {
                    stats.running += 1;
                    running_jobs += 1;
                }
                _ => {}
            }
        }

        let mut queue_names: BTreeSet<String> = BTreeSet::new();
        if include_idle_agents {
            queue_names.extend(agent_names.iter().cloned());
        }
        queue_names.extend(queue_stats.keys().cloned());

        let queues = queue_names
            .iter()
            .map(|name| {
                let stats = queue_stats.get(name).cloned().unwrap_or_default();
                QueueSummary {
                    id: name.clone(),
                    name: name.clone(),
                    depth: stats.pending,
                    active_workers: stats.running,
                    sla: "n/a".to_string(),
                }
            })
            .collect::<Vec<_>>();

        let worker_names: BTreeSet<String> = if include_idle_agents {
            agent_names.iter().cloned().collect()
        } else {
            queue_stats.keys().cloned().collect()
        };

        let workers = worker_names
            .iter()
            .map(|name| {
                let stats = queue_stats.get(name).cloned().unwrap_or_default();
                let state = if stats.running > 0 { "busy" } else { "idle" };
                WorkerSummary {
                    id: name.clone(),
                    name: name.clone(),
                    state: state.to_string(),
                    active_jobs: stats.running,
                    heartbeat: "n/a".to_string(),
                }
            })
            .collect::<Vec<_>>();

        let agent_count = if include_idle_agents {
            agent_names.len()
        } else {
            queue_names.len()
        };

        let metrics = vec![
            MetricCardData {
                title: "Active jobs".to_string(),
                value: running_jobs.to_string(),
                trend: format!("{running_jobs} running"),
            },
            MetricCardData {
                title: "Queued jobs".to_string(),
                value: pending_jobs.to_string(),
                trend: format!("{pending_jobs} pending"),
            },
            MetricCardData {
                title: "Agent queues".to_string(),
                value: agent_count.to_string(),
                trend: format!("{agent_count} configured"),
            },
            MetricCardData {
                title: "Total jobs".to_string(),
                value: jobs.len().to_string(),
                trend: format!("{} total", jobs.len()),
            },
        ];

        (metrics, queues, workers)
    }

    fn job_summary(job: &JobRecord) -> JobSummary {
        JobSummary {
            id: MetisId::try_from(job.id.to_string()).expect("task id should be valid"),
            name: job.task.prompt.clone(),
            status: status_label(job.status_log.current_status()).to_string(),
            queue: job_queue_name(job),
            last_run: last_run_label(&job.status_log),
        }
    }

    fn job_queue_name(job: &JobRecord) -> String {
        job.task
            .image
            .clone()
            .unwrap_or_else(|| "default".to_string())
    }

    fn status_label(status: Status) -> &'static str {
        match status {
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Complete => "complete",
            Status::Failed => "failed",
        }
    }

    fn last_run_label(status_log: &TaskStatusLog) -> String {
        status_log
            .end_time()
            .or(status_log.start_time())
            .or(status_log.creation_time())
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn loading_metrics() -> Vec<MetricCardData> {
        vec![
            MetricCardData {
                title: "Active jobs".to_string(),
                value: "...".to_string(),
                trend: "Loading".to_string(),
            },
            MetricCardData {
                title: "Queued jobs".to_string(),
                value: "...".to_string(),
                trend: "Loading".to_string(),
            },
            MetricCardData {
                title: "Agent queues".to_string(),
                value: "...".to_string(),
                trend: "Loading".to_string(),
            },
            MetricCardData {
                title: "Total jobs".to_string(),
                value: "...".to_string(),
                trend: "Loading".to_string(),
            },
        ]
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    web_app::launch();

    #[cfg(not(target_arch = "wasm32"))]
    println!("metis-dashboard targets wasm32; build with wasm32-unknown-unknown");
}

#[cfg(test)]
mod tests {
    use super::{filter_jobs, JobSummary};
    use metis_common::TaskId;

    fn sample_job(queue: &str, status: &str) -> JobSummary {
        JobSummary {
            id: TaskId::new().into(),
            name: "demo".to_string(),
            status: status.to_string(),
            queue: queue.to_string(),
            last_run: "now".to_string(),
        }
    }

    #[test]
    fn filter_jobs_intersection() {
        let jobs = vec![
            sample_job("alpha", "pending"),
            sample_job("alpha", "running"),
            sample_job("beta", "pending"),
        ];

        let filtered = filter_jobs(&jobs, Some("alpha"), Some("running"));

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].queue, "alpha");
        assert_eq!(filtered[0].status, "running");
    }

    #[test]
    fn filter_jobs_allows_empty_filters() {
        let jobs = vec![sample_job("alpha", "pending"), sample_job("beta", "failed")];

        let filtered = filter_jobs(&jobs, None, None);

        assert_eq!(filtered.len(), 2);
    }
}
