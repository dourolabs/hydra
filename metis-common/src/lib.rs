#![allow(clippy::too_many_arguments)]

/// Identifier used for jobs, tasks, and artifacts within Metis.
pub type MetisId = String;

pub mod artifacts;
pub mod constants;
pub mod job_status;
pub mod jobs;
pub mod logs;
pub mod task_status;
