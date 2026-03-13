use crate::{
    client::MetisClientInterface,
    command::output::{render_session_records, CommandContext},
};
use anyhow::Result;
use metis_common::SessionId;
use std::io::{self, Write};

pub async fn run(
    client: &dyn MetisClientInterface,
    session: SessionId,
    context: &CommandContext,
) -> Result<()> {
    let response = client.kill_session(&session).await?;
    let session = client.get_session(&response.session_id).await?;

    let mut buffer = Vec::new();
    render_session_records(context.output_format, &[session], &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    Ok(())
}
