use serde_json::Value;

use crate::llm::ImageContent;
use super::tool::{expand_home, resolve_existing_path, validate_path_in_work_dir, Tool, ToolResult};

/// Maps file extensions to MIME types for image files.
fn image_mime_from_ext(ext: &str) -> Option<&'static str> {
    match ext {
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".png" => Some("image/png"),
        ".gif" => Some("image/gif"),
        ".webp" => Some("image/webp"),
        ".bmp" => Some("image/bmp"),
        _ => None,
    }
}

/// Inspects magic bytes to determine the actual image format.
fn detect_image_mime_from_bytes(data: &[u8], hint: &str) -> &'static str {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return "image/jpeg";
    }
    if data.len() >= 4 && data[0] == 0x89 && data[1] == b'P' && data[2] == b'N' && data[3] == b'G' {
        return "image/png";
    }
    if data.len() >= 4 && data[0] == b'G' && data[1] == b'I' && data[2] == b'F' && data[3] == b'8' {
        return "image/gif";
    }
    if data.len() >= 4 && data[0] == b'R' && data[1] == b'I' && data[2] == b'F' && data[3] == b'F' {
        return "image/webp";
    }
    hint_to_static(hint)
}

fn hint_to_static(hint: &str) -> &'static str {
    match hint {
        "image/jpeg" => "image/jpeg",
        "image/png" => "image/png",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// Reads the contents of a file.
pub struct ReadFileTool {
    /// If set, restricts reads to this directory.
    pub work_dir: String,
}

impl Default for ReadFileTool {
    fn default() -> Self { Self { work_dir: String::new() } }
}

impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path. Returns the file contents as text. For image files (jpg, png, gif, webp, bmp), returns the image for visual inspection."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The absolute or relative path to the file to read"
                }
            },
            "required": ["path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_owned(),
            _ => return Ok(ToolResult::err("path is required")),
        };

        let path = expand_home(&path);
        let path = resolve_existing_path(&path);

        if !self.work_dir.is_empty() {
            if let Err(e) = validate_path_in_work_dir(&path, &self.work_dir) {
                return Ok(ToolResult::err(e.to_string()));
            }
        }

        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => return Ok(ToolResult::err(format!("failed to read file: {}", e))),
        };

        // Check if this is an image file
        let ext = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_lowercase()))
            .unwrap_or_default();

        if let Some(hint_mime) = image_mime_from_ext(&ext) {
            let mime_type = detect_image_mime_from_bytes(&data, hint_mime);
            let filename = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path)
                .to_owned();
            return Ok(ToolResult {
                output: format!("Image file: {} ({} bytes)", filename, data.len()),
                images: vec![ImageContent { mime_type: mime_type.to_owned(), data }],
                ..Default::default()
            });
        }

        Ok(ToolResult::ok(String::from_utf8_lossy(&data).into_owned()))
    }
}