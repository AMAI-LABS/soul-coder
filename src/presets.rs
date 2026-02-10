//! Preset tool collections for common use cases.

use std::sync::Arc;

use soul_core::tool::ToolRegistry;
use soul_core::vexec::VirtualExecutor;
use soul_core::vfs::VirtualFs;

use crate::tools::{bash::BashTool, edit::EditTool, find::FindTool, grep::GrepTool, ls::LsTool, read::ReadTool, write::WriteTool};

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
}
