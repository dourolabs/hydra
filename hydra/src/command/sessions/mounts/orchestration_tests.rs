//! End-to-end orchestration tests for the [`Mount`] trait machinery.
//!
//! [`TestMount`] is a recording mount whose setup/save outcomes are
//! controllable per-instance. The harness below mirrors the two `for`
//! loops in `worker_run::run`: a setup pass over every mount, then a
//! save pass that skips mounts whose `save_phase()` returns `None`.
//! Together they exercise the orchestration policy: ordering,
//! tracked-vs-fatal error routing, fatal short-circuit, and
//! `save_phase() == None` skipping.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::orchestrator::run_phase;
use super::{Mount, MountError, MountResult, Phase};

#[derive(Clone, Copy)]
enum TestOutcome {
    Ok,
    Tracked,
    Fatal,
}

struct TestMount {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
    setup_outcome: TestOutcome,
    save_outcome: Option<TestOutcome>,
}

impl TestMount {
    fn new(
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        setup_outcome: TestOutcome,
        save_outcome: Option<TestOutcome>,
    ) -> Self {
        Self {
            name,
            events,
            setup_outcome,
            save_outcome,
        }
    }
}

fn outcome_to_result(outcome: TestOutcome, name: &str) -> MountResult {
    match outcome {
        TestOutcome::Ok => Ok(()),
        TestOutcome::Tracked => Err(MountError::tracked(anyhow!("{name} tracked"))),
        TestOutcome::Fatal => Err(MountError::fatal(anyhow!("{name} fatal"))),
    }
}

#[async_trait]
impl Mount for TestMount {
    fn setup_phase(&self) -> Phase {
        Phase {
            label: self.name,
            timeout: None,
        }
    }

    fn save_phase(&self) -> Option<Phase> {
        self.save_outcome.map(|_| Phase {
            label: self.name,
            timeout: None,
        })
    }

    async fn setup(&mut self) -> MountResult {
        self.events
            .lock()
            .unwrap()
            .push(format!("setup:{}", self.name));
        outcome_to_result(self.setup_outcome, self.name)
    }

    async fn save(&mut self) -> MountResult {
        self.events
            .lock()
            .unwrap()
            .push(format!("save:{}", self.name));
        let outcome = self
            .save_outcome
            .expect("save invoked but save_outcome is None — orchestrator should skip");
        outcome_to_result(outcome, self.name)
    }
}

/// Mirrors the two `for` loops in `worker_run::run`: setup pass over every
/// mount, then save pass that skips mounts whose `save_phase()` is `None`.
/// Returns `Err` (and skips the remaining setup pass + the entire save
/// pass) if a fatal mount surfaces from `run_phase`, matching the `await?`
/// in the production loops.
async fn drive_mounts(mounts: &mut [Box<dyn Mount>]) -> (Result<()>, Vec<anyhow::Error>) {
    let mut errors = Vec::new();
    for mount in mounts.iter_mut() {
        if let Err(err) = run_phase(mount.setup_phase(), || mount.setup(), &mut errors).await {
            return (Err(err), errors);
        }
    }
    for mount in mounts.iter_mut() {
        let Some(phase) = mount.save_phase() else {
            continue;
        };
        if let Err(err) = run_phase(phase, || mount.save(), &mut errors).await {
            return (Err(err), errors);
        }
    }
    (Ok(()), errors)
}

fn test_mount(
    name: &'static str,
    events: &Arc<Mutex<Vec<String>>>,
    setup_outcome: TestOutcome,
    save_outcome: Option<TestOutcome>,
) -> Box<dyn Mount> {
    Box::new(TestMount::new(
        name,
        Arc::clone(events),
        setup_outcome,
        save_outcome,
    ))
}

#[tokio::test]
async fn ordering_setup_then_save_in_declaration_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut mounts: Vec<Box<dyn Mount>> = vec![
        test_mount("a", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
        test_mount("b", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
        test_mount("c", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
    ];

    let (result, errors) = drive_mounts(&mut mounts).await;

    assert!(result.is_ok());
    assert!(errors.is_empty());
    let observed = events.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec!["setup:a", "setup:b", "setup:c", "save:a", "save:b", "save:c",],
    );
}

#[tokio::test]
async fn tracked_setup_error_keeps_running_and_records_error() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut mounts: Vec<Box<dyn Mount>> = vec![
        test_mount("a", &events, TestOutcome::Tracked, Some(TestOutcome::Ok)),
        test_mount("b", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
    ];

    let (result, errors) = drive_mounts(&mut mounts).await;

    assert!(
        result.is_ok(),
        "tracked setup error must not abort the orchestrator"
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].to_string(), "a tracked");
    let observed = events.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec!["setup:a", "setup:b", "save:a", "save:b"],
        "subsequent setups and all saves must still run after a tracked setup failure"
    );
}

#[tokio::test]
async fn fatal_setup_error_short_circuits_no_save_phase_runs() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut mounts: Vec<Box<dyn Mount>> = vec![
        test_mount("a", &events, TestOutcome::Fatal, Some(TestOutcome::Ok)),
        test_mount("b", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
    ];

    let (result, errors) = drive_mounts(&mut mounts).await;

    let err = result.expect_err("fatal setup must abort the orchestrator");
    assert_eq!(err.to_string(), "a fatal");
    assert!(
        errors.is_empty(),
        "fatal failures bypass the tracked-errors vec"
    );
    let observed = events.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec!["setup:a"],
        "no subsequent setup mounts and no save phase must run after a fatal setup failure"
    );
}

#[tokio::test]
async fn tracked_save_error_keeps_running_and_records_error() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut mounts: Vec<Box<dyn Mount>> = vec![
        test_mount("a", &events, TestOutcome::Ok, Some(TestOutcome::Tracked)),
        test_mount("b", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
    ];

    let (result, errors) = drive_mounts(&mut mounts).await;

    assert!(result.is_ok());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].to_string(), "a tracked");
    let observed = events.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec!["setup:a", "setup:b", "save:a", "save:b"],
        "subsequent saves must still run after a tracked save failure"
    );
}

#[tokio::test]
async fn save_phase_none_skips_save_and_siblings_still_run() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut mounts: Vec<Box<dyn Mount>> = vec![
        test_mount("a", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
        test_mount("b", &events, TestOutcome::Ok, None),
        test_mount("c", &events, TestOutcome::Ok, Some(TestOutcome::Ok)),
    ];

    let (result, errors) = drive_mounts(&mut mounts).await;

    assert!(result.is_ok());
    assert!(errors.is_empty());
    let observed = events.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec!["setup:a", "setup:b", "setup:c", "save:a", "save:c"],
        "the save_phase()==None mount must be skipped while siblings still save"
    );
}

/// Belt-and-braces end-to-end exercise of `spec::instantiate` for a
/// `Bundle::None` mount spec: this is the only scenario the design says
/// is realistic to drive against the filesystem (empty repo, no network,
/// mocked document store). The other matrix rows (with/without cache for
/// `Bundle::GitRepository`) are covered by mount-count assertions in
/// `spec.rs` because exercising them would require hitting the network.
mod mount_spec_e2e {
    use super::*;
    use crate::client::{HydraClient, HydraClientInterface};
    use crate::command::sessions::mounts::spec::{instantiate, InstantiateInputs};
    use crate::test_utils::ids::task_id;
    use httpmock::prelude::*;
    use hydra_common::documents::ListDocumentsResponse;
    use hydra_common::sessions::{Bundle, MountItem, MountSpec, RelativePath};
    use reqwest::Client as HttpClient;

    fn mock_client(server: &MockServer) -> Arc<dyn HydraClientInterface> {
        Arc::new(
            HydraClient::with_http_client(server.base_url(), "tok", HttpClient::new())
                .expect("dummy client"),
        )
    }

    #[tokio::test]
    async fn bundle_none_creates_repo_dir_and_runs_document_round_trip() {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200)
                .json_body_obj(&ListDocumentsResponse::new(Vec::new()));
        });
        let client = mock_client(&server);

        let tempdir = tempfile::tempdir().expect("dest tempdir");
        let dest = tempdir.path().to_path_buf();
        let repo_path = dest.join("repo");
        let documents_path = dest.join("documents");
        assert!(
            !repo_path.exists(),
            "precondition: repo_path must not exist"
        );

        let repo_target = RelativePath::new("repo").expect("static `repo` is valid");
        let spec = MountSpec::new(
            repo_target.clone(),
            vec![
                MountItem::Bundle {
                    target: repo_target,
                    bundle: Bundle::None,
                    session_id: task_id("t-bm-e2e-none"),
                    issue_branch_id: None,
                },
                MountItem::Documents {
                    target: RelativePath::new("documents").expect("static `documents` is valid"),
                },
            ],
        );

        let instantiated = instantiate(
            &spec,
            InstantiateInputs {
                github_token: None,
                worker_home_dir: None,
                dest: &dest,
                client,
            },
        )
        .expect("instantiate");
        assert_eq!(
            instantiated.mounts.len(),
            2,
            "Bundle::None must produce [BundleMount::empty, DocumentsMount]"
        );

        let mut mounts = instantiated.mounts;
        let (result, errors) = drive_mounts(&mut mounts).await;

        assert!(result.is_ok(), "no fatal errors expected: {result:?}");
        assert!(errors.is_empty(), "no tracked errors expected: {errors:?}");
        assert!(
            repo_path.is_dir(),
            "BundleMount::empty must create its target directory at setup"
        );
        assert!(
            documents_path.is_dir(),
            "DocumentsMount must create its target directory at setup"
        );
        // Setup runs sync_documents (one list call) and save runs
        // push_documents (one more list call). No other endpoints should
        // be hit because the document list is empty.
        list_mock.assert_hits(2);
    }
}
