//! Reap orphan processes left over after the agent execution phase.
//!
//! After the agent (claude / codex) exits, the worker may still have descendants
//! alive in its PID namespace — backgrounded scripts, dev servers (`vite`,
//! `mock-server`), or anything an agent invoked with `&` that survived the
//! existing `kill_process_group` path in `worker/commands.rs::run_claude`.
//! Those orphans hold open pipes the worker is still draining, which prevents
//! the pod from exiting cleanly and can leave it billable for hours after the
//! agent finished useful work.
//!
//! `reap_other_processes` walks `/proc`, SIGTERMs every PID that is not the
//! worker itself or PID 1, sleeps a short grace period, and SIGKILLs any
//! survivors. Per-PID errors (a PID disappearing mid-scan, a /proc entry that
//! can't be read) are logged and ignored.
//!
//! # Safety boundary
//!
//! This function assumes it is invoked inside an isolated PID namespace —
//! a Kubernetes pod or Docker container. In that environment the only live
//! processes are descendants of the worker, so reaping them is correct.
//!
//! Calling this on a developer's laptop would SIGKILL every other process the
//! user owns. The only call site, `worker_run::run`, is entered from session
//! execution flows that the worker only runs inside containers, so this is
//! safe in practice. Do not add new call sites without auditing the namespace
//! assumption.

use std::time::Duration;

/// Grace period between SIGTERM and SIGKILL. Matches the spirit of
/// `PROCESS_GROUP_GRACE_PERIOD` in `worker/commands.rs` so well-behaved
/// children get the same shutdown window before being force-killed.
#[cfg(unix)]
const REAP_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Outcome of a reap pass — emitted to logs and returned for diagnostics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReapStats {
    /// Number of non-self, non-init processes that were sent SIGTERM.
    pub sigtermed: usize,
    /// Subset of the above that were still alive after the grace period and
    /// then sent SIGKILL.
    pub sigkilled: usize,
}

/// Reap every other process visible in this process's PID namespace.
///
/// Returns counts of how many processes were SIGTERM'd and how many had to be
/// escalated to SIGKILL after the grace period. See module docs for the
/// safety assumptions.
#[cfg(unix)]
pub async fn reap_other_processes() -> ReapStats {
    let self_pid = std::process::id();
    let candidates = enumerate_proc_pids();

    let mut victims: Vec<VictimInfo> = Vec::new();
    for pid in candidates {
        if pid == self_pid {
            continue;
        }
        // Defensive: never SIGTERM the pod's init process unless we're it.
        if pid == 1 && self_pid != 1 {
            continue;
        }
        let cmdline = read_cmdline(pid);
        // Kernel threads have an empty cmdline. They aren't user processes
        // and `kill` against them is meaningless / errors anyway.
        if cmdline.is_empty() {
            continue;
        }
        let ppid = read_ppid(pid).unwrap_or(0);
        eprintln!("reaper: SIGTERM pid={pid} ppid={ppid} cmdline={cmdline:?}");
        // SAFETY: kill(pid, SIGTERM) is a documented libc call; passing a
        // positive pid signals that single process.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        victims.push(VictimInfo {
            pid,
            ppid,
            cmdline,
        });
    }

    if victims.is_empty() {
        return ReapStats::default();
    }

    tokio::time::sleep(REAP_GRACE_PERIOD).await;

    let mut sigkilled = 0usize;
    for victim in &victims {
        if !pid_is_alive(victim.pid) {
            continue;
        }
        eprintln!(
            "reaper: SIGKILL (still alive after grace) pid={} ppid={} cmdline={:?}",
            victim.pid, victim.ppid, victim.cmdline
        );
        // SAFETY: same as above.
        unsafe {
            libc::kill(victim.pid as i32, libc::SIGKILL);
        }
        sigkilled += 1;
    }

    ReapStats {
        sigtermed: victims.len(),
        sigkilled,
    }
}

/// On non-unix platforms there is no `/proc` and no agent-spawned children to
/// reap (the worker only runs in Linux containers in production). Return an
/// empty result without doing any work.
#[cfg(not(unix))]
pub async fn reap_other_processes() -> ReapStats {
    ReapStats::default()
}

#[cfg(unix)]
struct VictimInfo {
    pid: u32,
    ppid: u32,
    cmdline: String,
}

#[cfg(unix)]
fn enumerate_proc_pids() -> Vec<u32> {
    let read_dir = match std::fs::read_dir("/proc") {
        Ok(rd) => rd,
        Err(err) => {
            eprintln!("reaper: failed to read /proc: {err}");
            return Vec::new();
        }
    };
    let mut pids = Vec::new();
    for entry in read_dir.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(pid) = name.parse::<u32>() {
                pids.push(pid);
            }
        }
    }
    pids
}

#[cfg(unix)]
fn read_cmdline(pid: u32) -> String {
    let bytes = match std::fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    // /proc/<pid>/cmdline uses NUL separators between args and trails with NUL.
    let trimmed: Vec<u8> = bytes
        .split(|b| *b == 0)
        .filter(|seg| !seg.is_empty())
        .flat_map(|seg| {
            let mut v = seg.to_vec();
            v.push(b' ');
            v
        })
        .collect();
    let mut s = String::from_utf8_lossy(&trimmed).into_owned();
    if s.ends_with(' ') {
        s.pop();
    }
    s
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
fn pid_is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(all(test, unix, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    use tokio::process::Command;

    /// Spawns a `sleep 60` subprocess, waits briefly for it to register in
    /// /proc, then asserts that `reap_other_processes` removes it within the
    /// grace period plus a small buffer.
    #[tokio::test]
    async fn reaps_spawned_sleep_subprocess() {
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .expect("failed to spawn sleep");
        let pid = child.id().expect("sleep child should have a pid");

        // Give the kernel a moment to register the new PID under /proc.
        for _ in 0..20 {
            if pid_is_alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            pid_is_alive(pid),
            "spawned sleep pid {pid} never appeared in /proc"
        );

        let started = Instant::now();
        let stats = reap_other_processes().await;
        let elapsed = started.elapsed();

        // The sleep child is in our PID namespace, has a non-empty cmdline,
        // and is not us — so it must have been at least SIGTERM'd.
        assert!(
            stats.sigtermed >= 1,
            "expected at least one SIGTERM victim, got {stats:?}"
        );
        // SIGTERM alone kills `sleep`, so it should not have needed SIGKILL.
        // We don't assert sigkilled == 0 strictly because slow CI could in
        // principle force the escalation; but the grace period should be
        // ample for `sleep` to die.
        assert!(
            elapsed <= REAP_GRACE_PERIOD + Duration::from_secs(2),
            "reap took {elapsed:?}, expected <= grace ({REAP_GRACE_PERIOD:?}) + buffer"
        );

        // After reaping, the sleep PID should be gone (or a defunct zombie
        // until we reap the wait status below).
        // Reap the zombie to keep the test environment clean.
        let _ = child.wait().await;
        assert!(
            !pid_is_alive(pid),
            "sleep pid {pid} should be gone from /proc after reaper + wait"
        );
    }

    #[test]
    fn enumerate_proc_pids_includes_self() {
        let self_pid = std::process::id();
        let pids = enumerate_proc_pids();
        assert!(
            pids.contains(&self_pid),
            "self pid {self_pid} should appear in /proc enumeration"
        );
    }

    #[test]
    fn read_cmdline_returns_non_empty_for_self() {
        let self_pid = std::process::id();
        let cmdline = read_cmdline(self_pid);
        assert!(
            !cmdline.is_empty(),
            "self cmdline should not be empty for the test process"
        );
    }
}
