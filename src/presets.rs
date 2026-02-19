//! Preset tool collections for common use cases.
//!
//! Two integration modes:
//! - **ToolRegistry** (simple): `coding_tools()`, `read_only_tools()`, `all_tools()`
//! - **ExecutorRegistry** (config-driven): `coding_executor()`, `all_executor()`

use std::sync::Arc;

use soul_core::executor::direct::DirectExecutor;
use soul_core::executor::{ConfigTool, ExecutorRegistry, ToolExecutor};
use soul_core::tool::ToolRegistry;
use soul_core::vexec::VirtualExecutor;
use soul_core::vfs::VirtualFs;

use crate::tools::{
    bash::BashTool, edit::EditTool, find::FindTool, grep::GrepTool, ls::LsTool, read::ReadTool,
    write::WriteTool,
};

/// Create coding tools: read, write, edit, bash.
/// Full modification access for interactive coding sessions.
pub fn coding_tools(
    fs: Arc<dyn VirtualFs>,
    executor: Arc<dyn VirtualExecutor>,
    cwd: impl Into<String>,
) -> ToolRegistry {
    let cwd = cwd.into();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(WriteTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(EditTool::new(fs, &cwd)));
    registry.register(Box::new(BashTool::new(executor, &cwd)));
    registry
}

/// Create read-only tools: read, grep, find, ls.
/// Exploration without modification access.
pub fn read_only_tools(
    fs: Arc<dyn VirtualFs>,
    cwd: impl Into<String>,
) -> ToolRegistry {
    let cwd = cwd.into();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(GrepTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(FindTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(LsTool::new(fs, &cwd)));
    registry
}

/// Create all tools: read, write, edit, bash, grep, find, ls.
/// Complete toolkit for full agent capabilities.
pub fn all_tools(
    fs: Arc<dyn VirtualFs>,
    executor: Arc<dyn VirtualExecutor>,
    cwd: impl Into<String>,
) -> ToolRegistry {
    let cwd = cwd.into();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(WriteTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(EditTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(BashTool::new(executor, &cwd)));
    registry.register(Box::new(GrepTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(FindTool::new(fs.clone(), &cwd)));
    registry.register(Box::new(LsTool::new(fs, &cwd)));
    registry
}

/// Create an [`ExecutorRegistry`] with all coding tools wired via [`DirectExecutor`].
///
/// This integrates soul-coder tools into soul-core's config-driven executor system,
/// enabling routing alongside other executor backends (shell, HTTP, MCP, LLM).
///
/// ```rust
/// use std::sync::Arc;
/// use soul_core::vfs::MemoryFs;
/// use soul_core::vexec::NoopExecutor;
///
/// let fs = Arc::new(MemoryFs::new());
/// let exec = Arc::new(NoopExecutor);
/// let registry = soul_coder::presets::all_executor(fs, exec, "/workspace");
///
/// assert!(registry.has_tool("read"));
/// assert!(registry.has_tool("bash"));
/// ```
pub fn all_executor(
    fs: Arc<dyn VirtualFs>,
    executor: Arc<dyn VirtualExecutor>,
    cwd: impl Into<String>,
) -> ExecutorRegistry {
    let tools = all_tools(fs, executor, cwd);
    wrap_as_executor(tools)
}

/// Create an [`ExecutorRegistry`] with coding tools (read, write, edit, bash)
/// wired via [`DirectExecutor`].
pub fn coding_executor(
    fs: Arc<dyn VirtualFs>,
    executor: Arc<dyn VirtualExecutor>,
    cwd: impl Into<String>,
) -> ExecutorRegistry {
    let tools = coding_tools(fs, executor, cwd);
    wrap_as_executor(tools)
}

/// Wrap any [`ToolRegistry`] into an [`ExecutorRegistry`] with [`DirectExecutor`]
/// as the fallback, and all tool definitions registered as [`ConfigTool`] entries.
pub fn wrap_as_executor(tools: ToolRegistry) -> ExecutorRegistry {
    let definitions = tools.definitions();
    let direct = Arc::new(DirectExecutor::new(Arc::new(tools)));

    let mut registry = ExecutorRegistry::new();
    registry.register_executor(direct.clone() as Arc<dyn ToolExecutor>);

    for def in definitions {
        registry.register_config_tool(ConfigTool {
            definition: def,
            executor_name: "direct".into(),
            executor_config: serde_json::json!({}),
        });
    }

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use soul_core::vexec::NoopExecutor;
    use soul_core::vfs::MemoryFs;

    #[test]
    fn coding_tools_has_four() {
        let fs = Arc::new(MemoryFs::new());
        let exec = Arc::new(NoopExecutor);
        let registry = coding_tools(fs, exec, "/");
        assert_eq!(registry.len(), 4);
        assert!(registry.get("read").is_some());
        assert!(registry.get("write").is_some());
        assert!(registry.get("edit").is_some());
        assert!(registry.get("bash").is_some());
    }

    #[test]
    fn read_only_tools_has_four() {
        let fs = Arc::new(MemoryFs::new());
        let registry = read_only_tools(fs, "/");
        assert_eq!(registry.len(), 4);
        assert!(registry.get("read").is_some());
        assert!(registry.get("grep").is_some());
        assert!(registry.get("find").is_some());
        assert!(registry.get("ls").is_some());
    }

    #[test]
    fn all_tools_has_seven() {
        let fs = Arc::new(MemoryFs::new());
        let exec = Arc::new(NoopExecutor);
        let registry = all_tools(fs, exec, "/");
        assert_eq!(registry.len(), 7);
        let names = registry.names();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"write"));
        assert!(names.contains(&"edit"));
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"find"));
        assert!(names.contains(&"ls"));
    }

    #[test]
    fn definitions_all_have_schemas() {
        let fs = Arc::new(MemoryFs::new());
        let exec = Arc::new(NoopExecutor);
        let registry = all_tools(fs, exec, "/");
        for def in registry.definitions() {
            assert!(!def.name.is_empty());
            assert!(!def.description.is_empty());
            assert!(def.input_schema.is_object());
        }
    }

    #[test]
    fn all_executor_has_tools() {
        let fs = Arc::new(MemoryFs::new());
        let exec = Arc::new(NoopExecutor);
        let registry = all_executor(fs, exec, "/");
        assert!(registry.has_tool("read"));
        assert!(registry.has_tool("write"));
        assert!(registry.has_tool("edit"));
        assert!(registry.has_tool("bash"));
        assert!(registry.has_tool("grep"));
        assert!(registry.has_tool("find"));
        assert!(registry.has_tool("ls"));
        assert_eq!(registry.definitions().len(), 7);
    }

    #[test]
    fn coding_executor_has_four() {
        let fs = Arc::new(MemoryFs::new());
        let exec = Arc::new(NoopExecutor);
        let registry = coding_executor(fs, exec, "/");
        assert!(registry.has_tool("read"));
        assert!(registry.has_tool("bash"));
        assert_eq!(registry.definitions().len(), 4);
    }

    #[tokio::test]
    async fn executor_registry_routes_correctly() {
        let fs = Arc::new(MemoryFs::new());
        fs.write("/test.txt", "hello").await.unwrap();
        let exec = Arc::new(NoopExecutor);
        let registry = all_executor(fs, exec, "/");

        let result = registry
            .execute("read", "c1", serde_json::json!({"path": "/test.txt"}), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello"));
    }
}
