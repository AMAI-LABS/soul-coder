//! Append tool — append content to a file, auto-creating it if it doesn't exist.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

use super::resolve_path;

pub struct AppendTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl AppendTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for AppendTool {
    fn name(&self) -> &str {
        "append"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "append".into(),
            description: "Append content to the end of a file. \
                          Creates the file and parent directories if they don't exist. \
                          Use for incrementally building up notes, logs, or reports \
                          without overwriting existing content.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to append to (relative to working directory or absolute)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to append to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(
        &self,
        _call_id: &str,
        arguments: serde_json::Value,
        _partial_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> SoulResult<ToolOutput> {
        // If the provider passed args as a JSON string, unwrap it.
        let arguments = if let Some(s) = arguments.as_str() {
            serde_json::from_str(s).unwrap_or(arguments)
        } else {
            arguments
        };

        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if path.is_empty() {
            return Ok(ToolOutput::error("Missing required parameter: path"));
        }

        let resolved = resolve_path(&self.cwd, path);

        // Auto-create parent directories
        if let Some(parent) = resolved.rsplit_once('/') {
            if !parent.0.is_empty() {
                let _ = self.fs.create_dir_all(parent.0).await;
            }
        }

        // Read existing content, then append
        let existing = match self.fs.read_to_string(&resolved).await {
            Ok(s) => s,
            Err(_) => String::new(), // file doesn't exist yet
        };

        let new_content = format!("{}{}", existing, content);
        let appended_bytes = content.len();

        match self.fs.write(&resolved, &new_content).await {
            Ok(()) => Ok(ToolOutput::success(format!(
                "Appended {} bytes to {} ({} bytes total)",
                appended_bytes,
                path,
                new_content.len()
            ))
            .with_metadata(json!({
                "bytes_appended": appended_bytes,
                "total_bytes": new_content.len(),
                "path": path,
            }))),
            Err(e) => Ok(ToolOutput::error(format!("Failed to append to {}: {}", path, e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vfs::MemoryFs;

    async fn setup() -> (Arc<MemoryFs>, AppendTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = AppendTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    #[tokio::test]
    async fn append_to_new_file() {
        let (fs, tool) = setup().await;
        let result = tool
            .execute("c1", json!({"path": "notes.md", "content": "first line\n"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("11 bytes"));
        let content = fs.read_to_string("/project/notes.md").await.unwrap();
        assert_eq!(content, "first line\n");
    }

    #[tokio::test]
    async fn append_to_existing_file() {
        let (fs, tool) = setup().await;
        fs.write("/project/notes.md", "first line\n").await.unwrap();

        let result = tool
            .execute("c2", json!({"path": "notes.md", "content": "second line\n"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = fs.read_to_string("/project/notes.md").await.unwrap();
        assert_eq!(content, "first line\nsecond line\n");
    }

    #[tokio::test]
    async fn append_creates_parent_dirs() {
        let (fs, tool) = setup().await;
        let result = tool
            .execute("c3", json!({"path": "deep/nested/notes.md", "content": "data"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = fs.read_to_string("/project/deep/nested/notes.md").await.unwrap();
        assert_eq!(content, "data");
    }

    #[tokio::test]
    async fn append_empty_path_returns_error() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c4", json!({"path": "", "content": "data"}), None)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn append_string_encoded_args() {
        let (fs, tool) = setup().await;
        let args_as_string = r#"{"path": "encoded.md", "content": "hello"}"#;
        let result = tool
            .execute("c5", serde_json::Value::String(args_as_string.to_string()), None)
            .await
            .unwrap();

        assert!(!result.is_error, "should succeed: {}", result.content);
        let content = fs.read_to_string("/project/encoded.md").await.unwrap();
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn multiple_appends_accumulate() {
        let (fs, tool) = setup().await;
        for i in 1..=3 {
            tool.execute("cx", json!({"path": "log.txt", "content": format!("entry {}\n", i)}), None)
                .await
                .unwrap();
        }
        let content = fs.read_to_string("/project/log.txt").await.unwrap();
        assert_eq!(content, "entry 1\nentry 2\nentry 3\n");
    }

    #[test]
    fn tool_name_and_definition() {
        let fs = Arc::new(MemoryFs::new());
        let tool = AppendTool::new(fs as Arc<dyn VirtualFs>, "/project");
        assert_eq!(tool.name(), "append");
        let def = tool.definition();
        assert_eq!(def.name, "append");
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("content")));
    }
}
