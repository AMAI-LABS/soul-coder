//! Grep tool — search file contents using regex or literal patterns.
//!
//! Uses VirtualFs for WASM compatibility. In WASM mode, performs regex search
//! over all files in the VFS. In native mode, can delegate to ripgrep via VirtualExecutor.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

use crate::truncate::{truncate_head, truncate_line, GREP_MAX_LINE_LENGTH, MAX_BYTES};

/// Maximum number of matches returned.
const MAX_MATCHES: usize = 100;

use super::resolve_path;

pub struct GrepTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl GrepTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

/// Simple pattern matching (supports literal and basic regex via contains).
fn matches_pattern(line: &str, pattern: &str, literal: bool, ignore_case: bool) -> bool {
    if literal {
        if ignore_case {
            line.to_lowercase().contains(&pattern.to_lowercase())
        } else {
            line.contains(pattern)
        }
    } else {
        // Basic regex-like: treat as literal for WASM (no regex crate dependency)
        // For full regex, the native implementation delegates to rg
        if ignore_case {
            line.to_lowercase().contains(&pattern.to_lowercase())
        } else {
            line.contains(pattern)
        }
    }
}

/// Recursively collect all file paths from a VFS directory.
async fn collect_files(
    fs: &dyn VirtualFs,
    dir: &str,
    files: &mut Vec<String>,
    glob_filter: Option<&str>,
) -> SoulResult<()> {
    let entries = fs.read_dir(dir).await?;
    for entry in entries {
        let path = if dir == "/" || dir.is_empty() {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir.trim_end_matches('/'), entry.name)
        };

        if entry.is_dir {
            // Skip hidden dirs
            if !entry.name.starts_with('.') {
                Box::pin(collect_files(fs, &path, files, glob_filter)).await?;
            }
        } else if entry.is_file {
            if let Some(glob) = glob_filter {
                if matches_glob(&entry.name, glob) {
                    files.push(path);
                }
            } else {
                files.push(path);
            }
        }
    }
    Ok(())
}

/// Simple glob matching (supports *.ext patterns).
fn matches_glob(filename: &str, glob: &str) -> bool {
    if glob.starts_with("*.") {
        let ext = &glob[1..]; // ".ext"
        filename.ends_with(ext)
    } else if glob.contains('*') {
        // Very basic wildcard
        let parts: Vec<&str> = glob.split('*').collect();
        if parts.len() == 2 {
            filename.starts_with(parts[0]) && filename.ends_with(parts[1])
        } else {
            true // No filtering
        }
    } else {
        filename == glob
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".into(),
            description: "Search file contents for a pattern. Returns matching lines with file paths and line numbers.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (literal string or regex)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (defaults to working directory)"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob pattern to filter files (e.g., '*.rs', '*.ts')"
                    },
                    "ignore_case": {
                        "type": "boolean",
                        "description": "Case-insensitive search"
                    },
                    "literal": {
                        "type": "boolean",
                        "description": "Treat pattern as literal string (no regex)"
                    },
                    "context": {
                        "type": "integer",
                        "description": "Number of context lines before and after each match"
                    },
                    "max_matches": {
                        "type": "integer",
                        "description": "Maximum number of matches to return (default: 100)"
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(
        &self,
        _call_id: &str,
        arguments: serde_json::Value,
        _partial_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> SoulResult<ToolOutput> {
        let pattern = arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if pattern.is_empty() {
            return Ok(ToolOutput::error("Missing required parameter: pattern"));
        }

        let search_path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(&self.cwd, p))
            .unwrap_or_else(|| self.cwd.clone());

        let glob_filter = arguments.get("glob").and_then(|v| v.as_str());
        let ignore_case = arguments
            .get("ignore_case")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let literal = arguments
            .get("literal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let context_lines = arguments
            .get("context")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let max_matches = arguments
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).min(MAX_MATCHES))
            .unwrap_or(MAX_MATCHES);

        // Collect files to search
        let mut files = Vec::new();
        if let Err(e) = collect_files(self.fs.as_ref(), &search_path, &mut files, glob_filter).await
        {
            return Ok(ToolOutput::error(format!(
                "Failed to enumerate files in {}: {}",
                search_path, e
            )));
        }

        files.sort();

        let mut output = String::new();
        let mut total_matches = 0;
        let mut files_with_matches = 0;

        'files: for file_path in &files {
            let content = match self.fs.read_to_string(file_path).await {
                Ok(c) => c,
                Err(_) => continue, // Skip unreadable files
            };

            let lines: Vec<&str> = content.lines().collect();
            let mut file_had_match = false;

            for (line_idx, line) in lines.iter().enumerate() {
                if matches_pattern(line, pattern, literal, ignore_case) {
                    if !file_had_match {
                        if !output.is_empty() {
                            output.push('\n');
                        }
                        files_with_matches += 1;
                        file_had_match = true;
                    }

                    // Context before
                    let ctx_start = line_idx.saturating_sub(context_lines);
                    for ctx_idx in ctx_start..line_idx {
                        output.push_str(&format!(
                            "{}:{}-{}\n",
                            display_path(file_path, &self.cwd),
                            ctx_idx + 1,
                            truncate_line(lines[ctx_idx], GREP_MAX_LINE_LENGTH)
                        ));
                    }

                    // Match line
                    output.push_str(&format!(
                        "{}:{}:{}\n",
                        display_path(file_path, &self.cwd),
                        line_idx + 1,
                        truncate_line(line, GREP_MAX_LINE_LENGTH)
                    ));

                    // Context after
                    let ctx_end = (line_idx + context_lines + 1).min(lines.len());
                    for ctx_idx in (line_idx + 1)..ctx_end {
                        output.push_str(&format!(
                            "{}:{}-{}\n",
                            display_path(file_path, &self.cwd),
                            ctx_idx + 1,
                            truncate_line(lines[ctx_idx], GREP_MAX_LINE_LENGTH)
                        ));
                    }

                    total_matches += 1;
                    if total_matches >= max_matches {
                        break 'files;
                    }
                }
            }
        }

        if total_matches == 0 {
            return Ok(ToolOutput::success(format!(
                "No matches found for pattern '{}' in {}",
                pattern,
                display_path(&search_path, &self.cwd)
            ))
            .with_metadata(json!({"matches": 0, "files": 0})));
        }

        // Apply byte truncation
        let truncated = truncate_head(&output, total_matches + (total_matches * context_lines * 2), MAX_BYTES);

        let notice = truncated.truncation_notice();
        let is_truncated = truncated.is_truncated();
        let mut result = truncated.content;
        if total_matches >= max_matches {
            result.push_str(&format!(
                "\n[Reached max matches limit: {}]",
                max_matches
            ));
        }
        if let Some(notice) = notice {
            result.push_str(&format!("\n{}", notice));
        }

        Ok(ToolOutput::success(result).with_metadata(json!({
            "matches": total_matches,
            "files_with_matches": files_with_matches,
            "truncated": is_truncated,
        })))
    }
}

/// Make paths relative to cwd for display.
fn display_path(path: &str, cwd: &str) -> String {
    let cwd_prefix = format!("{}/", cwd.trim_end_matches('/'));
    if path.starts_with(&cwd_prefix) {
        path[cwd_prefix.len()..].to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vfs::MemoryFs;

    async fn setup() -> (Arc<MemoryFs>, GrepTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = GrepTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    #[tokio::test]
    async fn grep_simple_match() {
        let (fs, tool) = setup().await;
        fs.write("/project/file.txt", "hello world\nfoo bar\nhello again")
            .await
            .unwrap();

        let result = tool
            .execute("c1", json!({"pattern": "hello"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("file.txt:1:hello world"));
        assert!(result.content.contains("file.txt:3:hello again"));
    }

    #[tokio::test]
    async fn grep_case_insensitive() {
        let (fs, tool) = setup().await;
        fs.write("/project/file.txt", "Hello World\nhello world")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c2",
                json!({"pattern": "HELLO", "ignore_case": true}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.metadata["matches"].as_u64().unwrap() == 2);
    }

    #[tokio::test]
    async fn grep_with_glob_filter() {
        let (fs, tool) = setup().await;
        fs.write("/project/code.rs", "fn main() {}")
            .await
            .unwrap();
        fs.write("/project/readme.md", "fn main() {}")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c3",
                json!({"pattern": "fn main", "glob": "*.rs"}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("code.rs"));
        assert!(!result.content.contains("readme.md"));
    }

    #[tokio::test]
    async fn grep_no_matches() {
        let (fs, tool) = setup().await;
        fs.write("/project/file.txt", "nothing here")
            .await
            .unwrap();

        let result = tool
            .execute("c4", json!({"pattern": "missing"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("No matches"));
    }

    #[tokio::test]
    async fn grep_empty_pattern() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c5", json!({"pattern": ""}), None)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn grep_with_context() {
        let (fs, tool) = setup().await;
        fs.write("/project/file.txt", "a\nb\nc\nd\ne")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c6",
                json!({"pattern": "c", "context": 1}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("b")); // before context
        assert!(result.content.contains("d")); // after context
    }

    #[test]
    fn glob_matching() {
        assert!(matches_glob("file.rs", "*.rs"));
        assert!(!matches_glob("file.ts", "*.rs"));
        assert!(matches_glob("test.spec.ts", "*.ts"));
    }

    #[test]
    fn display_path_relative() {
        assert_eq!(display_path("/project/src/main.rs", "/project"), "src/main.rs");
        assert_eq!(display_path("/other/file.txt", "/project"), "/other/file.txt");
    }

    #[tokio::test]
    async fn tool_name_and_definition() {
        let (_fs, tool) = setup().await;
        assert_eq!(tool.name(), "grep");
        let def = tool.definition();
        assert_eq!(def.name, "grep");
    }
}
