use crate::{
    client::MetisClientInterface,
    command::output::{render_job_records, CommandContext},
};
use anyhow::Result;
use metis_common::TaskId;
use std::io::{self, Write};

pub async fn run(
    client: &dyn MetisClientInterface,
    job: TaskId,
    context: &CommandContext,
) -> Result<()> {
    let response = client.kill_job(&job).await?;
    let job = client.get_job(&response.job_id).await?;

    let mut buffer = Vec::new();
    render_job_records(context.output_format, &[job], &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    Ok(())
}
