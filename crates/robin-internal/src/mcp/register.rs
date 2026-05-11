use crate::tools::Registry;
use super::adapter::{McpToolAdapter, ParallelSafeFn};
use super::manager::Manager;

/// RegisterTools registers every tool exposed by mgr's servers into reg, with
/// the per-server tool_prefix applied. Collisions with names already in reg
/// (e.g. core tools) cause a hard error — operators must set tool_prefix to
/// disambiguate. Server enumeration order matches Manager::servers().
///
/// parallel_safe is a live-read callback the adapter consults on every
/// is_concurrency_safe call so that toggling mcp_servers[].parallel_safe via
/// the settings UI takes effect on the next agent run without restart.
/// Pass None to preserve the legacy "always false" behavior (used by tests).
///
/// Uses a blocking call for tools/list — this function should be called from
/// an async context (spawning blocking work onto the tokio thread pool).
pub async fn register_tools(
    reg: &Registry,
    mgr: &Manager,
    parallel_safe: Option<ParallelSafeFn>,
) -> anyhow::Result<Vec<String>> {
    let mut registered = Vec::new();
    for s in mgr.servers() {
        let client = s.live().ok_or_else(|| {
            anyhow::anyhow!("mcp[{}]: no connected client", s.id)
        })?;

        let tool_list = client.list_tools().await
            .map_err(|e| anyhow::anyhow!("mcp[{}]: list tools: {}", s.id, e))?;

        for t in tool_list {
            let full_name = format!("{}{}", s.tool_prefix, t.name);
            if reg.get(&full_name).is_some() {
                return Err(anyhow::anyhow!(
                    "mcp[{}]: tool name collision on {:?} — set tool_prefix in mcp_servers config",
                    s.id, full_name
                ));
            }
            let adapter = McpToolAdapter::new(
                full_name.clone(),
                t.name,
                t.description,
                t.input_schema,
                s.clone(),
                parallel_safe.clone(),
            );
            reg.register(adapter);
            registered.push(full_name);
        }
    }
    Ok(registered)
}