use std::collections::HashMap;
use parking_lot::RwLock;
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};

use super::provider::{Diagnostic, ToolDef};

struct NormalizedEntry {
    tools: Vec<ToolDef>,
    diags: Vec<Diagnostic>,
}

const STRIP_CACHE_MAX: usize = 256;

static STRIP_CACHE: Lazy<RwLock<HashMap<String, NormalizedEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub fn apply_strip_list(tools: Vec<ToolDef>, fields: &[&str]) -> (Vec<ToolDef>, Vec<Diagnostic>) {
    if tools.is_empty() {
        return (tools, vec![]);
    }
    let key = strip_cache_key(&tools, fields);

    {
        let cache = STRIP_CACHE.read();
        if let Some(hit) = cache.get(&key) {
            return (clone_tool_defs(&hit.tools), clone_diags(&hit.diags));
        }
    }

    let mut out = Vec::with_capacity(tools.len());
    let mut all_diags = Vec::new();
    for t in &tools {
        let (new_params, diags) = strip_fields(&t.name, &t.parameters, fields);
        let mut td = t.clone();
        td.parameters = new_params;
        out.push(td);
        all_diags.extend(diags);
    }

    {
        let mut cache = STRIP_CACHE.write();
        if cache.len() >= STRIP_CACHE_MAX {
            if let Some(k) = cache.keys().next().cloned() {
                cache.remove(&k);
            }
        }
        cache.insert(key, NormalizedEntry {
            tools: clone_tool_defs(&out),
            diags: clone_diags(&all_diags),
        });
    }

    (out, all_diags)
}

fn strip_cache_key(tools: &[ToolDef], fields: &[&str]) -> String {
    let mut sorted_fields: Vec<&str> = fields.to_vec();
    sorted_fields.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(sorted_fields.join("\x00").as_bytes());
    hasher.update(b"\x01");
    for t in tools {
        hasher.update(t.name.as_bytes());
        hasher.update(b"\x02");
        hasher.update(t.parameters.to_string().as_bytes());
        hasher.update(b"\x03");
    }
    hex::encode(hasher.finalize())
}

fn clone_tool_defs(in_: &[ToolDef]) -> Vec<ToolDef> {
    in_.to_vec()
}

fn clone_diags(in_: &[Diagnostic]) -> Vec<Diagnostic> {
    in_.to_vec()
}

pub fn reset_strip_cache() {
    let mut cache = STRIP_CACHE.write();
    cache.clear();
}

pub fn strip_fields(
    tool_name: &str,
    schema: &serde_json::Value,
    fields: &[&str],
) -> (serde_json::Value, Vec<Diagnostic>) {
    if fields.is_empty() {
        return (schema.clone(), vec![]);
    }
    let strip_set: std::collections::HashSet<&str> = fields.iter().copied().collect();
    let mut diags = Vec::new();
    let result = walk_strip(schema.clone(), "", tool_name, &strip_set, &mut diags);
    (result, diags)
}

fn walk_strip(
    node: serde_json::Value,
    path: &str,
    tool_name: &str,
    strip_set: &std::collections::HashSet<&str>,
    diags: &mut Vec<Diagnostic>,
) -> serde_json::Value {
    let mut m = match node {
        serde_json::Value::Object(m) => m,
        other => return other,
    };

    let mut keys: Vec<String> = m.keys().cloned().collect();
    keys.sort();

    for k in keys {
        let field_path = join_path(path, &k);
        if strip_set.contains(k.as_str()) {
            diags.push(Diagnostic {
                tool_name: tool_name.to_string(),
                field: field_path.clone(),
                action: "stripped".to_string(),
                reason: "field not supported by provider".to_string(),
            });
            m.remove(&k);
            continue;
        }
        match k.as_str() {
            "properties" => {
                if let Some(serde_json::Value::Object(props)) = m.get_mut(&k) {
                    let prop_keys: Vec<String> = props.keys().cloned().collect();
                    let mut sorted_pk = prop_keys;
                    sorted_pk.sort();
                    for pk in sorted_pk {
                        if let Some(v) = props.remove(&pk) {
                            let nested_path = join_path(&field_path, &pk);
                            let new_v = walk_strip(v, &nested_path, tool_name, strip_set, diags);
                            props.insert(pk, new_v);
                        }
                    }
                }
            }
            "items" | "additionalProperties" => {
                if let Some(v) = m.remove(&k) {
                    let new_v = walk_strip(v, &field_path, tool_name, strip_set, diags);
                    m.insert(k, new_v);
                }
            }
            _ => {}
        }
    }

    serde_json::Value::Object(m)
}

fn join_path(base: &str, leaf: &str) -> String {
    if base.is_empty() {
        leaf.to_string()
    } else {
        format!("{}.{}", base, leaf)
    }
}