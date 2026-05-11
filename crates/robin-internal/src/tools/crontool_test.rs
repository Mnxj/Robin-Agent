#[cfg(test)]
mod tests {
    use super::super::{CronTool, JobInfo, JobScheduler};
    use crate::tools::tool::Tool;

    struct MockJobScheduler {
        last_job_name: std::sync::Mutex<String>,
        last_job_schedule: std::sync::Mutex<String>,
        last_job_prompt: std::sync::Mutex<String>,
        removed_name: std::sync::Mutex<String>,
        jobs: Vec<JobInfo>,
        add_err: Option<String>,
        remove_err: Option<String>,
    }

    impl MockJobScheduler {
        fn new(jobs: Vec<JobInfo>) -> Self {
            Self {
                last_job_name: Default::default(),
                last_job_schedule: Default::default(),
                last_job_prompt: Default::default(),
                removed_name: Default::default(),
                jobs,
                add_err: None,
                remove_err: None,
            }
        }

        fn with_add_err(mut self, err: &str) -> Self {
            self.add_err = Some(err.to_owned());
            self
        }
    }

    impl JobScheduler for MockJobScheduler {
        fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()> {
            *self.last_job_name.lock().unwrap() = name.to_owned();
            *self.last_job_schedule.lock().unwrap() = schedule.to_owned();
            *self.last_job_prompt.lock().unwrap() = prompt.to_owned();
            if let Some(e) = &self.add_err {
                anyhow::bail!("{}", e);
            }
            Ok(())
        }
        fn remove_job(&self, name: &str) -> anyhow::Result<()> {
            *self.removed_name.lock().unwrap() = name.to_owned();
            if let Some(e) = &self.remove_err {
                anyhow::bail!("{}", e);
            }
            Ok(())
        }
        fn list_jobs(&self) -> Vec<JobInfo> { self.jobs.clone() }
        fn pause_job(&self, _: &str) -> anyhow::Result<()> { Ok(()) }
        fn resume_job(&self, _: &str) -> anyhow::Result<()> { Ok(()) }
        fn update_job_schedule(&self, _: &str, _: &str) -> anyhow::Result<()> { Ok(()) }
    }

    #[test]
    fn test_cron_tool_name() {
        let tool = CronTool::default();
        assert_eq!(tool.name(), "cron");
    }

    #[test]
    fn test_cron_tool_parameters_valid_json() {
        let tool = CronTool::default();
        let params = tool.parameters();
        assert!(params.is_object());
    }

    #[test]
    fn test_cron_tool_add_job() {
        let scheduler = MockJobScheduler::new(vec![]);
        let tool = CronTool::new(Box::new(scheduler));
        let input = serde_json::json!({
            "action": "add",
            "name": "daily-check",
            "schedule": "24h",
            "prompt": "Run daily diagnostics"
        });
        let res = tool.execute(input).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("daily-check"), "output: {}", res.output);
        assert!(res.output.contains("24h"), "output: {}", res.output);
    }

    #[test]
    fn test_cron_tool_list_jobs() {
        let jobs = vec![
            JobInfo { name: "job1".to_owned(), schedule: "1h".to_owned(), prompt: "check status".to_owned(), paused: false },
            JobInfo { name: "job2".to_owned(), schedule: "30m".to_owned(), prompt: "monitor logs".to_owned(), paused: false },
        ];
        let tool = CronTool::new(Box::new(MockJobScheduler::new(jobs)));
        let res = tool.execute(serde_json::json!({"action": "list"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("2 scheduled job(s)"), "output: {}", res.output);
        assert!(res.output.contains("job1"), "output: {}", res.output);
        assert!(res.output.contains("job2"), "output: {}", res.output);
    }

    #[test]
    fn test_cron_tool_list_jobs_empty() {
        let tool = CronTool::new(Box::new(MockJobScheduler::new(vec![])));
        let res = tool.execute(serde_json::json!({"action": "list"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("No scheduled jobs"), "output: {}", res.output);
    }

    #[test]
    fn test_cron_tool_unknown_action() {
        let tool = CronTool::new(Box::new(MockJobScheduler::new(vec![])));
        let res = tool.execute(serde_json::json!({"action": "delete"})).unwrap();
        assert!(res.error.contains("unknown action"), "error: {}", res.error);
        assert!(res.error.contains("delete"), "error: {}", res.error);
    }

    #[test]
    fn test_cron_tool_add_missing_name() {
        let tool = CronTool::new(Box::new(MockJobScheduler::new(vec![])));
        let res = tool.execute(serde_json::json!({
            "action": "add",
            "schedule": "1h",
            "prompt": "do stuff"
        })).unwrap();
        assert!(res.error.contains("name is required"), "error: {}", res.error);
    }

    #[test]
    fn test_cron_tool_add_missing_schedule() {
        let tool = CronTool::new(Box::new(MockJobScheduler::new(vec![])));
        let res = tool.execute(serde_json::json!({
            "action": "add",
            "name": "job1",
            "prompt": "do stuff"
        })).unwrap();
        assert!(res.error.contains("schedule is required"), "error: {}", res.error);
    }

    #[test]
    fn test_cron_tool_add_missing_prompt() {
        let tool = CronTool::new(Box::new(MockJobScheduler::new(vec![])));
        let res = tool.execute(serde_json::json!({
            "action": "add",
            "name": "job1",
            "schedule": "1h"
        })).unwrap();
        assert!(res.error.contains("prompt is required"), "error: {}", res.error);
    }

    #[test]
    fn test_cron_tool_nil_scheduler() {
        let tool = CronTool::default();
        let res = tool.execute(serde_json::json!({"action": "list"})).unwrap();
        assert!(res.error.contains("not available"), "error: {}", res.error);
    }

    #[test]
    fn test_cron_tool_add_job_error() {
        let scheduler = MockJobScheduler::new(vec![]).with_add_err("duplicate name");
        let tool = CronTool::new(Box::new(scheduler));
        let res = tool.execute(serde_json::json!({
            "action": "add",
            "name": "job1",
            "schedule": "1h",
            "prompt": "do stuff"
        })).unwrap();
        assert!(res.error.contains("failed to schedule job"), "error: {}", res.error);
        assert!(res.error.contains("duplicate name"), "error: {}", res.error);
    }
}