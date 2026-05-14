//! Process reaper for the worker.
//!
//! After the agent execution phase completes, [`reap_other_processes`] enumerates
//! every other live process in the worker's PID namespace and terminates them
//! (SIGTERM, short grace period, then SIGKILL on any survivors). This is the
//! systemic safety net for agent-started background processes (e.g. `pnpm dev`,
//! `vite`, `mock-server`, or anything the agent backgrounded via `some-script &`)
//! that would otherwise keep the worker pod alive past its useful end.
//!
//! # Safety boundary
//!
//! **This function assumes the worker runs inside an isolated PID namespace**
//! (a K8s pod or a Docker container). In that context PID 1 is the worker
//! itself (or, in single-player setups, a direct child of PID 1), and reaping
//! "every other process in the namespace" only touches processes the agent
//! started. **Invoking this on a developer's laptop would SIGTERM unrelated
//! user processes.** `worker_run::run` is only entered from session execution
//! contexts so this is safe in practice.
//!
//! The companion in-band shutdown paths
//! (`worker::commands::kill_process_group` / `worker::interactive`) remain in
//! place — they handle the common stdout-pipe path. This reaper exists to catch
//! the cases where a child detached its stdout (`> /dev/null 2>&1 &`) or called
//! `setsid` and so escaped the process-group SIGTERM.

#[cfg(unix)]
use std::time::Duration;

/// Grace period between SIGTERM and SIGKILL, aligned with
/// `worker::commands::PROCESS_GROUP_GRACE_PERIOD`.
#[cfg(unix)]
const REAP_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Summary of a single reaper invocation, returned for logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReapSummary {
    /// Number of victims that received SIGTERM.
    pub sigterm_sent: usize,
    /// Number of victims that were still alive after the grace period and
    /// received SIGKILL.
    pub sigkill_sent: usize,
}

/// Reap every non-self process alive in the current PID namespace.
///
/// Reads `/proc` to enumerate PIDs, sends SIGTERM to each candidate, waits a
/// short grace period, then SIGKILLs any survivor. PIDs that vanish mid-scan
/// (the normal race) and PIDs whose `/proc` entries can't be read are skipped
/// without failing the whole call. See module docs for the safety boundary.
#[cfg(unix)]
pub(crate) async fn reap_other_processes() -> ReapSummary {
    let self_pid = std::process::id();
    let victims = collect_victim_pids(self_pid);
    reap_pids(&victims).await
}

/// Non-unix no-op so call sites compile without `cfg` gates.
#[cfg(not(unix))]
pub(crate) async fn reap_other_processes() -> ReapSummary {
    ReapSummary::default()
}

#[cfg(unix)]
#[derive(Debug)]
struct VictimInfo {
    pid: u32,
    ppid: u32,
    cmdline: String,
}

/// SIGTERM the given victims, wait the grace period, then SIGKILL survivors.
///
/// Split out from [`reap_other_processes`] so tests can drive the signal
/// machinery against a single, known-safe PID without nuking the test
/// runner's environment.
#[cfg(unix)]
async fn reap_pids(victims: &[VictimInfo]) -> ReapSummary {
    let mut summary = ReapSummary::default();
    for victim in victims {
        log(format!(
            "Reaper: SIGTERM pid={} ppid={} cmd={:?}",
            victim.pid, victim.ppid, victim.cmdline
        ));
        // SAFETY: `kill(pid, sig)` is async-signal-safe and well-defined for
        // any pid_t. Errors (e.g. ESRCH for a PID that vanished) are ignored.
        unsafe {
            libc::kill(victim.pid as libc::pid_t, libc::SIGTERM);
        }
        summary.sigterm_sent += 1;
    }

    if summary.sigterm_sent == 0 {
        return summary;
    }

    tokio::time::sleep(REAP_GRACE_PERIOD).await;

    for victim in victims {
        if !pid_exists(victim.pid) {
            continue;
        }
        log(format!(
            "Reaper: SIGKILL pid={} ppid={} cmd={:?} (survived SIGTERM grace period)",
            victim.pid, victim.ppid, victim.cmdline
        ));
        // SAFETY: same justification as the SIGTERM call above.
        unsafe {
            libc::kill(victim.pid as libc::pid_t, libc::SIGKILL);
        }
        summary.sigkill_sent += 1;
    }

    summary
}

#[cfg(unix)]
fn collect_victim_pids(self_pid: u32) -> Vec<VictimInfo> {
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(err) => {
            log(format!(
                "Reaper: failed to read /proc, skipping reap pass: {err}"
            ));
            return Vec::new();
        }
    };

    let mut victims = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };

        if pid == self_pid {
            continue;
        }
        // Defensive: don't kill PID 1 unless the worker is itself PID 1. In a
        // pod/container, PID 1 is the namespace's init — killing it tears down
        // the namespace entirely and races with our own shutdown path.
        if pid == 1 {
            continue;
        }

        let cmdline = match read_cmdline(pid) {
            Ok(line) => line,
            Err(_) => continue,
        };
        // Kernel threads have empty cmdline — skip them. (They aren't visible
        // inside a PID namespace anyway, but defensive belt-and-suspenders.)
        if cmdline.is_empty() {
            continue;
        }

        let ppid = read_ppid(pid).unwrap_or(0);
        victims.push(VictimInfo {
            pid,
            ppid,
            cmdline,
        });
    }
    victims
}

#[cfg(unix)]
fn read_cmdline(pid: u32) -> std::io::Result<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline"))?;
    // /proc/<pid>/cmdline is NUL-separated argv. Replace NULs with spaces so
    // the log line is human-readable, and strip a trailing NUL if present.
    let trimmed = if bytes.last() == Some(&0) {
        &bytes[..bytes.len() - 1]
    } else {
        &bytes[..]
    };
    Ok(String::from_utf8_lossy(trimmed).replace('\0', " "))
}

#[cfg(unix)]
fn read_ppid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("PPid:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

#[cfg(unix)]
fn pid_exists(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(unix)]
fn log(message: impl std::fmt::Display) {
    println!("{message}");
}

#[cfg(all(test, unix, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::Instant;
    use tokio::process::Command;

    /// Spawn a `sleep 60` and drive [`reap_pids`] directly against just that
    /// PID. We deliberately bypass [`reap_other_processes`] here because that
    /// function nukes *every* other process in the namespace, which would
    /// break the cargo test runner (and the developer's shell) when run
    /// outside a container. This test exercises the SIGTERM → grace →
    /// SIGKILL machinery against a single, controlled victim.
    #[tokio::test]
    async fn reap_pids_terminates_orphan_sleep() {
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .expect("failed to spawn sleep subprocess");

        let child_pid = child.id().expect("sleep child should have a PID");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            pid_exists(child_pid),
            "child pid {child_pid} should be in /proc after spawn"
        );

        let victim = VictimInfo {
            pid: child_pid,
            ppid: std::process::id(),
            cmdline: "sleep 60".to_string(),
        };

        let start = Instant::now();
        let summary = reap_pids(&[victim]).await;
        let elapsed = start.elapsed();

        assert_eq!(summary.sigterm_sent, 1, "expected 1 SIGTERM sent");

        // Collect the now-dead child so it isn't left as a zombie.
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;

        assert!(
            !pid_exists(child_pid),
            "child pid {child_pid} should be gone after reap (elapsed {elapsed:?})"
        );
        assert!(
            elapsed < REAP_GRACE_PERIOD + Duration::from_secs(2),
            "reap should complete within grace period + slack, took {elapsed:?}"
        );
    }

    /// Verify the /proc enumeration sees a spawned child and correctly
    /// excludes the test process and PID 1. We do *not* invoke the killer
    /// half here for the same reason as above.
    #[tokio::test]
    async fn collect_victim_pids_includes_spawned_child_excludes_self_and_init() {
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("failed to spawn sleep subprocess");

        let child_pid = child.id().expect("sleep child should have a PID");
        tokio::time::sleep(Duration::from_millis(100)).await;

        let self_pid = std::process::id();
        let victims = collect_victim_pids(self_pid);

        assert!(
            victims.iter().any(|v| v.pid == child_pid),
            "spawned child pid {child_pid} should be in victim set"
        );
        assert!(
            !victims.iter().any(|v| v.pid == self_pid),
            "self pid {self_pid} must never be in victim set"
        );
        assert!(
            !victims.iter().any(|v| v.pid == 1),
            "PID 1 must never be in victim set"
        );

        // kill_on_drop(true) cleans up the child as the handle drops.
        let _ = child.kill().await;
    }

    #[test]
    fn read_cmdline_handles_missing_pid() {
        assert!(read_cmdline(0).is_err());
    }

    #[test]
    fn pid_exists_returns_false_for_missing_pid() {
        assert!(!pid_exists(u32::MAX));
    }

    #[test]
    fn read_cmdline_handles_self() {
        let self_pid = std::process::id();
        let cmdline = read_cmdline(self_pid).expect("reading own cmdline should succeed");
        assert!(
            !cmdline.is_empty(),
            "own cmdline should not be empty for a user-space process"
        );
    }
}
