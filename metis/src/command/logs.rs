use crate::{client::MetisClientInterface, command::spawn::stream_job_logs_via_server};
use anyhow::{bail, Result};

pub async fn run(client: &dyn MetisClientInterface, job: String, watch: bool) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let job_id = job_id.to_string();

    if watch {
        println!("Streaming logs for job '{job_id}' via metis-server…");
    } else {
        println!("Fetching logs for job '{job_id}' via metis-server…");
    }

    stream_job_logs_via_server(client, &job_id, watch).await
}
