#[cfg(target_arch = "wasm32")]
mod client;

#[cfg(target_arch = "wasm32")]
#[allow(non_snake_case)]
mod web_app {
    use crate::client;
    use dioxus::prelude::*;
    use metis_common::{
        jobs::JobRecord,
        task_status::{Status, TaskStatusLog},
        MetisId,
    };
    use std::collections::{BTreeSet, HashMap};

    static MAIN_CSS: Asset = asset!("/assets/app.css");

    pub fn launch() {
        dioxus::launch(App);
    }

    #[component]
    fn App() -> Element {
        let dashboard = use_resource(|| async move { client::load_dashboard().await });

        let (metrics, jobs, queues, workers, status_message) = match &*dashboard.read() {
            None => (
                loading_metrics(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Some("Loading live data...".to_string()),
            ),
            Some(Err(err)) => (
                loading_metrics(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Some(format!("Unable to load dashboard data: {err}")),
            ),
            Some(Ok(data)) => {
                let view_model = map_dashboard(data);
                (
                    view_model.metrics,
                    view_model.jobs,
                    view_model.queues,
                    view_model.workers,
                    None,
                )
            }
        };

        rsx!(
            document::Stylesheet { href: MAIN_CSS }
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
                                for metric in metrics.iter() {
                                    MetricCard {
                                        title: metric.title.clone(),
                                        value: metric.value.clone(),
                                        trend: metric.trend.clone(),
                                    }
                                }
                            }
                        }
                        section { class: "panel-grid",
                            JobPanel { jobs: jobs.clone(), status: status_message.clone() }
                            QueuePanel { queues: queues.clone(), status: status_message.clone() }
                            WorkerPanel { workers: workers.clone(), status: status_message.clone() }
                        }
                    }
                }
            }
        )
    }

    #[derive(Clone, PartialEq)]
    struct DashboardViewModel {
        metrics: Vec<MetricCardData>,
        jobs: Vec<JobSummary>,
        queues: Vec<QueueSummary>,
        workers: Vec<WorkerSummary>,
    }

    #[derive(Clone, PartialEq)]
    struct MetricCardData {
        title: String,
        value: String,
        trend: String,
    }

    #[derive(Clone, PartialEq)]
    struct JobSummary {
        id: MetisId,
        name: String,
        status: String,
        queue: String,
        last_run: String,
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

    #[component]
    fn MetricCard(title: String, value: String, trend: String) -> Element {
        rsx!(
            div { class: "metric-card",
                p { class: "metric-title", "{title}" }
                h3 { class: "metric-value", "{value}" }
                p { class: "metric-trend", "{trend}" }
            }
        )
    }

    #[component]
    fn JobPanel(jobs: Vec<JobSummary>, status: Option<String>) -> Element {
        rsx!(
            section { class: "panel",
                header {
                    h3 { "Jobs" }
                    button { class: "ghost", "View all" }
                }
                div { class: "panel-table",
                    if let Some(message) = status.as_ref() {
                        p { class: "panel-status", "{message}" }
                    } else if jobs.is_empty() {
                        p { class: "panel-status", "No jobs found" }
                    } else {
                        div { class: "table-row table-head",
                            span { "Job" }
                            span { "Queue" }
                            span { "Status" }
                            span { "Last run" }
                        }
                        for job in jobs.iter() {
                            div { class: "table-row",
                                span { class: "table-title",
                                    "{job.name}"
                                    small { "{job.id}" }
                                }
                                span { "{job.queue}" }
                                span { class: "status-pill", "{job.status}" }
                                span { "{job.last_run}" }
                            }
                        }
                    }
                }
            }
        )
    }

    #[component]
    fn QueuePanel(queues: Vec<QueueSummary>, status: Option<String>) -> Element {
        rsx!(
            section { class: "panel",
                header {
                    h3 { "Queues" }
                    button { class: "ghost", "Tune" }
                }
                div { class: "panel-table",
                    if let Some(message) = status.as_ref() {
                        p { class: "panel-status", "{message}" }
                    } else if queues.is_empty() {
                        p { class: "panel-status", "No queues configured" }
                    } else {
                        div { class: "table-row table-head",
                            span { "Queue" }
                            span { "Depth" }
                            span { "Workers" }
                            span { "SLA" }
                        }
                        for queue in queues.iter() {
                            div { class: "table-row",
                                span { class: "table-title",
                                    "{queue.name}"
                                    small { "{queue.id}" }
                                }
                                span { "{queue.depth}" }
                                span { "{queue.active_workers}" }
                                span { "{queue.sla}" }
                            }
                        }
                    }
                }
            }
        )
    }

    #[component]
    fn WorkerPanel(workers: Vec<WorkerSummary>, status: Option<String>) -> Element {
        rsx!(
            section { class: "panel",
                header {
                    h3 { "Workers" }
                    button { class: "ghost", "Scale" }
                }
                div { class: "panel-table",
                    if let Some(message) = status.as_ref() {
                        p { class: "panel-status", "{message}" }
                    } else if workers.is_empty() {
                        p { class: "panel-status", "No workers configured" }
                    } else {
                        div { class: "table-row table-head",
                            span { "Worker" }
                            span { "State" }
                            span { "Active" }
                            span { "Heartbeat" }
                        }
                        for worker in workers.iter() {
                            div { class: "table-row",
                                span { class: "table-title",
                                    "{worker.name}"
                                    small { "{worker.id}" }
                                }
                                span { class: "status-pill", "{worker.state}" }
                                span { "{worker.active_jobs}" }
                                span { "{worker.heartbeat}" }
                            }
                        }
                    }
                }
            }
        )
    }

    fn map_dashboard(data: &client::DashboardResponse) -> DashboardViewModel {
        let jobs = &data.jobs.jobs;
        let agents = &data.agents.agents;

        let mut queue_stats: HashMap<String, QueueStats> = HashMap::new();
        let mut running_jobs = 0;
        let mut pending_jobs = 0;

        for job in jobs {
            let queue_name = job_queue_name(job);
            let stats = queue_stats.entry(queue_name).or_default();
            stats.total += 1;

            match job.status_log.current_status() {
                Status::Pending => {
                    stats.pending += 1;
                    pending_jobs += 1;
                }
                Status::Running => {
                    stats.running += 1;
                    running_jobs += 1;
                }
                Status::Complete | Status::Failed => {}
            }
        }

        let mut queue_names: BTreeSet<String> =
            agents.iter().map(|agent| agent.name.clone()).collect();
        for name in queue_stats.keys() {
            queue_names.insert(name.clone());
        }

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

        let workers = agents
            .iter()
            .map(|agent| {
                let stats = queue_stats.get(&agent.name).cloned().unwrap_or_default();
                let state = if stats.running > 0 { "busy" } else { "idle" };
                WorkerSummary {
                    id: agent.name.clone(),
                    name: agent.name.clone(),
                    state: state.to_string(),
                    active_jobs: stats.running,
                    heartbeat: "n/a".to_string(),
                }
            })
            .collect::<Vec<_>>();

        let job_summaries = jobs.iter().map(job_summary).collect::<Vec<_>>();

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
                value: agents.len().to_string(),
                trend: format!("{} configured", agents.len()),
            },
            MetricCardData {
                title: "Total jobs".to_string(),
                value: jobs.len().to_string(),
                trend: format!("{} total", jobs.len()),
            },
        ];

        DashboardViewModel {
            metrics,
            jobs: job_summaries,
            queues,
            workers,
        }
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
