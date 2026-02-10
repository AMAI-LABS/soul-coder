//! Write tool — create or overwrite files, auto-creating parent directories.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

use super::resolve_path;

pub struct WriteTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl WriteTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "Write content to a file. Creates the file and parent directories if they don't exist. Overwrites existing files.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to write to (relative to working directory or absolute)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
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

        match self.fs.write(&resolved, content).await {
            Ok(()) => Ok(ToolOutput::success(format!(
                "Wrote {} bytes to {}",
                content.len(),
                path
            ))
            .with_metadata(json!({
                "bytes_written": content.len(),
                "path": path,
            }))),
            Err(e) => Ok(ToolOutput::error(format!(
                "Failed to write {}: {}",
                path, e
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vfs::MemoryFs;

    async fn setup() -> (Arc<MemoryFs>, WriteTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = WriteTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    #[tokio::test]
    async fn write_new_file() {
        let (fs, tool) = setup().await;
        let result = tool
            .execute("c1", json!({"path": "new.txt", "content": "hello world"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("11 bytes"));

        let content = fs.read_to_string("/project/new.txt").await.unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn write_creates_parent_dirs() {
        let (fs, tool) = setup().await;
        let result = tool
            .execute(
                "c2",
                json!({"path": "deep/nested/dir/file.txt", "content": "deep"}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = fs.read_to_string("/project/deep/nested/dir/file.txt").await.unwrap();
        assert_eq!(content, "deep");
    }

    #[tokio::test]
    async fn write_overwrites() {
        let (fs, tool) = setup().await;
        fs.write("/project/existing.txt", "old content").await.unwrap();

        let result = tool
            .execute(
                "c3",
                json!({"path": "existing.txt", "content": "new content"}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = fs.read_to_string("/project/existing.txt").await.unwrap();
        assert_eq!(content, "new content");
    }

    #[tokio::test]
    async fn write_empty_path() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c4", json!({"path": "", "content": "data"}), None)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn write_absolute_path() {
        let (fs, tool) = setup().await;
        let result = tool
            .execute(
                "c5",
                json!({"path": "/abs/file.txt", "content": "abs"}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = fs.read_to_string("/abs/file.txt").await.unwrap();
        assert_eq!(content, "abs");
    }

    #[tokio::test]
    async fn tool_name_and_definition() {
        let (_fs, tool) = setup().await;
        assert_eq!(tool.name(), "write");
        let def = tool.definition();
        assert_eq!(def.name, "write");
    }
}
