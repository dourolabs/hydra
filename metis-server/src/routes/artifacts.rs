use crate::{
    AppState,
    routes::jobs::ApiError,
    store::{Status, Store, StoreError},
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use chrono::Utc;
use metis_common::MetisId;
use metis_common::artifacts::{
    Artifact, ArtifactKind, ArtifactRecord, IssueDependency, IssueDependencyType, IssueStatus,
    IssueType, ListArtifactsResponse, SearchArtifactsQuery, UpsertArtifactRequest,
    UpsertArtifactResponse,
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct ArtifactIdPath(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for ArtifactIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(artifact_id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        let trimmed = artifact_id.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("artifact_id must not be empty"));
        }

        Ok(Self(trimmed.to_string()))
    }
}

pub async fn create_artifact(
    State(state): State<AppState>,
    Json(payload): Json<UpsertArtifactRequest>,
) -> Result<Json<UpsertArtifactResponse>, ApiError> {
    info!("create_artifact invoked");
    upsert_artifact_internal(state, None, payload).await
}

pub async fn update_artifact(
    State(state): State<AppState>,
    ArtifactIdPath(artifact_id): ArtifactIdPath,
    Json(payload): Json<UpsertArtifactRequest>,
) -> Result<Json<UpsertArtifactResponse>, ApiError> {
    info!(artifact_id = %artifact_id, "update_artifact invoked");
    upsert_artifact_internal(state, Some(artifact_id), payload).await
}

pub async fn get_artifact(
    State(state): State<AppState>,
    ArtifactIdPath(artifact_id): ArtifactIdPath,
) -> Result<Json<ArtifactRecord>, ApiError> {
    info!(artifact_id = %artifact_id, "get_artifact invoked");
    let store_read = state.store.read().await;
    let artifact = store_read
        .get_artifact(&artifact_id)
        .await
        .map_err(|err| map_store_error(err, Some(&artifact_id)))?;

    let readiness = compute_issue_readiness(store_read.as_ref(), &artifact_id, &artifact).await?;
    Ok(Json(ArtifactRecord {
        id: artifact_id,
        artifact,
        is_ready: readiness,
    }))
}

pub async fn list_artifacts(
    State(state): State<AppState>,
    Query(query): Query<SearchArtifactsQuery>,
) -> Result<Json<ListArtifactsResponse>, ApiError> {
    info!(
        artifact_type = ?query.artifact_type,
        issue_type = ?query.issue_type,
        status = ?query.status,
        assignee = ?query.assignee,
        query = ?query.q,
        "list_artifacts invoked"
    );

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let assignee_filter = query
        .assignee
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());

    let store_read = state.store.read().await;
    let artifacts = store_read
        .list_artifacts()
        .await
        .map_err(|err| map_store_error(err, None))?;

    let mut filtered = Vec::new();
    for (id, artifact) in artifacts {
        if !artifact_matches(
            &query.artifact_type,
            query.issue_type,
            query.status,
            search_term.as_deref(),
            assignee_filter,
            &id,
            &artifact,
        ) {
            continue;
        }

        let readiness = compute_issue_readiness(store_read.as_ref(), &id, &artifact).await?;
        filtered.push(ArtifactRecord {
            id,
            artifact,
            is_ready: readiness,
        });
    }

    Ok(Json(ListArtifactsResponse {
        artifacts: filtered,
    }))
}

async fn validate_issue_dependencies(
    store: &mut dyn Store,
    dependencies: &[IssueDependency],
) -> Result<(), ApiError> {
    for dependency in dependencies {
        let target_id = &dependency.issue_id;
        let target = store
            .get_artifact(target_id)
            .await
            .map_err(|err| match err {
                StoreError::ArtifactNotFound(id) => {
                    ApiError::bad_request(format!("issue dependency '{id}' not found"))
                }
                other => map_store_error(other, Some(target_id)),
            })?;

        if !matches!(target, Artifact::Issue { .. }) {
            return Err(ApiError::bad_request(format!(
                "artifact '{target_id}' is not an issue"
            )));
        }
    }

    Ok(())
}

async fn upsert_artifact_internal(
    state: AppState,
    artifact_id: Option<String>,
    payload: UpsertArtifactRequest,
) -> Result<Json<UpsertArtifactResponse>, ApiError> {
    let UpsertArtifactRequest { artifact, job_id } = payload;

    let mut store = state.store.write().await;
    if let Artifact::Issue { dependencies, .. } = &artifact {
        validate_issue_dependencies(store.as_mut(), dependencies).await?;
    }
    if let (
        Some(id),
        Artifact::Issue {
            status: IssueStatus::Closed,
            ..
        },
    ) = (&artifact_id, &artifact)
    {
        ensure_issue_can_close(store.as_mut(), id).await?;
    }
    let artifact_id = match artifact_id {
        Some(id) => {
            if job_id.is_some() {
                return Err(ApiError::bad_request(
                    "job_id may only be provided when creating an artifact",
                ));
            }
            match store.update_artifact(&id, artifact).await {
                Ok(()) => id,
                Err(err) => return Err(map_store_error(err, Some(&id))),
            }
        }
        None => {
            let job_id = job_id
                .as_ref()
                .map(|value| value.trim())
                .map(|value| value.to_string());

            if let Some(ref job_id) = job_id {
                if job_id.is_empty() {
                    return Err(ApiError::bad_request("job_id must not be empty"));
                }

                let status = store.get_status(job_id).await.map_err(|err| match err {
                    StoreError::TaskNotFound(id) => {
                        error!(job_id = %id, "job not found when creating artifact");
                        ApiError::not_found(format!("job '{id}' not found"))
                    }
                    other => {
                        error!(job_id = %job_id, error = %other, "failed to validate job status");
                        ApiError::internal(anyhow!(
                            "failed to validate job status for '{job_id}': {other}"
                        ))
                    }
                })?;

                if status != Status::Running {
                    return Err(ApiError::bad_request(
                        "job_id must reference a running job to record emitted artifacts",
                    ));
                }
            }

            let id = store
                .add_artifact(artifact)
                .await
                .map_err(|err| map_store_error(err, None))?;

            if let Some(job_id) = job_id {
                store
                    .emit_task_artifacts(&job_id, vec![id.clone()], Utc::now())
                    .await
                    .map_err(|err| map_emit_error(err, &job_id))?;
            }

            id
        }
    };

    info!(artifact_id = %artifact_id, "artifact stored successfully");

    Ok(Json(UpsertArtifactResponse { artifact_id }))
}

fn artifact_matches(
    kind_filter: &Option<ArtifactKind>,
    issue_type_filter: Option<IssueType>,
    status_filter: Option<IssueStatus>,
    search_term: Option<&str>,
    assignee_filter: Option<&str>,
    artifact_id: &str,
    artifact: &Artifact,
) -> bool {
    if let Some(kind) = kind_filter {
        let artifact_kind = ArtifactKind::from(artifact);
        if &artifact_kind != kind {
            return false;
        }
    }

    if let Some(issue_type) = issue_type_filter {
        match artifact {
            Artifact::Issue {
                issue_type: current,
                ..
            } if current == &issue_type => {}
            Artifact::Issue { .. } => return false,
            _ => return false,
        }
    }

    if let Some(status) = status_filter {
        match artifact {
            Artifact::Issue {
                status: current, ..
            } if current == &status => {}
            Artifact::Issue { .. } => return false,
            _ => return false,
        }
    }

    if let Some(expected_assignee) = assignee_filter {
        match artifact {
            Artifact::Issue { assignee, .. } => match assignee.as_ref() {
                Some(current) if current.eq_ignore_ascii_case(expected_assignee) => {}
                _ => return false,
            },
            _ => return false,
        }
    }

    if let Some(term) = search_term {
        let lower_id = artifact_id.to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return match artifact {
            Artifact::Patch {
                title,
                diff,
                description,
                ..
            } => {
                title.to_lowercase().contains(term)
                    || diff.to_lowercase().contains(term)
                    || description.to_lowercase().contains(term)
            }
            Artifact::Issue {
                description,
                issue_type,
                status,
                assignee,
                ..
            } => {
                description.to_lowercase().contains(term)
                    || issue_type_matches(term, issue_type)
                    || issue_status_matches(term, status)
                    || assignee
                        .as_deref()
                        .map(|value| value.to_lowercase().contains(term))
                        .unwrap_or(false)
            }
        };
    }

    true
}

fn issue_type_matches(search_term: &str, issue_type: &IssueType) -> bool {
    issue_type.as_str() == search_term
}

fn issue_status_matches(search_term: &str, status: &IssueStatus) -> bool {
    status.as_str() == search_term
}

async fn compute_issue_readiness(
    store: &dyn Store,
    issue_id: &MetisId,
    artifact: &Artifact,
) -> Result<Option<bool>, ApiError> {
    let (status, dependencies) = match artifact {
        Artifact::Issue {
            status,
            dependencies,
            ..
        } => (status, dependencies),
        _ => return Ok(None),
    };

    if matches!(status, IssueStatus::Closed) {
        return Ok(Some(false));
    }

    if has_open_blockers(store, dependencies).await? {
        return Ok(Some(false));
    }

    if matches!(status, IssueStatus::Open) {
        return Ok(Some(true));
    }

    let children = store
        .get_issue_children(issue_id)
        .await
        .map_err(|err| map_store_error(err, Some(issue_id.as_str())))?;
    for child_id in children {
        match store.get_artifact(&child_id).await {
            Ok(Artifact::Issue {
                status: IssueStatus::Closed,
                ..
            }) => {}
            Ok(Artifact::Issue { .. }) => return Ok(Some(false)),
            Ok(_) => {
                return Err(ApiError::internal(anyhow!(
                    "artifact '{child_id}' indexed as an issue child is not an issue"
                )));
            }
            Err(err) => return Err(map_store_error(err, Some(&child_id))),
        }
    }

    Ok(Some(true))
}

async fn has_open_blockers(
    store: &dyn Store,
    dependencies: &[IssueDependency],
) -> Result<bool, ApiError> {
    for dependency in dependencies
        .iter()
        .filter(|dep| dep.dependency_type == IssueDependencyType::BlockedOn)
    {
        match store.get_artifact(&dependency.issue_id).await {
            Ok(Artifact::Issue { status, .. }) if status != IssueStatus::Closed => return Ok(true),
            Ok(Artifact::Issue { .. }) => {}
            Ok(_) => {
                return Err(ApiError::internal(anyhow!(
                    "artifact '{}' is not an issue",
                    dependency.issue_id
                )));
            }
            Err(err) => return Err(map_store_error(err, Some(&dependency.issue_id))),
        }
    }

    Ok(false)
}

async fn ensure_issue_can_close(store: &mut dyn Store, issue_id: &MetisId) -> Result<(), ApiError> {
    let children = store
        .get_issue_children(issue_id)
        .await
        .map_err(|err| map_store_error(err, Some(issue_id.as_str())))?;

    for child_id in children {
        match store.get_artifact(&child_id).await {
            Ok(Artifact::Issue {
                status: IssueStatus::Closed,
                ..
            }) => {}
            Ok(Artifact::Issue { status, .. }) => {
                return Err(ApiError::bad_request(format!(
                    "issue '{issue_id}' cannot be closed while child '{child_id}' has status '{status}'"
                )));
            }
            Ok(_) => {
                return Err(ApiError::internal(anyhow!(
                    "artifact '{child_id}' indexed as an issue child is not an issue"
                )));
            }
            Err(err) => return Err(map_store_error(err, Some(&child_id))),
        }
    }

    Ok(())
}

fn map_store_error(err: StoreError, artifact_id: Option<&str>) -> ApiError {
    match err {
        StoreError::ArtifactNotFound(id) => {
            error!(artifact_id = %id, "artifact not found");
            ApiError::not_found(format!("artifact '{id}' not found"))
        }
        StoreError::InvalidDependency(message) => {
            error!(artifact_id = artifact_id.unwrap_or_default(), %message, "invalid artifact dependency");
            ApiError::bad_request(message)
        }
        other => {
            error!(
                artifact_id = artifact_id.unwrap_or_default(),
                error = %other,
                "artifact store operation failed"
            );
            ApiError::internal(anyhow!("artifact store error: {other}"))
        }
    }
}

fn map_emit_error(err: StoreError, job_id: &str) -> ApiError {
    match err {
        StoreError::TaskNotFound(id) => {
            error!(job_id = %id, "job not found when emitting artifacts");
            ApiError::not_found(format!("job '{id}' not found"))
        }
        StoreError::InvalidStatusTransition => {
            error!(job_id = %job_id, "job not running when emitting artifacts");
            ApiError::bad_request("job must be running to record emitted artifacts")
        }
        other => {
            error!(job_id = %job_id, error = %other, "failed to emit artifacts");
            ApiError::internal(anyhow!("failed to emit artifacts for '{job_id}': {other}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::test_state;
    use axum::extract::Query;
    use metis_common::artifacts::{Artifact, ArtifactKind};

    fn issue(status: IssueStatus, dependencies: Vec<IssueDependency>) -> Artifact {
        Artifact::Issue {
            issue_type: IssueType::Task,
            description: "issue details".to_string(),
            status,
            assignee: None,
            dependencies,
        }
    }

    #[tokio::test]
    async fn issue_readiness_respects_blockers_and_children() {
        let state = test_state();
        let mut store = state.store.write().await;
        let blocker_id = store
            .add_artifact(issue(IssueStatus::InProgress, vec![]))
            .await
            .unwrap();
        let parent_id = store
            .add_artifact(issue(IssueStatus::InProgress, vec![]))
            .await
            .unwrap();

        let child_dependency = IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id: parent_id.clone(),
        };
        let child_id = store
            .add_artifact(issue(IssueStatus::Open, vec![child_dependency.clone()]))
            .await
            .unwrap();
        let open_issue_id = store
            .add_artifact(issue(
                IssueStatus::Open,
                vec![IssueDependency {
                    dependency_type: IssueDependencyType::BlockedOn,
                    issue_id: blocker_id.clone(),
                }],
            ))
            .await
            .unwrap();
        drop(store);

        let store_read = state.store.read().await;
        let open_artifact = store_read.get_artifact(&open_issue_id).await.unwrap();
        assert_eq!(
            compute_issue_readiness(store_read.as_ref(), &open_issue_id, &open_artifact)
                .await
                .unwrap(),
            Some(false)
        );

        let parent_artifact = store_read.get_artifact(&parent_id).await.unwrap();
        assert_eq!(
            compute_issue_readiness(store_read.as_ref(), &parent_id, &parent_artifact)
                .await
                .unwrap(),
            Some(false)
        );
        drop(store_read);

        let mut store = state.store.write().await;
        store
            .update_artifact(
                &child_id,
                issue(IssueStatus::Closed, vec![child_dependency]),
            )
            .await
            .unwrap();
        store
            .update_artifact(&blocker_id, issue(IssueStatus::Closed, vec![]))
            .await
            .unwrap();
        drop(store);

        let store_read = state.store.read().await;
        let open_artifact = store_read.get_artifact(&open_issue_id).await.unwrap();
        assert_eq!(
            compute_issue_readiness(store_read.as_ref(), &open_issue_id, &open_artifact)
                .await
                .unwrap(),
            Some(true)
        );
        let parent_artifact = store_read.get_artifact(&parent_id).await.unwrap();
        assert_eq!(
            compute_issue_readiness(store_read.as_ref(), &parent_id, &parent_artifact)
                .await
                .unwrap(),
            Some(true)
        );
    }

    #[tokio::test]
    async fn update_rejects_closing_when_children_open() {
        let state = test_state();
        let mut store = state.store.write().await;
        let parent_id = store
            .add_artifact(issue(IssueStatus::Open, vec![]))
            .await
            .unwrap();
        let child_dependency = IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id: parent_id.clone(),
        };
        store
            .add_artifact(issue(
                IssueStatus::InProgress,
                vec![child_dependency.clone()],
            ))
            .await
            .unwrap();
        drop(store);

        let result = update_artifact(
            State(state.clone()),
            ArtifactIdPath(parent_id.clone()),
            Json(UpsertArtifactRequest {
                artifact: issue(IssueStatus::Closed, vec![]),
                job_id: None,
            }),
        )
        .await;
        assert!(result.is_err(), "expected closing to be rejected");

        let mut store = state.store.write().await;
        let child_id = store.get_issue_children(&parent_id).await.unwrap()[0].clone();
        store
            .update_artifact(
                &child_id,
                issue(IssueStatus::Closed, vec![child_dependency]),
            )
            .await
            .unwrap();
        drop(store);

        let _ = update_artifact(
            State(state.clone()),
            ArtifactIdPath(parent_id.clone()),
            Json(UpsertArtifactRequest {
                artifact: issue(IssueStatus::Closed, vec![]),
                job_id: None,
            }),
        )
        .await
        .expect("closing should succeed when children are closed");
    }

    #[tokio::test]
    async fn api_responses_include_issue_readiness() {
        let state = test_state();
        let mut store = state.store.write().await;
        let blocker_id = store
            .add_artifact(issue(IssueStatus::Open, vec![]))
            .await
            .unwrap();
        let issue_id = store
            .add_artifact(issue(
                IssueStatus::Open,
                vec![IssueDependency {
                    dependency_type: IssueDependencyType::BlockedOn,
                    issue_id: blocker_id,
                }],
            ))
            .await
            .unwrap();
        drop(store);

        let artifact = get_artifact(State(state.clone()), ArtifactIdPath(issue_id.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(artifact.is_ready, Some(false));

        let list = list_artifacts(
            State(state.clone()),
            Query(SearchArtifactsQuery {
                artifact_type: Some(ArtifactKind::Issue),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        let record = list
            .artifacts
            .into_iter()
            .find(|record| record.id == issue_id)
            .expect("issue missing from list response");
        assert_eq!(record.is_ready, Some(false));
    }
}
