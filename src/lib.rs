//! # soul-coder
//!
//! Coding-specific tools for [soul-core](https://crates.io/crates/soul-core) —
//! read, write, edit, bash, grep, find, ls.
//!
//! WASM-first, cross-platform. All tools use `soul_core::vfs::VirtualFs` and
//! `soul_core::vexec::VirtualExecutor` for platform abstraction, enabling
//! full operation in both native and WebAssembly environments.
//!
//! ## Quick Start
//!
//! ```rust
//! use std::sync::Arc;
//! use soul_core::vfs::MemoryFs;
//! use soul_core::vexec::NoopExecutor;
//! use soul_coder::presets;
//!
//! // Create all 7 coding tools with in-memory VFS (WASM-ready)
//! let fs = Arc::new(MemoryFs::new());
//! let exec = Arc::new(NoopExecutor);
//! let registry = presets::all_tools(fs, exec, "/workspace");
//!
//! assert_eq!(registry.len(), 7);
//! ```
//!
//! ## Tool Presets
//!
//! | Preset | Tools | Use Case |
//! |--------|-------|----------|
//! | `coding_tools` | read, write, edit, bash | Interactive coding sessions |
//! | `read_only_tools` | read, grep, find, ls | Codebase exploration |
//! | `all_tools` | all 7 tools | Full agent capabilities |
//!
//! ## Individual Tools
//!
//! Each tool can be instantiated independently:
//!
//! ```rust
//! use std::sync::Arc;
//! use soul_core::vfs::MemoryFs;
//! use soul_coder::tools::read::ReadTool;
//!
//! let fs = Arc::new(MemoryFs::new());
//! let tool = ReadTool::new(fs, "/workspace");
//! ```

pub mod presets;
pub mod tools;
pub mod truncate;

// Re-export key types for convenience
pub use presets::{all_tools, coding_tools, read_only_tools};
pub use tools::{
    bash::BashTool,
    edit::EditTool,
    find::FindTool,
    grep::GrepTool,
    ls::LsTool,
    read::ReadTool,
    write::WriteTool,
};
