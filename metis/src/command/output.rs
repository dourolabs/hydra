use crate::{client::MetisClient, config::AppConfig};
use anyhow::{bail, Result};

pub async fn run(config: &AppConfig, job: String) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let job_id = job_id.to_string();
    let client = MetisClient::from_config(config)?;

    println!("Fetching output for job '{}' via metis-server…", job_id);

    let response = client.get_job_output(&job_id).await?;
    println!("\nLast agent message:\n{}\n", response.output.last_message);
    println!("Patch:\n{}", response.output.patch);

    Ok(())
}
