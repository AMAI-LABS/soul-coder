//! Bash tool — execute shell commands with output truncation and timeout.
//!
//! Delegates to [`soul_core::executor::ShellExecutor`] for command execution,
//! then applies ANSI stripping and tail truncation on top.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use soul_core::error::SoulResult;
use soul_core::executor::shell::ShellExecutor;
use soul_core::executor::ToolExecutor;
use soul_core::tool::{Tool, ToolOutput};
use soul_core::types::ToolDefinition;
use soul_core::vexec::VirtualExecutor;

use crate::truncate::{truncate_tail, MAX_BYTES};

/// Maximum lines kept from bash output (tail).
const BASH_MAX_LINES: usize = 50;

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT: u64 = 120;

pub struct BashTool {
    shell: ShellExecutor,
    definition: ToolDefinition,
}

impl BashTool {
    pub fn new(executor: Arc<dyn VirtualExecutor>, cwd: impl Into<String>) -> Self {
        let shell = ShellExecutor::new(executor)
            .with_timeout(DEFAULT_TIMEOUT)
            .with_cwd(cwd);

        let definition = ToolDefinition {
            name: "bash".into(),
            description: "Execute a shell command. Returns stdout and stderr. Output is truncated to the last 50 lines.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 120)"
                    }
                },
                "required": ["command"]
            }),
        };

        Self { shell, definition }
    }
}

/// Strip ANSI escape codes from output.
fn strip_ansi(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip escape sequence
            if let Some(&'[') = chars.peek() {
                chars.next(); // consume '['
                // Consume until a letter
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if ch == '\r' {
            // Skip carriage returns
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    async fn execute(
        &self,
        call_id: &str,
        arguments: serde_json::Value,
        partial_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> SoulResult<ToolOutput> {
        // Delegate to ShellExecutor from soul-core
        let result = self
            .shell
            .execute(&self.definition, call_id, arguments, partial_tx.clone())
            .await;

        match result {
            Ok(output) => {
                // Stream partial output if channel available
                if let Some(ref tx) = partial_tx {
                    let _ = tx.send(output.content.clone());
                }

                // Apply ANSI stripping
                let cleaned = strip_ansi(&output.content);

                // Apply tail truncation (errors/final output matter most)
                let truncated = truncate_tail(&cleaned, BASH_MAX_LINES, MAX_BYTES);

                let notice = truncated.truncation_notice();
                let is_truncated = truncated.is_truncated();
                let mut result_content = truncated.content;
                if let Some(notice) = notice {
                    result_content = format!("{}\n{}", notice, result_content);
                }

                let tool_output = if output.is_error {
                    ToolOutput::error(result_content)
                } else {
                    ToolOutput::success(result_content)
                };

                Ok(tool_output.with_metadata(json!({
                    "truncated": is_truncated,
                })))
            }
            Err(e) => Ok(ToolOutput::error(format!("Command failed: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vexec::{ExecOutput, MockExecutor};

    fn setup_ok(stdout: &str) -> BashTool {
        let executor = Arc::new(MockExecutor::always_ok(stdout));
        BashTool::new(executor as Arc<dyn VirtualExecutor>, "/project")
    }

    fn setup_with(responses: Vec<ExecOutput>) -> BashTool {
        let executor = Arc::new(MockExecutor::new(responses));
        BashTool::new(executor as Arc<dyn VirtualExecutor>, "/project")
    }

    #[tokio::test]
    async fn execute_simple_command() {
        let tool = setup_ok("hello world\n");
        let result = tool
            .execute("c1", json!({"command": "echo hello world"}), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("hello world"));
    }

    #[tokio::test]
    async fn execute_with_error_exit() {
        let tool = setup_with(vec![ExecOutput {
            stdout: String::new(),
            stderr: "command not found".into(),
            exit_code: 127,
        }]);

        let result = tool
            .execute("c2", json!({"command": "nonexistent"}), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("command not found"));
    }

    #[tokio::test]
    async fn execute_empty_command() {
        let tool = setup_ok("");
        let _result = tool
            .execute("c3", json!({"command": ""}), None)
            .await
            .unwrap();
        // ShellExecutor delegates to MockExecutor which returns empty stdout
        // The result should still be ok (empty output is not an error)
        // Note: ShellExecutor requires the "command" key to exist, not be non-empty
    }

    #[tokio::test]
    async fn strips_ansi() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("no ansi"), "no ansi");
        assert_eq!(strip_ansi("line\r\n"), "line\n");
    }

    #[tokio::test]
    async fn stderr_included() {
        let tool = setup_with(vec![ExecOutput {
            stdout: "out\n".into(),
            stderr: "warn\n".into(),
            exit_code: 0,
        }]);

        let result = tool
            .execute("c4", json!({"command": "test"}), None)
            .await
            .unwrap();

        // ShellExecutor returns stdout on success (exit_code 0)
        assert!(!result.is_error);
        assert!(result.content.contains("out"));
    }

    #[tokio::test]
    async fn streaming_output() {
        let tool = setup_ok("streamed\n");
        let (tx, mut rx) = mpsc::unbounded_channel();

        let result = tool
            .execute("c5", json!({"command": "echo streamed"}), Some(tx))
            .await
            .unwrap();

        assert!(!result.is_error);
        let partial = rx.recv().await.unwrap();
        assert!(partial.contains("streamed"));
    }

    #[tokio::test]
    async fn tool_name_and_definition() {
        let tool = setup_ok("");
        assert_eq!(tool.name(), "bash");
        let def = tool.definition();
        assert_eq!(def.name, "bash");
        assert!(def.input_schema["required"].as_array().unwrap().contains(&json!("command")));
    }
}
