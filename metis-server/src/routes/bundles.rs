use crate::{routes::jobs::ApiError, state::ServiceState};
use metis_common::jobs::{Bundle, BundleSpec};

/// Resolve a BundleSpec into a concrete Bundle using server state.
/// Returns the instantiated bundle and an optional GitHub token to surface to the worker.
pub fn resolve_bundle_spec(
    spec: BundleSpec,
    service_state: &ServiceState,
) -> Result<(Bundle, Option<String>), ApiError> {
    match spec {
        BundleSpec::None => Ok((Bundle::None, None)),
        BundleSpec::TarGz { archive_base64 } => Ok((Bundle::TarGz { archive_base64 }, None)),
        BundleSpec::GitRepository { url, rev } => Ok((Bundle::GitRepository { url, rev }, None)),
        BundleSpec::GitBundle { bundle_base64 } => Ok((Bundle::GitBundle { bundle_base64 }, None)),
        BundleSpec::ServiceRepository { name, rev } => {
            let repo = service_state
                .repositories
                .get(&name)
                .ok_or_else(|| ApiError::bad_request(format!("unknown repository '{name}'")))?;

            let resolved_rev = rev
                .or_else(|| repo.default_branch.clone())
                .unwrap_or_else(|| "main".to_string());

            Ok((
                Bundle::GitRepository {
                    url: repo.remote_url.clone(),
                    rev: resolved_rev,
                },
                repo.github_token.clone(),
            ))
        }
    }
}
