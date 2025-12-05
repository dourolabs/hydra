use crate::{client::MetisClient, config::AppConfig};
use anyhow::Result;

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 9;
const RUNTIME_WIDTH: usize = 12;

pub async fn run(config: &AppConfig) -> Result<()> {
    let client = MetisClient::from_config(config)?;
    let response = client.list_jobs().await?;

    if response.jobs.is_empty() {
        println!("No Metis jobs found.");
        return Ok(());
    }

    let header = format!(
        "{:<name_width$} {:<status_width$} {:<runtime_width$} {}",
        "ID",
        "STATUS",
        "RUNTIME",
        "NOTES",
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH,
        runtime_width = RUNTIME_WIDTH
    );
    println!("{}", header);
    println!("{}", "-".repeat(header.len()));

    for job in response.jobs {
        let runtime = job.runtime.unwrap_or_else(|| "-".into());
        let notes = job.notes.unwrap_or_else(|| "-".into());
        let row = format!(
            "{:<name_width$} {:<status_width$} {:<runtime_width$} {}",
            job.id,
            job.status,
            runtime,
            notes,
            name_width = NAME_WIDTH,
            status_width = STATUS_WIDTH,
            runtime_width = RUNTIME_WIDTH
        );
        println!("{}", row);
    }

    Ok(())
}
