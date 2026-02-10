//! Ls tool — list directory contents with metadata.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

/// Maximum entries returned.
const MAX_ENTRIES: usize = 500;

use super::resolve_path;

pub struct LsTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl LsTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ls".into(),
            description: "List the contents of a directory. Shows files and subdirectories with '/' suffix for directories.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (defaults to working directory)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum entries to return (default: 500)"
                    }
                }
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

        let resolved = if path.is_empty() {
            self.cwd.clone()
        } else {
            resolve_path(&self.cwd, path)
        };

        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).min(MAX_ENTRIES))
            .unwrap_or(MAX_ENTRIES);

        // Check if path exists
        let exists = self.fs.exists(&resolved).await?;
        if !exists {
            return Ok(ToolOutput::error(format!(
                "Directory not found: {}",
                if path.is_empty() { &self.cwd } else { path }
            )));
        }

        let entries = match self.fs.read_dir(&resolved).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to read directory {}: {}",
                    path, e
                )));
            }
        };

        // Sort alphabetically (case-insensitive)
        let mut sorted: Vec<_> = entries.into_iter().collect();
        sorted.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
        });

        let total = sorted.len();
        let displayed: Vec<String> = sorted
            .iter()
            .take(limit)
            .map(|e| {
                if e.is_dir {
                    format!("{}/", e.name)
                } else {
                    e.name.clone()
                }
            })
            .collect();

        let mut output = displayed.join("\n");

        if total > limit {
            output.push_str(&format!(
                "\n[Showing {} of {} entries]",
                limit, total
            ));
        }

        if total == 0 {
            output = "(empty directory)".into();
        }

        Ok(ToolOutput::success(output).with_metadata(json!({
            "total_entries": total,
            "displayed": displayed.len(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vfs::MemoryFs;

    async fn setup() -> (Arc<MemoryFs>, LsTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = LsTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    #[tokio::test]
    async fn ls_directory() {
        let (fs, tool) = setup().await;
        fs.write("/project/file.txt", "content").await.unwrap();
        fs.write("/project/code.rs", "fn main() {}").await.unwrap();
        fs.write("/project/sub/nested.txt", "nested").await.unwrap();

        let result = tool
            .execute("c1", json!({}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("file.txt"));
        assert!(result.content.contains("code.rs"));
        assert!(result.content.contains("sub/"));
    }

    #[tokio::test]
    async fn ls_subdirectory() {
        let (fs, tool) = setup().await;
        fs.write("/project/src/main.rs", "fn main() {}").await.unwrap();

        let result = tool
            .execute("c2", json!({"path": "src"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("main.rs"));
    }

    #[tokio::test]
    async fn ls_empty_dir() {
        let (fs, tool) = setup().await;
        fs.create_dir_all("/project/empty").await.unwrap();

        let result = tool
            .execute("c3", json!({"path": "empty"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("empty directory"));
    }

    #[tokio::test]
    async fn ls_nonexistent() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c4", json!({"path": "nope"}), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn ls_with_limit() {
        let (fs, tool) = setup().await;
        for i in 0..10 {
            fs.write(&format!("/project/file{}.txt", i), "").await.unwrap();
        }

        let result = tool
            .execute("c5", json!({"limit": 3}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Showing 3 of 10"));
    }

    #[tokio::test]
    async fn ls_sorted_case_insensitive() {
        let (fs, tool) = setup().await;
        fs.write("/project/Banana.txt", "").await.unwrap();
        fs.write("/project/apple.txt", "").await.unwrap();
        fs.write("/project/Cherry.txt", "").await.unwrap();

        let result = tool
            .execute("c6", json!({}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        let lines: Vec<&str> = result.content.lines().collect();
        assert_eq!(lines[0], "apple.txt");
        assert_eq!(lines[1], "Banana.txt");
        assert_eq!(lines[2], "Cherry.txt");
    }

    #[tokio::test]
    async fn tool_name_and_definition() {
        let (_fs, tool) = setup().await;
        assert_eq!(tool.name(), "ls");
        let def = tool.definition();
        assert_eq!(def.name, "ls");
    }
}
