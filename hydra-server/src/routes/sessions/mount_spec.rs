use hydra_common::{
    BuildCacheContext, RepoName,
    api::v1::sessions::{Bundle, MountItem, MountSpec, RelativePath},
};

/// Single source of truth for `CreateSessionRequest → MountSpec` translation.
///
/// The shape returned here mirrors the migration backfill in
/// `20260523020000_add_session_shape_columns.sql` and supersedes the inline
/// builder previously known as `build_mount_spec` (deleted in Phase A).
///
/// Inputs reflect what the server knows after the CreateSessionRequest has
/// been resolved: the lowered `Bundle`, plus the optional service-repo /
/// build-cache pair. The build-cache item is config-derived (not stored on
/// the row) and is appended only when both a service repository and a
/// configured build-cache context are present.
///
/// `MountSpec` is session-id-free: the worker stamps `session_id` and the
/// `$HYDRA_ISSUE_ID`-derived branch id on `InstantiateInputs` at mount
/// instantiation time, so neither rides on `MountItem` variants anymore.
pub fn mount_spec_from_create_request(
    bundle: Bundle,
    build_cache: Option<(RepoName, BuildCacheContext)>,
) -> MountSpec {
    let repo_target = RelativePath::new("repo").expect("static `repo` is valid");
    let docs_target = RelativePath::new("documents").expect("static `documents` is valid");

    let mut mounts = Vec::with_capacity(3);
    mounts.push(MountItem::Bundle {
        target: repo_target.clone(),
        bundle,
    });
    if let Some((service_repo_name, context)) = build_cache {
        mounts.push(MountItem::BuildCache {
            repo_target: repo_target.clone(),
            service_repo_name,
            context,
        });
    }
    mounts.push(MountItem::Documents {
        target: docs_target,
    });

    MountSpec::new(repo_target, mounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::{
        BuildCacheContext, RepoName,
        api::v1::sessions::{Bundle, MountItem},
        build_cache::{BuildCacheSettings, BuildCacheStorageConfig},
    };

    #[test]
    fn produces_bundle_plus_documents_when_no_build_cache() {
        let spec = mount_spec_from_create_request(Bundle::None, None);
        assert_eq!(spec.working_dir.as_path().to_str(), Some("repo"));
        assert_eq!(spec.mounts.len(), 2);
        assert!(matches!(spec.mounts[0], MountItem::Bundle { .. }));
        assert!(matches!(spec.mounts[1], MountItem::Documents { .. }));
    }

    #[test]
    fn appends_build_cache_between_bundle_and_documents() {
        let cache_context = BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: "/tmp/cache".to_string(),
            },
            settings: BuildCacheSettings::default(),
        };
        let repo = RepoName::try_from("acme/widgets".to_string()).unwrap();
        let spec =
            mount_spec_from_create_request(Bundle::None, Some((repo.clone(), cache_context)));
        assert_eq!(spec.mounts.len(), 3);
        assert!(matches!(spec.mounts[0], MountItem::Bundle { .. }));
        assert!(matches!(spec.mounts[1], MountItem::BuildCache { .. }));
        assert!(matches!(spec.mounts[2], MountItem::Documents { .. }));
        if let MountItem::BuildCache {
            service_repo_name, ..
        } = &spec.mounts[1]
        {
            assert_eq!(service_repo_name, &repo);
        }
    }
}
