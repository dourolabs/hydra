use super::{LogStream, MetisClientInterface};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream;
use metis_common::{
    artifact_status::{ArtifactStatusUpdate, GetArtifactStatusResponse, SetArtifactStatusResponse},
    artifacts::{
        ArtifactRecord, ListArtifactsResponse, SearchArtifactsQuery, UpsertArtifactRequest,
        UpsertArtifactResponse,
    },
    logs::LogsQuery,
    sessions::{
        CreateSessionRequest, CreateSessionResponse, KillSessionResponse, ListSessionsResponse,
    },
    MetisId,
};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
pub struct MockMetisClient {
    pub create_session_responses: Mutex<VecDeque<CreateSessionResponse>>,
    pub list_sessions_responses: Mutex<VecDeque<ListSessionsResponse>>,
    pub log_responses: Mutex<VecDeque<Vec<String>>>,
    pub log_requests: Mutex<Vec<MetisId>>,
    pub artifact_upsert_responses: Mutex<VecDeque<UpsertArtifactResponse>>,
    pub get_artifact_responses: Mutex<VecDeque<ArtifactRecord>>,
    pub list_artifacts_responses: Mutex<VecDeque<ListArtifactsResponse>>,
    pub artifact_upsert_requests: Mutex<Vec<(Option<MetisId>, UpsertArtifactRequest)>>,
    pub artifact_get_requests: Mutex<Vec<MetisId>>,
    pub list_artifacts_queries: Mutex<Vec<SearchArtifactsQuery>>,
    pub recorded_requests: Mutex<Vec<CreateSessionRequest>>,
}

impl MockMetisClient {
    pub fn push_create_session_response(&self, response: CreateSessionResponse) {
        self.create_session_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_list_sessions_response(&self, response: ListSessionsResponse) {
        self.list_sessions_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_log_lines<I, S>(&self, lines: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let lines = lines.into_iter().map(Into::into).collect();
        self.log_responses.lock().unwrap().push_back(lines);
    }

    pub fn recorded_requests(&self) -> Vec<CreateSessionRequest> {
        self.recorded_requests.lock().unwrap().clone()
    }

    pub fn recorded_log_requests(&self) -> Vec<MetisId> {
        self.log_requests.lock().unwrap().clone()
    }

    pub fn push_upsert_artifact_response(&self, response: UpsertArtifactResponse) {
        self.artifact_upsert_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_get_artifact_response(&self, response: ArtifactRecord) {
        self.get_artifact_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_list_artifacts_response(&self, response: ListArtifactsResponse) {
        self.list_artifacts_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn recorded_artifact_upserts(&self) -> Vec<(Option<MetisId>, UpsertArtifactRequest)> {
        self.artifact_upsert_requests.lock().unwrap().clone()
    }

    pub fn recorded_get_artifact_requests(&self) -> Vec<MetisId> {
        self.artifact_get_requests.lock().unwrap().clone()
    }

    pub fn recorded_list_artifacts_queries(&self) -> Vec<SearchArtifactsQuery> {
        self.list_artifacts_queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl MetisClientInterface for MockMetisClient {
    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse> {
        self.recorded_requests.lock().unwrap().push(request.clone());
        self.create_session_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_session"))
    }

    async fn list_sessions(&self) -> Result<ListSessionsResponse> {
        self.list_sessions_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_sessions"))
    }

    async fn kill_session(&self, _session_id: &MetisId) -> Result<KillSessionResponse> {
        Err(anyhow!("kill_session not implemented in MockMetisClient"))
    }

    async fn get_session_logs(
        &self,
        session_id: &MetisId,
        _query: &LogsQuery,
    ) -> Result<LogStream> {
        self.log_requests
            .lock()
            .unwrap()
            .push(session_id.to_string());
        let lines = self
            .log_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_session_logs"))?;
        let stream = stream::iter(lines.into_iter().map(Ok));
        Ok(Box::pin(stream))
    }

    async fn set_artifact_status(
        &self,
        _artifact_id: &MetisId,
        _status: &ArtifactStatusUpdate,
    ) -> Result<SetArtifactStatusResponse> {
        Err(anyhow!(
            "set_artifact_status not implemented in MockMetisClient"
        ))
    }

    async fn get_artifact_status(
        &self,
        _artifact_id: &MetisId,
    ) -> Result<GetArtifactStatusResponse> {
        Err(anyhow!(
            "get_artifact_status not implemented in MockMetisClient"
        ))
    }

    async fn create_artifact(
        &self,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        self.artifact_upsert_requests
            .lock()
            .unwrap()
            .push((None, request.clone()));
        self.artifact_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_artifact"))
    }

    async fn update_artifact(
        &self,
        artifact_id: &MetisId,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        self.artifact_upsert_requests
            .lock()
            .unwrap()
            .push((Some(artifact_id.clone()), request.clone()));
        self.artifact_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for update_artifact"))
    }

    async fn get_artifact(&self, artifact_id: &MetisId) -> Result<ArtifactRecord> {
        self.artifact_get_requests
            .lock()
            .unwrap()
            .push(artifact_id.clone());
        self.get_artifact_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_artifact"))
    }

    async fn list_artifacts(&self, query: &SearchArtifactsQuery) -> Result<ListArtifactsResponse> {
        self.list_artifacts_queries
            .lock()
            .unwrap()
            .push(query.clone());
        self.list_artifacts_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_artifacts"))
    }
}
