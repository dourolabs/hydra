use crate::config::{build_kube_client, AppConfig};
use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use k8s_openapi::api::batch::v1::{Job, JobStatus};
use kube::{api::ListParams, Api};

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 9;

pub async fn run(config: &AppConfig) -> Result<()> {
    let namespace = &config.metis.namespace;
    let client = build_kube_client(&config.kubernetes).await?;
    let jobs_api: Api<Job> = Api::namespaced(client, namespace);

    let mut jobs = jobs_api
        .list(&ListParams::default().labels("metis-worker"))
        .await?
        .into_iter()
        .collect::<Vec<_>>();

    if jobs.is_empty() {
        println!("No Metis jobs found in namespace '{}'.", namespace);
        return Ok(());
    }

    jobs.sort_by(|a, b| job_reference_time(b).cmp(&job_reference_time(a)));

    println!(
        "{:<name_width$} {:<status_width$} {}",
        "NAME",
        "STATUS",
        "RUNTIME",
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH
    );
    println!("{}", "-".repeat(NAME_WIDTH + STATUS_WIDTH + 9));

    let now = Utc::now();
    for job in jobs {
        let name = job
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string());
        let status = job_status(&job);
        let runtime = job_runtime(&job, now)
            .map(format_duration)
            .unwrap_or_else(|| "-".into());

        println!(
            "{:<name_width$} {:<status_width$} {}",
            name,
            status,
            runtime,
            name_width = NAME_WIDTH,
            status_width = STATUS_WIDTH
        );
    }

    Ok(())
}

fn job_status(job: &Job) -> &'static str {
    if let Some(status) = job.status.as_ref() {
        if status.succeeded.unwrap_or(0) > 0 {
            return "complete";
        }
        if status.failed.unwrap_or(0) > 0 {
            return "failed";
        }
    }

    "running"
}

fn job_runtime(job: &Job, now: DateTime<Utc>) -> Option<ChronoDuration> {
    let start = job_reference_time(job)?;
    let end = job_end_time(job).unwrap_or(now);

    if end < start {
        return Some(ChronoDuration::zero());
    }

    Some(end - start)
}

fn job_reference_time(job: &Job) -> Option<DateTime<Utc>> {
    job.status
        .as_ref()
        .and_then(|status| status.start_time.as_ref())
        .map(|time| time.0.clone())
        .or_else(|| {
            job.metadata
                .creation_timestamp
                .as_ref()
                .map(|time| time.0.clone())
        })
}

fn job_end_time(job: &Job) -> Option<DateTime<Utc>> {
    let status = job.status.as_ref()?;

    if status.succeeded.unwrap_or(0) > 0 {
        if let Some(completion_time) = status.completion_time.as_ref() {
            return Some(completion_time.0.clone());
        }

        if let Some(time) = condition_time(status, "Complete") {
            return Some(time);
        }
    }

    if status.failed.unwrap_or(0) > 0 {
        if let Some(time) = condition_time(status, "Failed") {
            return Some(time);
        }
    }

    None
}

fn condition_time(status: &JobStatus, kind: &str) -> Option<DateTime<Utc>> {
    status
        .conditions
        .as_ref()
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == kind)
                .and_then(|condition| condition.last_transition_time.as_ref())
        })
        .map(|time| time.0.clone())
}

fn format_duration(duration: ChronoDuration) -> String {
    let total_seconds = duration.num_seconds();
    if total_seconds <= 0 {
        return "0s".to_string();
    }

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}
