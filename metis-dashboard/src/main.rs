#[cfg(target_arch = "wasm32")]
#[allow(non_snake_case)]
mod web_app {
    use dioxus::prelude::*;
    use metis_common::MetisId;

    pub fn launch() {
        dioxus_web::launch(App);
    }

    fn App(cx: Scope) -> Element {
        let jobs = vec![
            JobSummary {
                id: metis_id("t-orbit"),
                name: "Ingest nightly telemetry",
                status: "running",
                queue: "signal",
                last_run: "3m ago",
            },
            JobSummary {
                id: metis_id("t-delta"),
                name: "Backfill revenue events",
                status: "paused",
                queue: "ledger",
                last_run: "2h ago",
            },
            JobSummary {
                id: metis_id("t-nova"),
                name: "Rebuild search index",
                status: "queued",
                queue: "search",
                last_run: "34m ago",
            },
        ];

        let queues = vec![
            QueueSummary {
                id: metis_id("t-signal"),
                name: "signal",
                depth: 24,
                active_workers: 6,
                sla: "99.2%",
            },
            QueueSummary {
                id: metis_id("t-ledger"),
                name: "ledger",
                depth: 8,
                active_workers: 3,
                sla: "97.4%",
            },
            QueueSummary {
                id: metis_id("t-search"),
                name: "search",
                depth: 42,
                active_workers: 4,
                sla: "98.6%",
            },
        ];

        let workers = vec![
            WorkerSummary {
                id: metis_id("t-axial"),
                name: "north-1a",
                state: "healthy",
                active_jobs: 3,
                heartbeat: "12s",
            },
            WorkerSummary {
                id: metis_id("t-echo"),
                name: "north-1b",
                state: "degraded",
                active_jobs: 1,
                heartbeat: "47s",
            },
            WorkerSummary {
                id: metis_id("t-glint"),
                name: "north-1c",
                state: "healthy",
                active_jobs: 4,
                heartbeat: "5s",
            },
        ];

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
                                MetricCard {
                                    title: "Active jobs",
                                    value: "18",
                                    trend: "+3 this hour",
                                }
                                MetricCard {
                                    title: "Queue depth",
                                    value: "74",
                                    trend: "Stable",
                                }
                                MetricCard {
                                    title: "Worker uptime",
                                    value: "99.4%",
                                    trend: "7d window",
                                }
                                MetricCard {
                                    title: "Alerts",
                                    value: "2",
                                    trend: "Needs review",
                                }
                            }
                        }
                        section { class: "panel-grid",
                            JobPanel { jobs: jobs.clone() }
                            QueuePanel { queues: queues.clone() }
                            WorkerPanel { workers: workers.clone() }
                        }
                    }
                }
            }
        ))
    }

    #[derive(Clone, PartialEq)]
    struct JobSummary {
        id: MetisId,
        name: &'static str,
        status: &'static str,
        queue: &'static str,
        last_run: &'static str,
    }

    #[derive(Clone, PartialEq)]
    struct QueueSummary {
        id: MetisId,
        name: &'static str,
        depth: u32,
        active_workers: u32,
        sla: &'static str,
    }

    #[derive(Clone, PartialEq)]
    struct WorkerSummary {
        id: MetisId,
        name: &'static str,
        state: &'static str,
        active_jobs: u32,
        heartbeat: &'static str,
    }

    #[derive(Props, PartialEq)]
    struct MetricCardProps {
        title: &'static str,
        value: &'static str,
        trend: &'static str,
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
    }

    fn JobPanel(cx: Scope<JobPanelProps>) -> Element {
        cx.render(rsx!(
            section { class: "panel",
                header {
                    h3 { "Jobs" }
                    button { class: "ghost", "View all" }
                }
                div { class: "panel-table",
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
        ))
    }

    #[derive(Props, PartialEq)]
    struct QueuePanelProps {
        queues: Vec<QueueSummary>,
    }

    fn QueuePanel(cx: Scope<QueuePanelProps>) -> Element {
        cx.render(rsx!(
            section { class: "panel",
                header {
                    h3 { "Queues" }
                    button { class: "ghost", "Tune" }
                }
                div { class: "panel-table",
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
        ))
    }

    #[derive(Props, PartialEq)]
    struct WorkerPanelProps {
        workers: Vec<WorkerSummary>,
    }

    fn WorkerPanel(cx: Scope<WorkerPanelProps>) -> Element {
        cx.render(rsx!(
            section { class: "panel",
                header {
                    h3 { "Workers" }
                    button { class: "ghost", "Scale" }
                }
                div { class: "panel-table",
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
        ))
    }

    fn metis_id(value: &str) -> MetisId {
        MetisId::try_from(value.to_string()).expect("valid placeholder MetisId")
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    web_app::launch();

    #[cfg(not(target_arch = "wasm32"))]
    println!("metis-dashboard targets wasm32; build with wasm32-unknown-unknown");
}
