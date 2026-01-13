#![allow(clippy::too_many_arguments)]

/// Identifier used for jobs, tasks, and other records within Metis.
pub type MetisId = String;

pub mod constants;
pub mod issues;
pub mod job_status;
pub mod jobs;
pub mod logs;
pub mod patches;
pub mod task_status;
