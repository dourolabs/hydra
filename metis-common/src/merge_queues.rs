use crate::PatchId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueue {
    #[serde(default)]
    pub patches: Vec<PatchId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnqueueMergePatchRequest {
    pub patch_id: PatchId,
}
