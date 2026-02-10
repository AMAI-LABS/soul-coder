//! Edit tool — precise text replacement with exact matching and fuzzy fallback.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use similar::{ChangeTag, TextDiff};
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vfs::VirtualFs;

use super::resolve_path;

pub struct EditTool {
    fs: Arc<dyn VirtualFs>,
    cwd: String,
}

impl EditTool {
    pub fn new(fs: Arc<dyn VirtualFs>, cwd: impl Into<String>) -> Self {
        Self {
            fs,
            cwd: cwd.into(),
        }
    }
}

/// Normalize text for fuzzy matching: trim trailing whitespace per line,
/// normalize smart quotes to ASCII, normalize unicode dashes.
fn normalize_for_fuzzy(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim_end();
            trimmed
                .replace('\u{2018}', "'")  // left single quote
                .replace('\u{2019}', "'")  // right single quote
                .replace('\u{201C}', "\"") // left double quote
                .replace('\u{201D}', "\"") // right double quote
                .replace('\u{2013}', "-")  // en dash
                .replace('\u{2014}', "-")  // em dash
                .replace('\u{00A0}', " ")  // non-breaking space
                .replace('\u{202F}', " ")  // narrow no-break space
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate a unified diff between old and new content.
fn unified_diff(old: &str, new: &str, path: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut output = format!("--- a/{}\n+++ b/{}\n", path, path);

    let mut udiff = diff.unified_diff();
    output.push_str(&udiff.header("", "").to_string());

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        output.push_str(&format!("{}{}", sign, change));
        if change.missing_newline() {
            output.push('\n');
        }
    }

    output
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit".into(),
            description: "Perform an exact text replacement in a file. The old text must match uniquely. Falls back to fuzzy matching (smart quote normalization, trailing whitespace) if exact match fails.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit"
                    },
                    "old": {
                        "type": "string",
                        "description": "Exact text to find and replace"
                    },
                    "new": {
                        "type": "string",
                        "description": "Replacement text"
                    }
                },
                "required": ["path", "old", "new"]
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
        let old_text = arguments
            .get("old")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_text = arguments
            .get("new")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if path.is_empty() {
            return Ok(ToolOutput::error("Missing required parameter: path"));
        }
        if old_text.is_empty() {
            return Ok(ToolOutput::error("Missing required parameter: old"));
        }
        if old_text == new_text {
            return Ok(ToolOutput::error(
                "old and new text are identical — no change would occur",
            ));
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

        // Phase 1: exact match
        let matches: Vec<_> = content.match_indices(old_text).collect();

        let (new_content, method) = if matches.len() == 1 {
            (content.replacen(old_text, new_text, 1), "exact")
        } else if matches.len() > 1 {
            return Ok(ToolOutput::error(format!(
                "Found {} occurrences of the old text — must be unique. Provide more context to disambiguate.",
                matches.len()
            )));
        } else {
            // Phase 2: fuzzy match
            let norm_content = normalize_for_fuzzy(&content);
            let norm_old = normalize_for_fuzzy(old_text);

            let fuzzy_matches: Vec<_> = norm_content.match_indices(&norm_old).collect();

            if fuzzy_matches.len() == 1 {
                // Find the corresponding position in the original content
                let fuzzy_pos = fuzzy_matches[0].0;
                // Map normalized position back to original by matching line-by-line
                let norm_lines_before = norm_content[..fuzzy_pos].lines().count();
                let original_lines: Vec<&str> = content.lines().collect();
                let search_lines: Vec<&str> = old_text.lines().collect();

                if norm_lines_before > 0 && norm_lines_before <= original_lines.len() {
                    let start_line = norm_lines_before.saturating_sub(1);
                    let end_line = (start_line + search_lines.len()).min(original_lines.len());
                    let original_section = original_lines[start_line..end_line].join("\n");
                    (content.replacen(&original_section, new_text, 1), "fuzzy")
                } else {
                    // Fallback: replace in normalized then write
                    let result = norm_content.replacen(&norm_old, new_text, 1);
                    (result, "fuzzy")
                }
            } else if fuzzy_matches.len() > 1 {
                return Ok(ToolOutput::error(format!(
                    "Found {} fuzzy occurrences — must be unique. Provide more context.",
                    fuzzy_matches.len()
                )));
            } else {
                return Ok(ToolOutput::error(
                    "Text not found in file (tried exact and fuzzy matching). Verify the old text matches the file content.",
                ));
            }
        };

        // Write the modified content
        match self.fs.write(&resolved, &new_content).await {
            Ok(()) => {
                let diff = unified_diff(&content, &new_content, path);
                // Find first changed line
                let first_changed_line = content
                    .lines()
                    .zip(new_content.lines())
                    .enumerate()
                    .find(|(_, (a, b))| a != b)
                    .map(|(i, _)| i + 1)
                    .unwrap_or(1);

                Ok(ToolOutput::success(format!(
                    "Applied edit to {} ({})\n\n{}",
                    path, method, diff
                ))
                .with_metadata(json!({
                    "method": method,
                    "first_changed_line": first_changed_line,
                    "path": path,
                })))
            }
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

    async fn setup() -> (Arc<MemoryFs>, EditTool) {
        let fs = Arc::new(MemoryFs::new());
        let tool = EditTool::new(fs.clone() as Arc<dyn VirtualFs>, "/project");
        (fs, tool)
    }

    #[tokio::test]
    async fn exact_replacement() {
        let (fs, tool) = setup().await;
        fs.write("/project/code.rs", "fn main() {\n    println!(\"hello\");\n}")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c1",
                json!({
                    "path": "code.rs",
                    "old": "println!(\"hello\")",
                    "new": "println!(\"world\")"
                }),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("exact"));
        let content = fs.read_to_string("/project/code.rs").await.unwrap();
        assert!(content.contains("world"));
        assert!(!content.contains("hello"));
    }

    #[tokio::test]
    async fn fuzzy_smart_quotes() {
        let (fs, tool) = setup().await;
        fs.write("/project/quotes.txt", "It\u{2019}s a test")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c2",
                json!({
                    "path": "quotes.txt",
                    "old": "It's a test",
                    "new": "It is a test"
                }),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("fuzzy"));
    }

    #[tokio::test]
    async fn multiple_matches_error() {
        let (fs, tool) = setup().await;
        fs.write("/project/dup.txt", "hello hello hello")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c3",
                json!({"path": "dup.txt", "old": "hello", "new": "world"}),
                None,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("occurrences"));
    }

    #[tokio::test]
    async fn text_not_found() {
        let (fs, tool) = setup().await;
        fs.write("/project/missing.txt", "something else")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c4",
                json!({"path": "missing.txt", "old": "nothere", "new": "replacement"}),
                None,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn identical_old_new() {
        let (fs, tool) = setup().await;
        fs.write("/project/same.txt", "content").await.unwrap();

        let result = tool
            .execute(
                "c5",
                json!({"path": "same.txt", "old": "content", "new": "content"}),
                None,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("identical"));
    }

    #[tokio::test]
    async fn file_not_found() {
        let (_fs, tool) = setup().await;
        let result = tool
            .execute(
                "c6",
                json!({"path": "nope.txt", "old": "a", "new": "b"}),
                None,
            )
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn diff_output() {
        let (fs, tool) = setup().await;
        fs.write("/project/diff.txt", "line1\nline2\nline3")
            .await
            .unwrap();

        let result = tool
            .execute(
                "c7",
                json!({"path": "diff.txt", "old": "line2", "new": "modified"}),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("-line2"));
        assert!(result.content.contains("+modified"));
    }

    #[test]
    fn normalize_fuzzy_quotes() {
        let input = "\u{201C}hello\u{201D} \u{2018}world\u{2019}";
        let normalized = normalize_for_fuzzy(input);
        assert_eq!(normalized, "\"hello\" 'world'");
    }

    #[test]
    fn normalize_fuzzy_dashes() {
        let input = "a\u{2013}b\u{2014}c";
        let normalized = normalize_for_fuzzy(input);
        assert_eq!(normalized, "a-b-c");
    }

    #[test]
    fn normalize_fuzzy_trailing_whitespace() {
        let input = "hello   \nworld  ";
        let normalized = normalize_for_fuzzy(input);
        assert_eq!(normalized, "hello\nworld");
    }

    #[test]
    fn tool_name_and_definition() {
        let fs = Arc::new(MemoryFs::new());
        let tool = EditTool::new(fs as Arc<dyn VirtualFs>, "/");
        assert_eq!(tool.name(), "edit");
        let def = tool.definition();
        assert_eq!(def.name, "edit");
    }
}
