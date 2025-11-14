use crate::config::{build_kube_client, AppConfig};
use k8s_openapi::api::batch::v1::Job;
use kube::{
    api::{DeleteParams, ListParams},
    Api,
};

pub async fn run(config: &AppConfig) -> anyhow::Result<()> {
    let namespace = &config.metis.namespace;
    let client = build_kube_client(&config.kubernetes).await?;
    let jobs_api: Api<Job> = Api::namespaced(client, namespace);

    let lp = ListParams::default().labels("metis-id");
    let jobs = jobs_api.list(&lp).await?.into_iter().collect::<Vec<_>>();

    if jobs.is_empty() {
        println!(
            "No Metis jobs with a 'metis-id' label found in namespace '{}'.",
            namespace
        );
        return Ok(());
    }

    let mut deleted_ids = Vec::new();
    let mut skipped_ids = Vec::new();

    for job in jobs {
        let job_name = match job.metadata.name.clone() {
            Some(name) => name,
            None => {
                skipped_ids.push("<unknown>".to_string());
                continue;
            }
        };
        let job_id = job
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("metis-id").cloned())
            .unwrap_or_else(|| job_name.clone());

        match terminal_state(&job) {
            Some(state) => {
                match jobs_api
                    .delete(&job_name, &DeleteParams::foreground())
                    .await
                {
                    Ok(_) => {
                        println!("Deleted Metis job '{}' ({}).", job_id, state);
                        deleted_ids.push(job_id);
                    }
                    Err(err) => {
                        eprintln!("Failed to delete job '{}' ({}): {}", job_id, job_name, err);
                        skipped_ids.push(job_id);
                    }
                }
            }
            None => skipped_ids.push(job_id),
        }
    }

    if deleted_ids.is_empty() {
        println!("No completed or failed Metis jobs to clean up.");
    } else {
        println!(
            "Deleted {} Metis job(s): {}",
            deleted_ids.len(),
            deleted_ids.join(", ")
        );
    }

    if !skipped_ids.is_empty() {
        println!(
            "Skipped {} job(s): {}",
            skipped_ids.len(),
            skipped_ids.join(", ")
        );
    }

    Ok(())
}

fn terminal_state(job: &Job) -> Option<&'static str> {
    let status = job.status.as_ref()?;

    if status.succeeded.unwrap_or(0) > 0 {
        return Some("complete");
    }

    if status.failed.unwrap_or(0) > 0 {
        return Some("failed");
    }

    None
}
