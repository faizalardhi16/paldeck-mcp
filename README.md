# Codebase MCP — Hybrid Rust+Python

**4-language codebase indexer as MCP server.** Like [CBM](https://github.com/DeusData/codebase-memory-mcp), but hackable.

## Why?

AI coding tools don't know which file to touch. This indexes your codebase into a queryable knowledge graph, then exposes it as MCP tools that any agent (Cursor, Codex, Claude Code, Hermes) can call.

Unlike CBM (single-maintainer C binary, 100MB+), this splits the problem:
- **Rust** indexer: single 7MB binary, tree-sitter AST parsing, zero dependencies
- **Python** MCP server: fast iteration, 7 focused tools, optional NetworkX enrichment

## Languages Supported

| Language | Extensions | Symbols Extracted |
|----------|-----------|-------------------|
| Python | `.py` | functions, classes, methods, imports, calls |
| TypeScript/JS | `.ts` `.tsx` `.js` `.jsx` | functions, classes, methods, imports, calls |
| Rust | `.rs` | functions, structs, calls |
| Go | `.go` | functions, methods, structs, interfaces, imports, calls |

## Quick Start

### 1. Build the indexer (requires Rust)
```bash
cd indexer
cargo build --release
# Output: target/release/codebase-indexer (~7MB single binary)
```

### 2. Index your project
```bash
codebase-indexer --project /path/to/your/project
# Creates: /path/to/your/project/index.db
```

### 3. Run the MCP server
```bash
cd server
pip install mcp
python -m server
```

### 4. Connect to your agent

**Cursor** (`~/.cursor/mcp.json`):
```json
{ "mcpServers": { "codebase": { "command": "python", "args": ["-m", "server"], "cwd": "/path/to/server/src", "env": { "PROJECT_ROOT": "/path/to/project" } } } }
```

**Codex CLI** (`~/.codex/config.toml`):
```toml
[mcp_servers.codebase]
command = "python"
args = ["-m", "server"]
cwd = "/path/to/server/src"
env = { PROJECT_ROOT = "/path/to/project" }
```

**Claude Code** (`~/.claude/mcp.json`):
```json
{ "mcpServers": { "codebase": { "command": "python", "args": ["-m", "server"], "cwd": "/path/to/server/src", "env": { "PROJECT_ROOT": "/path/to/project" } } } }
```

**Hermes** (`~/.hermes/config.yaml`):
```yaml
mcp_servers:
  codebase:
    command: "python"
    args: ["-m", "server"]
    cwd: "/path/to/server/src"
    env:
      PROJECT_ROOT: "/path/to/project"
```

## MCP Tools

| Tool | What it does |
|------|-------------|
| `search_symbol` | Find functions/classes by name (exact or fuzzy) |
| `trace_callers` | Who calls this function? (inbound call graph, recursive depth supported) |
| `trace_callees` | What does this function call? (outbound call graph) |
| `get_architecture` | All symbols in a module, grouped by kind |
| `list_symbols_in_file` | Every symbol in a file, ordered by line |
| `index_status` | Stats: file count, symbol count, last indexed |
| `reindex` | Re-run indexer to pick up code changes |

## Architecture

```
Rust indexer (AST parsing, petgraph, SQLite)  →  index.db  ←  Python MCP server (query, tools)
```

See `EXPLANATION.md` for the full architecture walkthrough.
See `INTEGRATION.md` for detailed agent setup.
See `WINDOWS.md` for Windows-specific instructions.

## Windows Users

Pre-built binary available in [Releases](https://github.com/faizalardhi16/codebase-mcp/releases).

Download `codebase-indexer.exe` (7MB, single file), then:
```powershell
codebase-indexer.exe --project C:\path\to\project
```

## License

MIT
