#[cfg(test)]
mod tests {
    use crate::tools::tool::Tool;
    use crate::tools::{
        bash::BashTool,
        browser::BrowserTool,
        cron::CronTool,
        editfile::EditFileTool,
        readfile::ReadFileTool,
        sendmessage::SendMessageTool,
        webfetch::WebFetchTool,
        websearch::WebSearchTool,
        writefile::WriteFileTool,
    };

    #[test]
    fn test_tool_is_concurrency_safe_classifications() {
        struct Case {
            name: &'static str,
            want: bool,
            got: bool,
        }

        let cases = vec![
            Case { name: "read_file", want: true, got: ReadFileTool::default().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "web_fetch", want: true, got: WebFetchTool.is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "web_search", want: true, got: WebSearchTool::new().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "write_file", want: false, got: WriteFileTool::default().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "edit_file", want: false, got: EditFileTool::default().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "bash", want: false, got: BashTool::default().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "browser", want: false, got: BrowserTool::new().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "send_message", want: false, got: SendMessageTool::default().is_concurrency_safe(&serde_json::Value::Null) },
            Case { name: "cron", want: false, got: CronTool::default().is_concurrency_safe(&serde_json::Value::Null) },
        ];

        for c in &cases {
            assert_eq!(c.got, c.want, "tool {:?}: IsConcurrencySafe = {}, want {}", c.name, c.got, c.want);
        }
    }
}