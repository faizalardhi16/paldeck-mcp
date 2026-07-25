# Paldeck MCP — Single-Binary Codebase Indexer + MCP Server

**Index your codebase into a queryable knowledge graph. Expose it as MCP tools any AI agent can call.** Like [CBM](https://github.com/DeusData/codebase-memory-mcp), but 9MB, hackable, and yours.

## Why

AI coding tools don't know which file to touch. Paldeck MCP indexes your codebase into a SQLite knowledge graph and serves it over MCP — so your agent instantly knows where every function, class, and call edge lives.

**Single binary. Zero dependencies.** Download `paldeck-mcp.exe`, put it on PATH, done.

## Languages Supported

| Language | Extensions | Symbols Extracted |
|----------|-----------|-------------------|
| Python | `.py` | functions, classes, methods, imports, calls |
| TypeScript/JS | `.ts` `.tsx` `.js` `.jsx` | functions, classes, methods, imports, calls |
| Rust | `.rs` | functions, structs, traits, impls, calls |
| Go | `.go` | functions, methods, structs, interfaces, imports, calls |

## Quick Start

### 1. Download

**Windows:**
```powershell
mkdir "$env:USERPROFILE\tools" -Force
Invoke-WebRequest -Uri "https://github.com/faizalardhi16/paldeck-mcp/releases/download/v0.2.0/paldeck-mcp.exe" -OutFile "$env:USERPROFILE\tools\paldeck-mcp.exe"
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:USERPROFILE\tools", "User")
```

**Linux/macOS (build from source):**
```bash
git clone https://github.com/faizalardhi16/paldeck-mcp.git
cd paldeck-mcp
cargo build --release
sudo ln -s $(pwd)/target/release/paldeck-mcp /usr/local/bin/
```

### 2. Index your project

```bash
paldeck-mcp index --project /path/to/your/project
# → index.db created in project root
```

### 3. Start the MCP server

```bash
paldeck-mcp serve
# → MCP server running on stdio — connect your agent
```

### 4. Connect your agent

Drop this into your MCP config:

```json
{"mcpServers": {"paldeck": {"command": "paldeck-mcp", "args": ["serve"]}}}
```

Works with **Cursor**, **Codex CLI**, **Claude Code**, and **Hermes Agent**. See [`INTEGRATION.md`](INTEGRATION.md) for platform-specific configs.

## MCP Tools (7)

| Tool | Description |
|------|-------------|
| `search_symbols` | Find functions/classes by name (exact + fuzzy) |
| `get_symbol` | Full info for a symbol: signature, docstring, location |
| `find_definition` | Jump to a symbol's definition |
| `find_references` | Every call site that references this symbol |
| `find_callers` | Who calls this function? (recursive depth supported) |
| `list_files` | All source files discovered during indexing |
| `get_file_summary` | All symbols in a file, grouped by kind |

## Architecture

```
paldeck-mcp
├── paldeck-mcp index --project /repo   → tree-sitter AST parsing → index.db (SQLite)
└── paldeck-mcp serve                    → reads index.db → MCP stdio server
```

Single Rust binary. Tree-sitter handles parsing (Python, TypeScript, Rust, Go grammars bundled). `petgraph` builds the in-memory call graph. `rusqlite` persists it. `rmcp` serves the MCP protocol over stdio.

## Comparison: Paldeck vs CBM

| | CBM | Paldeck MCP |
|---|---|---|
| Binary size | 100-150MB (158 languages) | **9MB** (4 languages) |
| Runtime deps | Zero | **Zero** |
| Single binary? | ✅ | ✅ |
| Languages | 158 | 4 (add more via tree-sitter grammars) |
| Codebase | C (hard to extend) | **Rust** (tree-sitter native, ergonomic) |
| MCP tools | 15 (ADR, cross-service, etc.) | **7 focused** (call graph core) |
| Maintainer risk | Solo volunteer | **Your code, your team** |

## Extending

### Add a language
1. Add tree-sitter grammar to `Cargo.toml`
2. Write a `parse_<lang>()` function in `src/parser.rs` using S-expression queries
3. Add the extension to `SUPPORTED_EXTENSIONS` in `src/walker.rs`

### Add an MCP tool
1. Define the tool schema in `src/server.rs`
2. Implement the handler calling into the query engine
3. Rebuild — `cargo build --release`

No Python. No separate server. Everything lives in one binary.

## License

MIT
