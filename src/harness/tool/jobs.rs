//! Background job registry for `bash --background`.
//!
//! Jobs are spawned shell processes whose stdout/stderr are redirected to
//! temp files. The registry keeps the child handle alive (kill_on_drop(false)),
//! polls completion with `try_wait` (non-blocking) and exposes status/output
//! for the `/jobs` command. When a job finishes, a `HarnessEvent::JobFinished`
//! is emitted on the spawning session's event channel (if available).

use crate::harness::event::{EventSender, HarnessEvent};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Status of a background job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    /// Finished with an exit code (None = killed by signal).
    Done(Option<i32>),
    /// Could not be waited on / wait failed.
    Failed(String),
}

/// Snapshot of a background job.
#[derive(Clone, Debug)]
#[allow(dead_code)] // command/started_at are part of the /jobs API surface
pub struct JobInfo {
    pub id: u64,
    pub command: String,
    pub status: JobStatus,
    pub started_at: SystemTime,
}

struct Job {
    info: JobInfo,
    child: Option<tokio::process::Child>,
    #[allow(dead_code)] // read via output()
    stdout_path: PathBuf,
    #[allow(dead_code)] // read via output()
    stderr_path: PathBuf,
    /// Event channel of the spawning session (for the finished notification).
    events: Option<EventSender>,
    #[allow(dead_code)] // kept for future per-session job scoping
    session_id: String,
}

/// Shared registry of background bash jobs (Arc/Mutex, one per runtime).
#[derive(Default)]
pub struct JobRegistry {
    inner: Mutex<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    next_id: u64,
    jobs: Vec<Job>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves (and returns) the next job id. Call before spawning so the
    /// output temp files can embed the id; pass it to [`Self::spawn_with_id`].
    pub fn reserve_id(&self) -> u64 {
        let mut inner = self.lock();
        inner.next_id += 1;
        inner.next_id
    }

    /// Registers a spawned child as a background job and starts a watcher
    /// task that polls completion (via `try_wait`) and emits the
    /// `JobFinished` event. Must be called on an `Arc<JobRegistry>` so the
    /// watcher keeps the registry alive. `id` comes from [`Self::reserve_id`].
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_id(
        self: &Arc<Self>,
        id: u64,
        command: String,
        child: tokio::process::Child,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        events: Option<EventSender>,
        session_id: String,
    ) -> u64 {
        let mut inner = self.lock();
        inner.jobs.push(Job {
            info: JobInfo {
                id,
                command,
                status: JobStatus::Running,
                started_at: SystemTime::now(),
            },
            child: Some(child),
            stdout_path,
            stderr_path,
            events: events.clone(),
            session_id: session_id.clone(),
        });
        drop(inner);

        let registry = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                match registry.poll_job(id) {
                    Some(JobStatus::Running) => continue,
                    Some(JobStatus::Done(code)) => {
                        let tx = registry
                            .lock()
                            .jobs
                            .iter()
                            .find(|j| j.info.id == id)
                            .and_then(|j| j.events.clone());
                        if let Some(tx) = tx {
                            let _ = tx.send(HarnessEvent::JobFinished {
                                session_id: session_id.clone(),
                                job_id: id,
                                exit_code: code,
                            });
                        }
                        registry.cleanup(id);
                        break;
                    }
                    Some(JobStatus::Failed(_)) | None => {
                        registry.cleanup(id);
                        break;
                    }
                }
            }
        });
        id
    }

    /// Removes a finished job from the registry and deletes its temp
    /// output files. No-op for unknown or still-running jobs.
    fn cleanup(&self, id: u64) {
        let (stdout_path, stderr_path) = {
            let mut inner = self.lock();
            let Some(idx) = inner.jobs.iter().position(|j| j.info.id == id) else {
                return;
            };
            if inner.jobs[idx].info.status == JobStatus::Running {
                return;
            }
            let job = inner.jobs.remove(idx);
            (job.stdout_path, job.stderr_path)
        };
        let _ = std::fs::remove_file(&stdout_path);
        let _ = std::fs::remove_file(&stderr_path);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Non-blocking status check of a single job via `try_wait`.
    /// Returns None for unknown jobs.
    pub fn poll_job(&self, id: u64) -> Option<JobStatus> {
        let mut inner = self.lock();
        let job = inner.jobs.iter_mut().find(|j| j.info.id == id)?;
        if job.info.status != JobStatus::Running {
            return Some(job.info.status.clone());
        }
        let child = job.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                job.child = None;
                let code = status.code();
                job.info.status = JobStatus::Done(code);
                Some(job.info.status.clone())
            }
            Ok(None) => Some(JobStatus::Running),
            Err(e) => {
                job.child = None;
                job.info.status = JobStatus::Failed(e.to_string());
                Some(job.info.status.clone())
            }
        }
    }

    /// Polls every job (non-blocking) and returns fresh snapshots.
    #[allow(dead_code)] // public API surface (used by /jobs via commands)
    pub fn poll(&self) -> Vec<JobInfo> {
        let ids: Vec<u64> = self.lock().jobs.iter().map(|j| j.info.id).collect();
        for id in ids {
            let _ = self.poll_job(id);
        }
        self.list()
    }

    /// Snapshot of all jobs (does not poll; call `poll()` first for fresh status).
    #[allow(dead_code)] // public API surface (used by /jobs via commands)
    pub fn list(&self) -> Vec<JobInfo> {
        self.lock().jobs.iter().map(|j| j.info.clone()).collect()
    }

    /// Combined stdout+stderr of a finished (or running) job.
    #[allow(dead_code)] // public API surface (used by /jobs via commands)
    pub fn output(&self, id: u64) -> Result<String, String> {
        let (stdout_path, stderr_path) = {
            let inner = self.lock();
            let job = inner
                .jobs
                .iter()
                .find(|j| j.info.id == id)
                .ok_or_else(|| format!("no such job: {}", id))?;
            (job.stdout_path.clone(), job.stderr_path.clone())
        };
        let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
        let mut combined = stdout;
        if !stderr.is_empty() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str("[stderr]\n");
            combined.push_str(&stderr);
        }
        Ok(super::env::mask_secrets(&combined))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_sleep_job(registry: &Arc<JobRegistry>) -> u64 {
        let dir = std::env::temp_dir();
        let out = dir.join(format!("rustclaw-job-test-{}.out", std::process::id()));
        let err = dir.join(format!("rustclaw-job-test-{}.err", std::process::id()));
        let out_file = std::fs::File::create(&out).unwrap();
        let err_file = std::fs::File::create(&err).unwrap();
        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 0.1 && echo done")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(out_file))
            .stderr(std::process::Stdio::from(err_file))
            .kill_on_drop(false)
            .spawn()
            .unwrap();
        registry.spawn_with_id(
            registry.reserve_id(),
            "sleep 0.1 && echo done".to_string(),
            child,
            out,
            err,
            None,
            "s1".to_string(),
        )
    }

    #[tokio::test]
    async fn test_job_runs_then_done_with_output() {
        let registry = Arc::new(JobRegistry::new());
        let id = spawn_sleep_job(&registry);

        // Initially running (or already done on slow machines — both fine,
        // but never Failed/unknown).
        let first = registry.poll_job(id).unwrap();
        assert_ne!(first, JobStatus::Failed("x".into()));

        // Wait for completion.
        let mut status = first;
        for _ in 0..100 {
            if status != JobStatus::Running {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            status = registry.poll_job(id).unwrap();
        }
        assert_eq!(status, JobStatus::Done(Some(0)));

        // The watcher cleans up the finished job and its temp files.
        let mut cleaned = false;
        for _ in 0..100 {
            if registry.list().is_empty() {
                cleaned = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(cleaned, "job should be removed after completion");
        assert!(
            registry.output(id).is_err(),
            "output should be gone after cleanup"
        );
    }

    #[tokio::test]
    async fn test_list_and_poll() {
        let registry = Arc::new(JobRegistry::new());
        let id = spawn_sleep_job(&registry);
        let jobs = registry.poll();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, id);
        assert_eq!(jobs[0].command, "sleep 0.1 && echo done");
        // Wait until done, then list reflects it (before cleanup removes it).
        let mut saw_done = false;
        for _ in 0..100 {
            if registry.poll_job(id) != Some(JobStatus::Running) {
                if let Some(j) = registry.list().into_iter().find(|j| j.id == id) {
                    if matches!(j.status, JobStatus::Done(Some(0))) {
                        saw_done = true;
                        break;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(saw_done, "expected to observe Done(Some(0)) before cleanup");
    }

    #[tokio::test]
    async fn test_output_unknown_job_errors() {
        let registry = Arc::new(JobRegistry::new());
        let err = registry.output(999).unwrap_err();
        assert!(err.contains("no such job"));
    }

    #[tokio::test]
    async fn test_job_finished_event_emitted() {
        let registry = Arc::new(JobRegistry::new());
        let (tx, mut rx) = crate::harness::event::event_channel();
        let dir = std::env::temp_dir();
        let out = dir.join(format!("rustclaw-job-ev-{}.out", std::process::id()));
        let err = dir.join(format!("rustclaw-job-ev-{}.err", std::process::id()));
        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("echo quick")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(
                std::fs::File::create(&out).unwrap(),
            ))
            .stderr(std::process::Stdio::from(
                std::fs::File::create(&err).unwrap(),
            ))
            .kill_on_drop(false)
            .spawn()
            .unwrap();
        let id = registry.spawn_with_id(
            registry.reserve_id(),
            "echo quick".to_string(),
            child,
            out,
            err,
            Some(tx),
            "s-ev".to_string(),
        );
        let mut got = None;
        for _ in 0..100 {
            if let Ok(ev) = rx.try_recv() {
                got = Some(ev);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        match got {
            Some(HarnessEvent::JobFinished {
                job_id,
                exit_code,
                session_id,
            }) => {
                assert_eq!(job_id, id);
                assert_eq!(exit_code, Some(0));
                assert_eq!(session_id, "s-ev");
            }
            other => panic!("expected JobFinished, got {:?}", other.is_some()),
        }
    }
}
