use serde_json::Value;

use super::tool::{Tool, ToolResult};

/// Summary of a scheduled job returned by `list_jobs`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobInfo {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub paused: bool,
}

/// Interface for scheduling recurring jobs.
/// Decouples the tool from the cron package.
pub trait JobScheduler: Send + Sync {
    fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()>;
    fn remove_job(&self, name: &str) -> anyhow::Result<()>;
    fn list_jobs(&self) -> Vec<JobInfo>;
    fn pause_job(&self, name: &str) -> anyhow::Result<()>;
    fn resume_job(&self, name: &str) -> anyhow::Result<()>;
    fn update_job_schedule(&self, name: &str, schedule: &str) -> anyhow::Result<()>;
}

/// Allows the agent to dynamically schedule recurring tasks.
pub struct CronTool {
    pub scheduler: Option<Box<dyn JobScheduler>>,
}

impl Default for CronTool {
    fn default() -> Self { Self { scheduler: None } }
}

impl CronTool {
    pub fn new(scheduler: Box<dyn JobScheduler>) -> Self {
        Self { scheduler: Some(scheduler) }
    }
}

#[derive(Debug, Default)]
struct CronInput {
    action: String,
    name: String,
    schedule: String,
    prompt: String,
}

impl CronInput {
    fn from_value(v: &Value) -> Self {
        Self {
            action: v.get("action").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            schedule: v.get("schedule").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            prompt: v.get("prompt").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
        }
    }
}

impl Tool for CronTool {
    fn name(&self) -> &str { "cron" }

    fn description(&self) -> &str {
        r#"Schedule, stop, or list recurring tasks. Supports three actions:
- "add": Schedule a new recurring job. Requires "name" (unique identifier), "schedule" (Go duration string like "30m", "1h", "24h"), and "prompt" (the instruction to execute each interval).
- "remove": Stop and remove a scheduled job. Requires "name".
- "list": List all currently scheduled jobs.
Use this to set up automated checks, reminders, or periodic tasks."#
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "remove", "list"],
                    "description": "The action to perform: add a new job, remove an existing job, or list all jobs"
                },
                "name": {
                    "type": "string",
                    "description": "Unique name for the job (required for add)"
                },
                "schedule": {
                    "type": "string",
                    "description": "How often to run, as a duration string (e.g. \"30m\", \"1h\", \"24h\") (required for add)"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt/instruction to execute each interval (required for add)"
                }
            },
            "required": ["action"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let ci = CronInput::from_value(&input);

        let scheduler = match &self.scheduler {
            None => return Ok(ToolResult::err("cron scheduling is not available")),
            Some(s) => s.as_ref(),
        };

        match ci.action.as_str() {
            "add" => self.add_job(&ci, scheduler),
            "remove" => self.remove_job(&ci, scheduler),
            "list" => self.list_jobs(scheduler),
            other => Ok(ToolResult::err(format!(
                "unknown action: {:?} (valid: add, remove, list)",
                other
            ))),
        }
    }
}

impl CronTool {
    fn add_job(&self, ci: &CronInput, scheduler: &dyn JobScheduler) -> anyhow::Result<ToolResult> {
        if ci.name.is_empty() {
            return Ok(ToolResult::err("name is required for add action"));
        }
        if ci.schedule.is_empty() {
            return Ok(ToolResult::err("schedule is required for add action"));
        }
        if ci.prompt.is_empty() {
            return Ok(ToolResult::err("prompt is required for add action"));
        }
        if let Err(e) = scheduler.add_job(&ci.name, &ci.schedule, &ci.prompt) {
            return Ok(ToolResult::err(format!("failed to schedule job: {}", e)));
        }
        let mut meta = serde_json::Map::new();
        meta.insert("name".to_owned(), Value::String(ci.name.clone()));
        meta.insert("schedule".to_owned(), Value::String(ci.schedule.clone()));
        Ok(ToolResult {
            output: format!("Scheduled job {:?} to run every {}", ci.name, ci.schedule),
            metadata: Some(meta),
            ..Default::default()
        })
    }

    fn remove_job(&self, ci: &CronInput, scheduler: &dyn JobScheduler) -> anyhow::Result<ToolResult> {
        if ci.name.is_empty() {
            return Ok(ToolResult::err("name is required for remove action"));
        }
        if let Err(e) = scheduler.remove_job(&ci.name) {
            return Ok(ToolResult::err(format!("failed to remove job: {}", e)));
        }
        Ok(ToolResult::ok(format!("Removed job {:?}", ci.name)))
    }

    fn list_jobs(&self, scheduler: &dyn JobScheduler) -> anyhow::Result<ToolResult> {
        let jobs = scheduler.list_jobs();
        if jobs.is_empty() {
            return Ok(ToolResult::ok("No scheduled jobs."));
        }
        let out = serde_json::to_string_pretty(&jobs)
            .unwrap_or_else(|e| format!("marshal error: {}", e));
        let mut meta = serde_json::Map::new();
        meta.insert("count".to_owned(), Value::Number(jobs.len().into()));
        Ok(ToolResult {
            output: format!("{} scheduled job(s):\n{}", jobs.len(), out),
            metadata: Some(meta),
            ..Default::default()
        })
    }
}

#[path = "crontool_test.rs"]
#[cfg(test)]
mod crontool_test;