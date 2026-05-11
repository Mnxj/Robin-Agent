use axum::response::{Html, IntoResponse};
use chrono::Utc;

/// Agent info for rendering in the UI.
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub model: String,
    pub workspace: String,
    pub sandbox: String,
}

/// Returns an axum handler that serves the control panel UI.
pub async fn ui_handler(
    version: String,
    agents: Vec<AgentInfo>,
) -> impl IntoResponse {
    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let agents_html = render_agents(&agents);
    let html = UI_HTML
        .replace("{VERSION}", &version)
        .replace("{TIMESTAMP}", &timestamp)
        .replace("{AGENTS}", &agents_html);
    Html(html)
}

fn render_agents(agents: &[AgentInfo]) -> String {
    let mut html = String::new();
    for a in agents {
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            escape_html(&a.id),
            escape_html(&a.name),
            escape_html(&a.model),
            escape_html(&a.workspace),
            escape_html(&a.sandbox),
        ));
    }
    html
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const UI_HTML: &str = include_str!("ui_template.html");