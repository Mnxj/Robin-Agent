use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub type AgentFunc =
    Arc<dyn Fn(CancellationToken, String) -> futures::future::BoxFuture<'static, anyhow::Result<String>> + Send + Sync>;

pub type OutputFunc = Arc<dyn Fn(String, String) + Send + Sync>;

#[derive(Clone)]
pub struct Job {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub paused: bool,
    pub agent_fn: AgentFunc,
    pub output_fn: Option<OutputFunc>,
    pub(crate) interval: Duration,
}

struct RunningJob {
    job: Job,
    cancel: CancellationToken,
    paused: bool,
}

pub struct Scheduler {
    mu: Mutex<SchedulerInner>,
}

struct SchedulerInner {
    jobs: Vec<Job>,
    running: HashMap<String, RunningJob>,
    root_cancel: Option<CancellationToken>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            mu: Mutex::new(SchedulerInner {
                jobs: Vec::new(),
                running: HashMap::new(),
                root_cancel: None,
            }),
        }
    }

    pub fn add(&self, mut job: Job) -> anyhow::Result<()> {
        let d = humantime::parse_duration(&job.schedule).or_else(|_| {
            job.schedule
                .parse::<u64>()
                .map(Duration::from_millis)
                .map_err(|e| anyhow::anyhow!("{}", e))
        })?;
        job.interval = d;

        let mut inner = self.mu.lock();
        inner.jobs.push(job);
        Ok(())
    }

    pub fn start(self: &Arc<Self>, root_cancel: CancellationToken) {
        let mut inner = self.mu.lock();
        inner.root_cancel = Some(root_cancel.clone());

        let jobs: Vec<Job> = inner.jobs.clone();
        for job in jobs {
            if inner.running.contains_key(&job.name) {
                continue;
            }
            let job_cancel = root_cancel.child_token();
            let rj = RunningJob {
                job: job.clone(),
                cancel: job_cancel.clone(),
                paused: false,
            };
            inner.running.insert(job.name.clone(), rj);
            let this = Arc::clone(self);
            tokio::spawn(Self::run_job(job, job_cancel, this));
        }

        info!("cron scheduler started, jobs={}", inner.running.len());
    }

    pub fn stop(&self) {
        let mut inner = self.mu.lock();
        for rj in inner.running.values() {
            rj.cancel.cancel();
        }
        inner.running.clear();
        info!("cron scheduler stopped");
    }

    pub fn remove(&self, name: &str) -> anyhow::Result<()> {
        let mut inner = self.mu.lock();
        let running_removed = if let Some(rj) = inner.running.remove(name) {
            rj.cancel.cancel();
            true
        } else {
            false
        };
        let job_removed = if let Some(pos) = inner.jobs.iter().position(|j| j.name == name) {
            inner.jobs.remove(pos);
            true
        } else {
            false
        };

        if !running_removed && !job_removed {
            return Err(anyhow::anyhow!("job {:?} not found", name));
        }
        info!("cron job removed, name={}", name);
        Ok(())
    }

    pub fn pause(self: &Arc<Self>, name: &str) -> anyhow::Result<()> {
        let mut inner = self.mu.lock();
        if let Some(rj) = inner.running.get_mut(name) {
            if rj.paused {
                return Err(anyhow::anyhow!("job {:?} is already paused", name));
            }
            rj.cancel.cancel();
            rj.paused = true;
            for j in inner.jobs.iter_mut() {
                if j.name == name {
                    j.paused = true;
                    break;
                }
            }
            info!("cron job paused, name={}", name);
            Ok(())
        } else {
            let found = inner.jobs.iter().any(|j| j.name == name);
            if found {
                Err(anyhow::anyhow!("job {:?} is not running", name))
            } else {
                Err(anyhow::anyhow!("job {:?} not found", name))
            }
        }
    }

    pub fn resume(self: &Arc<Self>, name: &str) -> anyhow::Result<()> {
        let mut inner = self.mu.lock();
        let root = inner
            .root_cancel
            .clone()
            .ok_or_else(|| anyhow::anyhow!("scheduler not started"))?;

        if let Some(rj) = inner.running.get_mut(name) {
            if !rj.paused {
                return Err(anyhow::anyhow!("job {:?} is not paused", name));
            }
            let job_cancel = root.child_token();
            rj.cancel = job_cancel.clone();
            rj.paused = false;
            let job = rj.job.clone();
            for j in inner.jobs.iter_mut() {
                if j.name == name {
                    j.paused = false;
                    break;
                }
            }
            let this = Arc::clone(self);
            tokio::spawn(Self::run_job(job, job_cancel, this));
            info!("cron job resumed, name={}", name);
            Ok(())
        } else {
            Err(anyhow::anyhow!("job {:?} not found", name))
        }
    }

    pub fn update_schedule(self: &Arc<Self>, name: &str, new_schedule: &str) -> anyhow::Result<()> {
        let d = humantime::parse_duration(new_schedule)
            .map_err(|e| anyhow::anyhow!("invalid schedule {:?}: {}", new_schedule, e))?;

        let mut inner = self.mu.lock();
        let was_paused = inner
            .running
            .get(name)
            .map(|rj| rj.paused)
            .ok_or_else(|| anyhow::anyhow!("job {:?} not found", name))?;

        if !was_paused {
            if let Some(rj) = inner.running.get(name) {
                rj.cancel.cancel();
            }
        }

        if let Some(rj) = inner.running.get_mut(name) {
            rj.job.schedule = new_schedule.to_string();
            rj.job.interval = d;
        }
        for j in inner.jobs.iter_mut() {
            if j.name == name {
                j.schedule = new_schedule.to_string();
                j.interval = d;
                break;
            }
        }

        if !was_paused {
            if let Some(root) = inner.root_cancel.clone() {
                let job_cancel = root.child_token();
                if let Some(rj) = inner.running.get_mut(name) {
                    rj.cancel = job_cancel.clone();
                    let job = rj.job.clone();
                    let this = Arc::clone(self);
                    tokio::spawn(Self::run_job(job, job_cancel, this));
                }
            }
        }

        info!("cron job schedule updated, name={}, schedule={}", name, new_schedule);
        Ok(())
    }

    async fn run_job(job: Job, cancel: CancellationToken, _scheduler: Arc<Scheduler>) {
        let mut ticker = tokio::time::interval(job.interval);
        info!("cron job registered, name={}, interval={:?}", job.name, job.interval);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    info!("cron job running, name={}", job.name);
                    let response = (job.agent_fn)(cancel.clone(), job.prompt.clone()).await;
                    match response {
                        Err(e) => {
                            if cancel.is_cancelled() {
                                return;
                            }
                            error!("cron job failed, name={}, error={}", job.name, e);
                        }
                        Ok(resp) => {
                            info!("cron job completed, name={}, response_length={}", job.name, resp.len());
                            if let Some(out_fn) = &job.output_fn {
                                out_fn(job.name.clone(), resp);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn jobs(&self) -> Vec<Job> {
        let inner = self.mu.lock();
        let mut result = inner.jobs.clone();
        for j in result.iter_mut() {
            if let Some(rj) = inner.running.get(&j.name) {
                j.paused = rj.paused;
            }
        }
        result
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}