use crate::{client::MetisClientInterface, command::output::CommandContext};
use anyhow::Result;
use metis_common::TaskId;

pub async fn run(
    client: &dyn MetisClientInterface,
    job: TaskId,
    _context: &CommandContext,
) -> Result<()> {
    let response = client.kill_job(&job).await?;

    println!(
        "Kill request for job '{}' acknowledged: {}",
        response.job_id, response.status
    );

    Ok(())
}
