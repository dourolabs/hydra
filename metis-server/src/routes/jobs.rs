use crate::{
    AppState,
    config::{AppConfig, build_kube_client},
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use k8s_openapi::{
    api::{
        batch::v1::{Job, JobSpec},
        core::v1::{Container, EnvVar, PodSpec, PodTemplateSpec},
    },
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{Api, Error as KubeError, api::PostParams};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, env};
use uuid::Uuid;

pub async fn create_job(
    State(state): State<AppState>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    let prompt = payload.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(ApiError::bad_request("prompt is required"));
    }

    let config = state.config;
    let namespace = config.metis.namespace.clone();
    let worker_image = config.metis.worker_image.clone();
    let job_uuid = Uuid::new_v4().hyphenated().to_string();
    let job_name = format!("metis-worker-{}", job_uuid);
    let openai_api_key = resolve_openai_key(&config)?;
    let client = build_kube_client(&config.kubernetes)
        .await
        .map_err(ApiError::internal)?;

    let jobs: Api<Job> = Api::namespaced(client, &namespace);
    let metadata_labels = build_metadata_labels(&job_uuid);

    let job = Job {
        metadata: ObjectMeta {
            name: Some(job_name.clone()),
            labels: Some(metadata_labels.clone()),
            ..Default::default()
        },
        spec: Some(JobSpec {
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(metadata_labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "metis-worker".to_string(),
                        image: Some(worker_image),
                        args: Some(vec![
                            "codex".into(),
                            "exec".into(),
                            "--dangerously-bypass-approvals-and-sandbox".into(),
                            prompt.clone(),
                        ]),
                        env: build_env_vars(
                            &job_uuid,
                            &openai_api_key,
                            payload.from_git_rev.as_deref(),
                        ),
                        ..Default::default()
                    }],
                    restart_policy: Some("Never".into()),
                    ..Default::default()
                }),
            },
            backoff_limit: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };

    let pp = PostParams::default();
    match jobs.create(&pp, &job).await {
        Ok(created) => {
            let display_name = created.metadata.name.clone().unwrap_or(job_name.clone());

            Ok(Json(CreateJobResponse {
                job_id: job_uuid,
                job_name: display_name,
                namespace,
            }))
        }
        Err(KubeError::Api(err)) if err.code == 409 => Err(ApiError::conflict(format!(
            "Job '{}' already exists in namespace '{}'",
            job_name, namespace
        ))),
        Err(err) => Err(ApiError::internal(err)),
    }
}

#[derive(Deserialize)]
pub struct CreateJobRequest {
    pub prompt: String,
    #[serde(default)]
    pub from_git_rev: Option<String>,
}

#[derive(Serialize)]
pub struct CreateJobResponse {
    pub job_id: String,
    pub job_name: String,
    pub namespace: String,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn internal(error: impl Into<anyhow::Error>) -> Self {
        let err = error.into();
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

fn build_metadata_labels(job_uuid: &str) -> BTreeMap<String, String> {
    let mut metadata_labels = BTreeMap::new();
    metadata_labels.insert("metis-id".to_string(), job_uuid.to_string());
    metadata_labels
}

fn build_env_vars(
    job_uuid: &str,
    openai_api_key: &str,
    from_git_rev: Option<&str>,
) -> Option<Vec<EnvVar>> {
    let mut vars = vec![
        EnvVar {
            name: "OPENAI_API_KEY".to_string(),
            value: Some(openai_api_key.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "METIS_ID".to_string(),
            value: Some(job_uuid.to_string()),
            ..Default::default()
        },
    ];

    if let Some(rev) = from_git_rev {
        if !rev.trim().is_empty() {
            vars.push(EnvVar {
                name: "FROM_GIT_REV".to_string(),
                value: Some(rev.trim().to_string()),
                ..Default::default()
            });
        }
    }

    Some(vars)
}

fn resolve_openai_key(config: &AppConfig) -> Result<String, ApiError> {
    env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| config.metis.openai_api_key.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ApiError::bad_request(
                "OPENAI_API_KEY is not set. Provide it via the environment or config.toml.",
            )
        })
}
