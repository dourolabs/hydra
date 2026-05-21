use crate::{
    client::HydraClientInterface,
    command::output::{render, CommandContext, SessionRecords},
    output_writer::write_stdout,
};
use anyhow::Result;
use hydra_common::SessionId;

pub async fn run(
    client: &dyn HydraClientInterface,
    session: SessionId,
    context: &CommandContext,
) -> Result<()> {
    let response = client.kill_session(&session).await?;
    let session = client.get_session(&response.session_id).await?;

    let mut buffer = Vec::new();
    render(
        SessionRecords(&[session]),
        context.output_format,
        &mut buffer,
    )?;
    write_stdout(&buffer)?;

    Ok(())
}
