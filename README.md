# soul-coder

Coding-specific tools for [soul-core](https://crates.io/crates/soul-core) — read, write, edit, bash, grep, find, ls.

WASM-first, cross-platform. All tools use `soul_core::vfs::VirtualFs` and `soul_core::vexec::VirtualExecutor` for platform abstraction, enabling full operation in native, WebAssembly, and sandboxed environments.

## Install

```toml
[dependencies]
soul-coder = "0.1"
```

For WASM targets:

```toml
[dependencies]
soul-coder = { version = "0.1", default-features = false, features = ["wasm"] }
```

## Quick Start

```rust
use std::sync::Arc;
use soul_core::vfs::MemoryFs;
use soul_core::vexec::NoopExecutor;
use soul_coder::presets;

// In-memory VFS — works everywhere including WASM
let fs = Arc::new(MemoryFs::new());
let exec = Arc::new(NoopExecutor);
let registry = presets::all_tools(fs, exec, "/workspace");

assert_eq!(registry.len(), 7);
```

## Tools

| Tool | Description |
|------|-------------|
| **read** | Read file contents with line numbers, offset/limit pagination, auto-truncation |
| **write** | Create or overwrite files, auto-creates parent directories |
| **edit** | Exact text replacement with fuzzy fallback (smart quotes, unicode dashes, trailing whitespace). Outputs unified diff |
| **bash** | Execute shell commands with ANSI stripping, tail truncation, configurable timeout |
| **grep** | Search file contents by pattern with glob filtering, context lines, match limits |
| **find** | Find files by glob pattern with recursive directory traversal |
| **ls** | List directory contents with case-insensitive sort and directory suffixes |

Every tool implements `soul_core::tool::Tool` and plugs directly into soul-core's `ToolRegistry` and `AgentLoop`.

## Presets

Three factory functions for common use cases:

```rust
use std::sync::Arc;
use soul_core::vfs::MemoryFs;
use soul_core::vexec::NoopExecutor;

let fs = Arc::new(MemoryFs::new());
let exec = Arc::new(NoopExecutor);

// Interactive coding sessions: read, write, edit, bash
let coding = soul_coder::coding_tools(fs.clone(), exec.clone(), "/workspace");

// Codebase exploration: read, grep, find, ls
let readonly = soul_coder::read_only_tools(fs.clone(), "/workspace");

// Everything: all 7 tools
let all = soul_coder::all_tools(fs, exec, "/workspace");
```

## Individual Tools

Each tool can be instantiated independently:

```rust
use std::sync::Arc;
use soul_core::vfs::MemoryFs;
use soul_core::tool::Tool;
use soul_coder::ReadTool;

let fs = Arc::new(MemoryFs::new());
let tool = ReadTool::new(fs, "/workspace");

assert_eq!(tool.name(), "read");
```

## Architecture

```
soul-coder
├── tools/
│   ├── read.rs      VirtualFs → line-numbered output with truncation
│   ├── write.rs     VirtualFs → create/overwrite with parent dir creation
│   ├── edit.rs      VirtualFs → exact + fuzzy match, unified diff output
│   ├── bash.rs      VirtualExecutor → shell execution with ANSI stripping
│   ├── grep.rs      VirtualFs → recursive content search with glob filter
│   ├── find.rs      VirtualFs → recursive file search by glob pattern
│   └── ls.rs        VirtualFs → sorted directory listing
├── truncate.rs      Unified truncation (head/tail, line/byte limits)
├── presets.rs        coding_tools, read_only_tools, all_tools
└── lib.rs           Public API and re-exports
```

### Platform Abstraction

All file I/O goes through `VirtualFs`. All command execution goes through `VirtualExecutor`. No direct `std::fs` or `std::process` calls anywhere.

| Environment | VirtualFs | VirtualExecutor |
|------------|-----------|-----------------|
| WASM / Browser | `MemoryFs` | `NoopExecutor` |
| Tests | `MemoryFs` | `MockExecutor` |
| Native | `NativeFs` | `NativeExecutor` |

### Truncation

Unified truncation system across all tools:

- **Head truncation** (file reads): keep first N lines/bytes — beginning of file matters
- **Tail truncation** (bash output): keep last N lines/bytes — errors and final output matter
- Constants: `MAX_LINES=2000`, `MAX_BYTES=50KB`, `GREP_MAX_LINE_LENGTH=500`

### Edit Tool: Fuzzy Matching

When exact match fails, the edit tool normalizes both the search text and file content:

- Smart quotes → ASCII (`'` `'` → `'`, `"` `"` → `"`)
- Unicode dashes → ASCII (`–` `—` → `-`)
- Non-breaking spaces → regular spaces
- Trailing whitespace stripped per line

This handles the formatting variations that LLMs naturally introduce when reproducing code.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `native` | yes | Enables `NativeFs`, `NativeExecutor`, full tokio |
| `wasm` | no | Enables WASM-compatible dependencies |

## License

MIT
