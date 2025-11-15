use crate::{client::MetisClient, config::AppConfig};
use anyhow::Result;

const NAME_WIDTH: usize = 48;
const STATUS_WIDTH: usize = 9;

pub async fn run(config: &AppConfig) -> Result<()> {
    let client = MetisClient::from_config(config)?;
    let response = client.list_jobs().await?;
    let namespace = response.namespace.clone();

    if response.jobs.is_empty() {
        println!("No Metis jobs found in namespace '{}'.", namespace);
        return Ok(());
    }

    println!(
        "{:<name_width$} {:<status_width$} {}",
        "ID",
        "STATUS",
        "RUNTIME",
        name_width = NAME_WIDTH,
        status_width = STATUS_WIDTH
    );
    println!("{}", "-".repeat(NAME_WIDTH + STATUS_WIDTH + 9));

    for job in response.jobs {
        let runtime = job.runtime.unwrap_or_else(|| "-".into());
        println!(
            "{:<name_width$} {:<status_width$} {}",
            job.id,
            job.status,
            runtime,
            name_width = NAME_WIDTH,
            status_width = STATUS_WIDTH
        );
    }

    Ok(())
}
