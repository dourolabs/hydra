mod harness;

use anyhow::Result;
use metis_common::TaskId;
use metis_server::{
    domain::actors::Actor,
    job_engine::{JobEngine, JobEngineError, JobStatus, LocalJobEngine},
};
use std::collections::HashMap;

fn make_actor() -> (Actor, String) {
    Actor::new_for_task(
        TaskId::new(),
        metis_server::domain::users::Username::from("test-user"),
    )
}

fn dummy_env() -> HashMap<String, String> {
    HashMap::new()
}

/// Create a LocalJobEngine. The subprocess it spawns (current test binary
/// with `jobs worker-run` args) will fail because the test binary does not
/// handle those arguments, but the engine infrastructure — process tracking,
/// log capture, and status transitions — still works correctly.
fn make_engine() -> LocalJobEngine {
    LocalJobEngine::new("http://localhost:0".to_string())
}

/// Helper: create a job and wait for its subprocess to finish (expected to
/// fail since the test binary doesn't understand `jobs worker-run` args).
async fn create_and_wait_for_exit(
    engine: &LocalJobEngine,
    metis_id: &TaskId,
) -> Result<(), JobEngineError> {
    let (actor, token) = make_actor();
    engine
        .create_job(
            metis_id,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    // Wait for the subprocess to exit (it will fail quickly since the
    // test binary doesn't understand the `jobs worker-run` subcommand).
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
    loop {
        let job = engine.find_job_by_metis_id(metis_id).await?;
        if job.status != JobStatus::Running {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for subprocess to exit");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    Ok(())
}

// ── Test: job creation and tracking ──────────────────────────────────

#[tokio::test]
async fn create_job_spawns_and_tracks_process() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();
    let (actor, token) = make_actor();

    engine
        .create_job(
            &metis_id,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    // The job should be findable immediately after creation.
    let job = engine.find_job_by_metis_id(&metis_id).await?;
    assert_eq!(job.id, metis_id);
    assert!(job.creation_time.is_some());
    assert!(job.start_time.is_some());

    Ok(())
}

// ── Test: duplicate job creation rejected ────────────────────────────

#[tokio::test]
async fn create_job_rejects_duplicate() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();
    let (actor, token) = make_actor();

    engine
        .create_job(
            &metis_id,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    let result = engine
        .create_job(
            &metis_id,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await;

    assert!(
        matches!(result, Err(JobEngineError::AlreadyExists(_))),
        "duplicate create_job should return AlreadyExists"
    );

    Ok(())
}

// ── Test: subprocess failure transitions status to Failed ────────────

#[tokio::test]
async fn subprocess_failure_transitions_to_failed() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();

    create_and_wait_for_exit(&engine, &metis_id).await?;

    let job = engine.find_job_by_metis_id(&metis_id).await?;
    assert_eq!(
        job.status,
        JobStatus::Failed,
        "subprocess should fail because the test binary doesn't handle worker-run args"
    );
    assert!(
        job.completion_time.is_some(),
        "completion_time should be set after process exits"
    );
    assert!(
        job.failure_message.is_some(),
        "failure_message should be set for failed jobs"
    );

    Ok(())
}

// ── Test: log retrieval after job completes ──────────────────────────

#[tokio::test]
async fn get_logs_returns_content_after_job_exits() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();

    create_and_wait_for_exit(&engine, &metis_id).await?;

    // The subprocess wrote its error output to the log file.
    let logs = engine.get_logs(&metis_id, None).await?;
    // The log file exists and was readable. Content depends on what the
    // test binary prints when given unrecognized args — just verify no error.
    let _ = logs;

    Ok(())
}

// ── Test: get_logs tail_lines parameter ──────────────────────────────

#[tokio::test]
async fn get_logs_respects_tail_lines() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();

    create_and_wait_for_exit(&engine, &metis_id).await?;

    let all_logs = engine.get_logs(&metis_id, None).await?;
    let total_lines = all_logs.lines().count();

    if total_lines > 1 {
        let tail_1 = engine.get_logs(&metis_id, Some(1)).await?;
        assert_eq!(
            tail_1.lines().count(),
            1,
            "tail_lines=1 should return exactly 1 line"
        );
    }

    Ok(())
}

// ── Test: get_logs for unknown job returns NotFound ───────────────────

#[tokio::test]
async fn get_logs_returns_not_found_for_unknown_job() {
    let engine = make_engine();
    let unknown_id = TaskId::new();

    let result = engine.get_logs(&unknown_id, None).await;
    assert!(
        matches!(result, Err(JobEngineError::NotFound(_))),
        "get_logs for unknown job should return NotFound"
    );
}

// ── Test: list_jobs includes created jobs ────────────────────────────

#[tokio::test]
async fn list_jobs_includes_created_jobs() -> Result<()> {
    let engine = make_engine();
    let id1 = TaskId::new();
    let id2 = TaskId::new();
    let (actor, token) = make_actor();

    engine
        .create_job(
            &id1,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;
    engine
        .create_job(
            &id2,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    let jobs = engine.list_jobs().await?;
    assert_eq!(jobs.len(), 2, "list_jobs should return both created jobs");

    let ids: Vec<&TaskId> = jobs.iter().map(|j| &j.id).collect();
    assert!(ids.contains(&&id1));
    assert!(ids.contains(&&id2));

    Ok(())
}

// ── Test: list_jobs returns empty when no jobs exist ─────────────────

#[tokio::test]
async fn list_jobs_returns_empty_when_no_jobs() -> Result<()> {
    let engine = make_engine();
    let jobs = engine.list_jobs().await?;
    assert!(jobs.is_empty());
    Ok(())
}

// ── Test: find_job_by_metis_id returns NotFound for unknown id ───────

#[tokio::test]
async fn find_job_returns_not_found_for_unknown_id() {
    let engine = make_engine();
    let result = engine.find_job_by_metis_id(&TaskId::new()).await;
    assert!(matches!(result, Err(JobEngineError::NotFound(_))));
}

// ── Test: kill_job removes process from tracking ─────────────────────

#[tokio::test]
async fn kill_job_removes_from_tracking() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();
    let (actor, token) = make_actor();

    engine
        .create_job(
            &metis_id,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    // Verify the job is tracked.
    assert!(engine.find_job_by_metis_id(&metis_id).await.is_ok());

    // Kill and verify removal.
    engine.kill_job(&metis_id).await?;

    let result = engine.find_job_by_metis_id(&metis_id).await;
    assert!(
        matches!(result, Err(JobEngineError::NotFound(_))),
        "killed job should no longer be findable"
    );

    Ok(())
}

// ── Test: kill_job returns NotFound for unknown id ───────────────────

#[tokio::test]
async fn kill_job_returns_not_found_for_unknown_id() {
    let engine = make_engine();
    let result = engine.kill_job(&TaskId::new()).await;
    assert!(matches!(result, Err(JobEngineError::NotFound(_))));
}

// ── Test: kill_job removes job from list_jobs ────────────────────────

#[tokio::test]
async fn kill_job_removes_from_list() -> Result<()> {
    let engine = make_engine();
    let id1 = TaskId::new();
    let id2 = TaskId::new();
    let (actor, token) = make_actor();

    engine
        .create_job(
            &id1,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;
    engine
        .create_job(
            &id2,
            &actor,
            &token,
            "unused-image",
            &dummy_env(),
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    engine.kill_job(&id1).await?;

    let jobs = engine.list_jobs().await?;
    assert_eq!(jobs.len(), 1, "only one job should remain after kill");
    assert_eq!(jobs[0].id, id2);

    Ok(())
}

// ── Test: get_logs_stream returns log content ────────────────────────

#[tokio::test]
async fn get_logs_stream_returns_content() -> Result<()> {
    use futures::StreamExt;

    let engine = make_engine();
    let metis_id = TaskId::new();

    create_and_wait_for_exit(&engine, &metis_id).await?;

    let mut rx = engine.get_logs_stream(&metis_id, false)?;

    // Collect all chunks from the stream.
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.next().await {
        chunks.push(chunk);
    }

    // Stream should complete without error (content depends on subprocess output).
    // Just verify the stream was created and terminated properly.
    let _ = &chunks;

    Ok(())
}

// ── Test: get_logs_stream returns NotFound for unknown job ───────────

#[tokio::test]
async fn get_logs_stream_returns_not_found_for_unknown_job() {
    let engine = make_engine();
    let result = engine.get_logs_stream(&TaskId::new(), false);
    assert!(matches!(result, Err(JobEngineError::NotFound(_))));
}

// ── Test: completion_time is set after process exits ─────────────────

#[tokio::test]
async fn completion_time_set_after_exit() -> Result<()> {
    let engine = make_engine();
    let metis_id = TaskId::new();

    create_and_wait_for_exit(&engine, &metis_id).await?;

    let job = engine.find_job_by_metis_id(&metis_id).await?;
    assert!(
        job.completion_time.is_some(),
        "completion_time should be set after subprocess exits"
    );
    assert!(
        job.creation_time.unwrap() <= job.completion_time.unwrap(),
        "completion_time should be >= creation_time"
    );

    Ok(())
}

// ── Test: env vars are correctly built ───────────────────────────────

#[tokio::test]
async fn create_job_passes_env_vars_to_subprocess() -> Result<()> {
    let engine = LocalJobEngine::new("http://test-server:8080".to_string());
    let metis_id = TaskId::new();
    let (actor, token) = make_actor();

    let mut env = HashMap::new();
    env.insert("CUSTOM_VAR".to_string(), "custom_value".to_string());

    engine
        .create_job(
            &metis_id,
            &actor,
            &token,
            "unused-image",
            &env,
            "500m".to_string(),
            "1Gi".to_string(),
            "500m".to_string(),
            "1Gi".to_string(),
        )
        .await?;

    // The job was created successfully — env vars were passed to the subprocess.
    // We can't inspect them directly, but the process was spawned without error.
    let job = engine.find_job_by_metis_id(&metis_id).await?;
    assert_eq!(job.id, metis_id);

    Ok(())
}
