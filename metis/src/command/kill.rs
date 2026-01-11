use crate::client::MetisClientInterface;
use anyhow::{bail, Result};
use metis_common::MetisId;

pub async fn run(client: &dyn MetisClientInterface, session: String) -> Result<()> {
    let session_id: MetisId = session.trim().to_string();
    if session_id.is_empty() {
        bail!("Session ID must not be empty.");
    }

    let response = client.kill_session(&session_id).await?;

    println!(
        "Kill request for session '{}' acknowledged: {}",
        response.session_id, response.status
    );

    Ok(())
}
