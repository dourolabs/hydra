pub mod jobs {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    pub struct CreateJobRequest {
        pub prompt: String,
        #[serde(default)]
        pub from_git_rev: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct CreateJobResponse {
        pub job_id: String,
        pub job_name: String,
        pub namespace: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct ListJobsResponse {
        pub namespace: String,
        pub jobs: Vec<JobSummary>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct JobSummary {
        pub id: String,
        pub status: String,
        pub runtime: Option<String>,
    }
}

pub mod logs {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, Serialize, Deserialize)]
    pub struct LogsQuery {
        #[serde(default)]
        pub watch: Option<bool>,
    }
}

pub mod job_outputs {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct JobOutputPayload {
        pub last_message: String,
        pub patch: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct JobOutputResponse {
        pub job_id: String,
        pub output: JobOutputPayload,
    }
}
