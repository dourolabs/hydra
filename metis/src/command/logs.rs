use crate::{client::MetisClient, command::spawn::stream_job_logs_via_server, config::AppConfig};
use anyhow::{bail, Result};

pub async fn run(config: &AppConfig, job: String, watch: bool) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let job_id = job_id.to_string();
    let client = MetisClient::from_config(config)?;

    if watch {
        println!("Streaming logs for job '{job_id}' via metis-server…");
    } else {
        println!("Fetching logs for job '{job_id}' via metis-server…");
    }

    stream_job_logs_via_server(&client, &job_id, watch).await
}
