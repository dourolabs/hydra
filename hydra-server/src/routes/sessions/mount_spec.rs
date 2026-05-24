use hydra_common::{
    BuildCacheContext, RepoName, SessionId,
    api::v1::sessions::{Bundle, MountItem, MountSpec, RelativePath},
};

/// Single source of truth for `CreateSessionRequest → MountSpec` translation.
///
/// The shape returned here mirrors the migration backfill in
/// `20260523020000_add_session_shape_columns.sql` and supersedes the inline
/// builder previously known as `build_mount_spec` (deleted in Phase A).
///
/// Inputs reflect what the server knows after the CreateSessionRequest has
/// been resolved: the lowered `Bundle`, the row's `session_id`, the optional
/// issue-branch id (populated from `$HYDRA_ISSUE_ID` on the resolved env),
/// and the optional service-repo/build-cache pair. The build-cache item is
/// config-derived (not stored on the row) and is appended only when both a
/// service repository and a configured build-cache context are present.
pub fn mount_spec_from_create_request(
    bundle: Bundle,
    session_id: SessionId,
    issue_branch_id: Option<String>,
    build_cache: Option<(RepoName, BuildCacheContext)>,
) -> MountSpec {
    let repo_target = RelativePath::new("repo").expect("static `repo` is valid");
    let docs_target = RelativePath::new("documents").expect("static `documents` is valid");

    let mut mounts = Vec::with_capacity(3);
    mounts.push(MountItem::Bundle {
        target: repo_target.clone(),
        bundle,
        session_id: session_id.clone(),
        issue_branch_id,
    });
    if let Some((service_repo_name, context)) = build_cache {
        mounts.push(MountItem::BuildCache {
            repo_target: repo_target.clone(),
            service_repo_name,
            context,
            session_id,
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
        BuildCacheContext, RepoName, SessionId,
        api::v1::sessions::{Bundle, MountItem},
        build_cache::{BuildCacheSettings, BuildCacheStorageConfig},
    };

    #[test]
    fn produces_bundle_plus_documents_when_no_build_cache() {
        let spec = mount_spec_from_create_request(Bundle::None, SessionId::new(), None, None);
        assert_eq!(spec.working_dir.as_path().to_str(), Some("repo"));
        assert_eq!(spec.mounts.len(), 2);
        assert!(matches!(spec.mounts[0], MountItem::Bundle { .. }));
        assert!(matches!(spec.mounts[1], MountItem::Documents { .. }));
    }

    #[test]
    fn appends_build_cache_between_bundle_and_documents() {
        let session_id = SessionId::new();
        let cache_context = BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: "/tmp/cache".to_string(),
            },
            settings: BuildCacheSettings::default(),
        };
        let repo = RepoName::try_from("acme/widgets".to_string()).unwrap();
        let spec = mount_spec_from_create_request(
            Bundle::None,
            session_id.clone(),
            Some("hydra/i-abcd/head".to_string()),
            Some((repo.clone(), cache_context)),
        );
        assert_eq!(spec.mounts.len(), 3);
        assert!(matches!(spec.mounts[0], MountItem::Bundle { .. }));
        assert!(matches!(spec.mounts[1], MountItem::BuildCache { .. }));
        assert!(matches!(spec.mounts[2], MountItem::Documents { .. }));
        if let MountItem::Bundle {
            session_id: sid,
            issue_branch_id,
            ..
        } = &spec.mounts[0]
        {
            assert_eq!(sid, &session_id);
            assert_eq!(issue_branch_id.as_deref(), Some("hydra/i-abcd/head"));
        }
        if let MountItem::BuildCache {
            service_repo_name,
            session_id: sid,
            ..
        } = &spec.mounts[1]
        {
            assert_eq!(service_repo_name, &repo);
            assert_eq!(sid, &session_id);
        }
    }
}
