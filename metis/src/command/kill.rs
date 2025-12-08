use crate::client::MetisClientInterface;
use anyhow::{bail, Result};

pub async fn run(client: &dyn MetisClientInterface, job: String) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }

    let response = client.kill_job(job_id).await?;

    println!(
        "Kill request for job '{}' acknowledged: {}",
        response.job_id, response.status
    );

    Ok(())
}
