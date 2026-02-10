//! Read tool — read file contents with line numbers, offset, and truncation.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

use crate::truncate::{add_line_numbers, truncate_head, MAX_BYTES, MAX_LINES};

use super::resolve_path;

pub struct ReadTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl ReadTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "Read the contents of a file. Returns line-numbered output. Use offset and limit for large files.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to read (relative to working directory or absolute)"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "1-indexed line number to start reading from"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Number of lines to read"
                    }
                },
                "required": ["path"]
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

        if path.is_empty() {
            return Ok(ToolOutput::error("Missing required parameter: path"));
        }

        let resolved = resolve_path(&self.cwd, path);

        let exists = self.fs.exists(&resolved).await?;
        if !exists {
            return Ok(ToolOutput::error(format!("File not found: {}", path)));
        }

        let content = match self.fs.read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolOutput::error(format!("Failed to read {}: {}", path, e))),
        };

        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(1);

        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let total_lines = content.lines().count();

        if offset < 1 {
            return Ok(ToolOutput::error("offset must be >= 1"));
        }

        // Extract the requested range
        let lines: Vec<&str> = content.lines().collect();
        let start_idx = (offset - 1).min(lines.len());
        let end_idx = match limit {
            Some(l) => (start_idx + l).min(lines.len()),
            None => lines.len(),
        };

        if start_idx >= lines.len() {
            return Ok(ToolOutput::error(format!(
                "offset {} exceeds file length ({} lines)",
                offset, total_lines
            )));
        }

        let selected: String = lines[start_idx..end_idx].join("\n");

        // Apply truncation
        let max_lines = limit.unwrap_or(MAX_LINES).min(MAX_LINES);
        let result = truncate_head(&selected, max_lines, MAX_BYTES);

        let numbered = add_line_numbers(&result.content, offset);

        let mut output = numbered;

        if result.is_truncated() {
            if let Some(notice) = result.truncation_notice() {
                output.push('\n');
                output.push_str(&notice);
            }
            // Suggest next read parameters
            let next_offset = offset + result.output_lines;
            let remaining = total_lines.saturating_sub(next_offset - 1);
            if remaining > 0 {
                output.push_str(&format!(
                    "\n[To continue reading: offset={}, limit={}]",
                    next_offset,
                    remaining.min(MAX_LINES)
                ));
            }
        }

        Ok(ToolOutput::success(output).with_metadata(json!({
            "total_lines": total_lines,
            "offset": offset,
            "lines_returned": result.output_lines,
            "truncated": result.is_truncated(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vfs::MemoryFs;

    async fn setup() -> (Arc<MemoryFs>, ReadTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = ReadTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    #[tokio::test]
    async fn read_file() {
        let (fs, tool) = setup().await;
        fs.write("/project/hello.txt", "line1\nline2\nline3")
            .await
            .unwrap();

        let result = tool
            .execute("c1", json!({"path": "hello.txt"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("line1"));
        assert!(result.content.contains("line2"));
        assert!(result.content.contains("line3"));
    }

    #[tokio::test]
    async fn read_with_offset_and_limit() {
        let (fs, tool) = setup().await;
        let content = (1..=10).map(|i| format!("line{}", i)).collect::<Vec<_>>().join("\n");
        fs.write("/project/big.txt", &content).await.unwrap();

        let result = tool
            .execute("c2", json!({"path": "big.txt", "offset": 3, "limit": 2}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("line3"));
        assert!(result.content.contains("line4"));
        assert!(!result.content.contains("line5"));
    }

    #[tokio::test]
    async fn read_nonexistent() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c3", json!({"path": "nope.txt"}), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn read_absolute_path() {
        let (fs, tool) = setup().await;
        fs.write("/abs/file.txt", "absolute").await.unwrap();

        let result = tool
            .execute("c4", json!({"path": "/abs/file.txt"}), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("absolute"));
    }

    #[tokio::test]
    async fn read_empty_path() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c5", json!({"path": ""}), None)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn read_offset_beyond_file() {
        let (fs, tool) = setup().await;
        fs.write("/project/short.txt", "one\ntwo").await.unwrap();

        let result = tool
            .execute("c6", json!({"path": "short.txt", "offset": 100}), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("exceeds"));
    }

    #[tokio::test]
    async fn tool_name_and_definition() {
        let (_fs, tool) = setup().await;
        assert_eq!(tool.name(), "read");
        let def = tool.definition();
        assert_eq!(def.name, "read");
        assert!(def.description.contains("Read"));
    }
}
