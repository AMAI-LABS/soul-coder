//! Find tool — search for files by name/glob pattern.
//!
//! Uses VirtualFs for WASM compatibility. Recursively walks directories
//! and matches filenames against glob patterns.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

use crate::truncate::{truncate_head, MAX_BYTES};

/// Maximum results returned.
const MAX_RESULTS: usize = 1000;

use super::resolve_path;

pub struct FindTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl FindTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

/// Match a filename against a glob pattern.
/// Supports: *.ext, prefix*, *suffix, exact match, **/ (recursive, treated as *)
fn matches_glob(name: &str, full_path: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();

    // Handle **/ patterns (recursive) - match against full path
    if pattern.contains("**/") || pattern.contains("/**") {
        let simple = pattern.replace("**/", "").replace("/**", "");
        return matches_simple_glob(name, &simple) || matches_simple_glob(full_path, pattern);
    }

    // Handle path patterns (containing /)
    if pattern.contains('/') {
        return path_matches_glob(full_path, pattern);
    }

    matches_simple_glob(name, pattern)
}

fn matches_simple_glob(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.starts_with("*.") {
        let ext = &pattern[1..];
        return name.ends_with(ext);
    }

    if pattern.starts_with('*') && pattern.ends_with('*') && pattern.len() > 2 {
        let middle = &pattern[1..pattern.len() - 1];
        return name.contains(middle);
    }

    if pattern.starts_with('*') {
        let suffix = &pattern[1..];
        return name.ends_with(suffix);
    }

    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return name.starts_with(prefix);
    }

    name == pattern
}

fn path_matches_glob(path: &str, pattern: &str) -> bool {
    let path_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let pattern_parts: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

    if pattern_parts.is_empty() {
        return true;
    }

    // Match from the end (most specific part first)
    let mut pi = pattern_parts.len();
    let mut qi = path_parts.len();

    while pi > 0 && qi > 0 {
        pi -= 1;
        qi -= 1;
        if pattern_parts[pi] == "**" {
            return true; // Matches any depth
        }
        if !matches_simple_glob(path_parts[qi], pattern_parts[pi]) {
            return false;
        }
    }

    pi == 0
}

/// Recursively collect matching files.
async fn find_files(
    fs: &dyn VirtualFs,
    dir: &str,
    pattern: &str,
    results: &mut Vec<String>,
    limit: usize,
) -> SoulResult<()> {
    if results.len() >= limit {
        return Ok(());
    }

    let entries = match fs.read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return Ok(()), // Skip unreadable dirs
    };

    for entry in entries {
        if results.len() >= limit {
            break;
        }

        let path = if dir == "/" || dir.is_empty() {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir.trim_end_matches('/'), entry.name)
        };

        if entry.is_dir {
            if !entry.name.starts_with('.') {
                Box::pin(find_files(fs, &path, pattern, results, limit)).await?;
            }
        } else if entry.is_file && matches_glob(&entry.name, &path, pattern) {
            results.push(path);
        }
    }

    Ok(())
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".into(),
            description: "Find files matching a glob pattern. Returns matching file paths.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match files (e.g., '*.rs', 'src/**/*.ts', 'Cargo.toml')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (defaults to working directory)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 1000)"
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

        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).min(MAX_RESULTS))
            .unwrap_or(MAX_RESULTS);

        let mut results = Vec::new();
        if let Err(e) =
            find_files(self.fs.as_ref(), &search_path, pattern, &mut results, limit).await
        {
            return Ok(ToolOutput::error(format!(
                "Failed to search {}: {}",
                search_path, e
            )));
        }

        results.sort();

        if results.is_empty() {
            return Ok(ToolOutput::success(format!(
                "No files matching '{}' found",
                pattern
            ))
            .with_metadata(json!({"count": 0})));
        }

        // Make paths relative to cwd
        let cwd_prefix = format!("{}/", self.cwd.trim_end_matches('/'));
        let relative: Vec<String> = results
            .iter()
            .map(|p| {
                if p.starts_with(&cwd_prefix) {
                    p[cwd_prefix.len()..].to_string()
                } else {
                    p.clone()
                }
            })
            .collect();

        let output = relative.join("\n");
        let truncated = truncate_head(&output, results.len(), MAX_BYTES);

        let notice = truncated.truncation_notice();
        let mut result = truncated.content;
        if results.len() >= limit {
            result.push_str(&format!("\n[Reached limit: {} results]", limit));
        }
        if let Some(notice) = notice {
            result.push_str(&format!("\n{}", notice));
        }

        Ok(ToolOutput::success(result).with_metadata(json!({
            "count": results.len(),
            "limit_reached": results.len() >= limit,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vfs::MemoryFs;

    async fn setup() -> (Arc<MemoryFs>, FindTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = FindTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    async fn populate(fs: &MemoryFs) {
        fs.write("/project/src/main.rs", "fn main() {}")
            .await
            .unwrap();
        fs.write("/project/src/lib.rs", "pub mod foo;")
            .await
            .unwrap();
        fs.write("/project/src/utils.ts", "export {}")
            .await
            .unwrap();
        fs.write("/project/Cargo.toml", "[package]").await.unwrap();
        fs.write("/project/README.md", "# readme").await.unwrap();
    }

    #[tokio::test]
    async fn find_by_extension() {
        let (fs, tool) = setup().await;
        populate(&*fs).await;

        let result = tool
            .execute("c1", json!({"pattern": "*.rs"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("main.rs"));
        assert!(result.content.contains("lib.rs"));
        assert!(!result.content.contains("utils.ts"));
    }

    #[tokio::test]
    async fn find_exact_name() {
        let (fs, tool) = setup().await;
        populate(&*fs).await;

        let result = tool
            .execute("c2", json!({"pattern": "Cargo.toml"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Cargo.toml"));
        assert_eq!(result.metadata["count"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn find_no_results() {
        let (fs, tool) = setup().await;
        populate(&*fs).await;

        let result = tool
            .execute("c3", json!({"pattern": "*.py"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("No files"));
    }

    #[tokio::test]
    async fn find_with_limit() {
        let (fs, tool) = setup().await;
        populate(&*fs).await;

        let result = tool
            .execute("c4", json!({"pattern": "*", "limit": 2}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.metadata["count"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn find_empty_pattern() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute("c5", json!({"pattern": ""}), None)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn glob_extensions() {
        assert!(matches_glob("file.rs", "/src/file.rs", "*.rs"));
        assert!(!matches_glob("file.ts", "/src/file.ts", "*.rs"));
    }

    #[test]
    fn glob_prefix() {
        assert!(matches_glob("Cargo.toml", "/Cargo.toml", "Cargo*"));
        assert!(!matches_glob("package.json", "/package.json", "Cargo*"));
    }

    #[test]
    fn glob_exact() {
        assert!(matches_glob("Makefile", "/Makefile", "Makefile"));
        assert!(!matches_glob("makefile", "/makefile", "Makefile"));
    }

    #[tokio::test]
    async fn tool_name_and_definition() {
        let (_fs, tool) = setup().await;
        assert_eq!(tool.name(), "find");
        let def = tool.definition();
        assert_eq!(def.name, "find");
    }
}
