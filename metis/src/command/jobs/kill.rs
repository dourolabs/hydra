use crate::client::MetisClientInterface;
use anyhow::Result;
use metis_common::TaskId;

pub async fn run(client: &dyn MetisClientInterface, job: TaskId) -> Result<()> {
    let response = client.kill_job(&job).await?;

    println!(
        "Kill request for job '{}' acknowledged: {}",
        response.job_id, response.status
    );

    Ok(())
}
