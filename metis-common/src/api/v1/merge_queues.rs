use crate::PatchId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct MergeQueue {
    #[serde(default)]
    pub patches: Vec<PatchId>,
}

impl MergeQueue {
    pub fn new(patches: Vec<PatchId>) -> Self {
        Self { patches }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct EnqueueMergePatchRequest {
    pub patch_id: PatchId,
}

impl EnqueueMergePatchRequest {
    pub fn new(patch_id: PatchId) -> Self {
        Self { patch_id }
    }
}
