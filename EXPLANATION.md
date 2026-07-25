# Codebase MCP — Hybrid Architecture

**Why this exists:** AI coding tools don't know which file to touch. CBM solves this by indexing the codebase into a queryable knowledge graph. But CBM is a single-maintainer C binary — hard to extend, hard to customize. This is a rebuild from first principles, splitting the problem into two layers where each language does what it does best.

---

## 1. Why Hybrid?

| Layer | Language | Because |
|-------|----------|---------|
| **Indexer** (AST parser + graph builder) | **Rust** | Tree-sitter is native Rust. Zero-cost abstractions. `petgraph` has Dijkstra, SCC, dominator trees built-in. Single static binary. |
| **MCP Server** (protocol + query layer) | **Python** | `mcp` package is mature. NetworkX optional for advanced graph algorithms. Iterate on tool definitions in minutes, not days. |

The **contract** between them is **SQLite** — the Rust indexer writes it, the Python server reads it. No RPC, no serialization overhead. Just open the same file.

### What each side owns

```
┌─────────────────────────┐      ┌──────────────────────────────┐
│     RUST INDEXER         │      │      PYTHON MCP SERVER       │
│─────────────────────────│      │──────────────────────────────│
│ • File discovery         │      │ • MCP protocol (JSON-RPC)    │
│ • Tree-sitter parsing    │  ┌───│ • Tool definitions           │
│ • Symbol extraction      │  │   │ • Query dispatch             │
│ • Call-graph edges       │  │   │ • Recursive CTE traversal   │
│ • petgraph in-memory     │  │   │ • Optional NetworkX graph    │
│ • SQLite persistence     │──┘   │ • reindex trigger (subprocess)│
│ • Single static binary   │      │ • Configurable per-project   │
└─────────────────────────┘      └──────────────────────────────┘
              │                              │
              │        index.db              │
              └──────────┬──────────────────┘
                         │
                    Shared SQLite
                    (the contract)
```

---

## 2. The SQLite Contract

This is the single source of truth. The schema is intentionally simple — three tables.

### `symbols`
```sql
CREATE TABLE symbols (
    id          TEXT PRIMARY KEY,   -- "src/main.rs::calculate_total"
    name        TEXT NOT NULL,      -- "calculate_total"
    kind        TEXT NOT NULL,      -- function | class | method | struct | interface | enum | route
    file_path   TEXT NOT NULL,      -- "src/main.rs"
    line_start  INTEGER NOT NULL,   -- 1-indexed
    line_end    INTEGER NOT NULL,
    signature   TEXT,               -- "fn calculate_total(items: &[Item]) -> f64"
    docstring   TEXT,               -- extracted docstring / JSDoc / /// comment
    properties  TEXT DEFAULT 'null' -- JSON: {"route": "/api/items", "method": "GET", "decorators": ["auth_required"]}
);
```

The `id` format is `{relative_path}::{symbol_name}`. This is the stable reference used everywhere.

### `edges`
```sql
CREATE TABLE edges (
    source_id   TEXT NOT NULL REFERENCES symbols(id),
    target_id   TEXT NOT NULL REFERENCES symbols(id),
    kind        TEXT NOT NULL,       -- calls | imports | inherits | defines
    PRIMARY KEY (source_id, target_id, kind)
);
```

Edge directions:
- `calls`: `function_a` → `function_b` (function_a memanggil function_b)
- `imports`: `file_a.rs` → `std::collections::HashMap` (atau module path)
- `inherits`: `child_class` → `parent_class`
- `defines`: `class` → `method` (class mendefinisikan method)

### `meta`
```sql
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);
-- Keys: schema_version, project_root, indexed_at, file_count, symbol_count
```

### Indexes (auto-created)
```sql
CREATE INDEX idx_symbols_name   ON symbols(name);
CREATE INDEX idx_symbols_kind   ON symbols(kind);
CREATE INDEX idx_symbols_file   ON symbols(file_path);
CREATE INDEX idx_edges_source   ON edges(source_id);
CREATE INDEX idx_edges_target   ON edges(target_id);
```

---

## 3. Rust Indexer — Code Walkthrough

### 3.1 Entry Point (`main.rs`)

The pipeline runs in order every time:

```
1. walker::discover_source_files()  →  Vec<PathBuf>
2. parser::parse_file() × N         →  (Vec<Symbol>, Vec<Edge>) per file
3. graph::CodeGraph::build()        →  petgraph::DiGraph (resolve partial edges)
4. storage::persist()               →  SQLite on disk
```

```
$ codebase-indexer --project /path/to/repo [--output custom.db] [-v]
🔍 Walking project: /path/to/repo
   Found 342 source files
   src/main.rs → 8 symbols, 12 edges
   src/parser.rs → 15 symbols, 23 edges
   ...
📊 Extracted 1847 symbols, 3210 edges
🌐 Graph built: 1847 nodes
💾 Persisted to /path/to/repo/index.db
```

### 3.2 File Walker (`walker.rs`)

Uses the `ignore` crate — same engine as ripgrep. This means:

- `.gitignore` is **automatically respected**. No manual skip-list for generated files.
- Common noise directories (`node_modules`, `__pycache__`, `target`, `venv`, `.mypy_cache`, etc.) are skipped even without `.gitignore` entries.
- Supported extensions: `.py`, `.ts`, `.tsx`, `.rs`, `.js`, `.jsx`, `.go` (tambah sendiri di `SUPPORTED_EXTENSIONS`).

```rust
// walker.rs — key logic
for entry in ignore::Walk::new(root) {
    if ext is in SUPPORTED_EXTENSIONS → collect
    if dir is in SKIP_DIRS           → entry.ignore_visit()  // skip entire subtree
}
```

### 3.3 AST Parser (`parser.rs`)

This is the heaviest file. Three language backends:

| Language | Tree-sitter Grammar | Key Query Patterns |
|----------|---------------------|--------------------|
| Python | `tree-sitter-python` | `function_definition`, `class_definition`, `call`, `import_statement` |
| TypeScript | `tree-sitter-typescript` | `function_declaration`, `class_declaration`, `method_definition`, `call_expression`, `import_statement` |
| Rust | `tree-sitter-rust` | `function_item`, `struct_item`, `call_expression` |
| Go | `tree-sitter-go` | `function_declaration`, `method_declaration`, `type_spec` (struct), `interface_type`, `call_expression`, `import_declaration` |

**Tree-sitter queries use S-expression syntax** — think of them as structural regex:

```scheme
;; Python function extraction
(function_definition
    name: (identifier) @name        ;; capture function name as @name
    parameters: (parameters) @params ;; capture parameters as @params
    body: (block) @body) @func       ;; capture the whole thing as @func
```

Each file goes through this flow:

```
Source text
  → Parser::new(language).parse(source)
    → tree.root_node()
      → QueryCursor::matches(query, root, bytes)
        → for each match: extract captures → build Symbol + Edge
```

**Call graph extraction** happens in a second pass inside each function/method body:

```scheme
;; Find all function calls inside a function body
(call function: (identifier) @called)
```

The extracted edges at this stage are **partial** — the `target` field might be just a name (`"calculate_total"`) instead of a full ID (`"src/main.rs::calculate_total"`). Resolution happens in the graph builder.

**Class method extraction** is recursive — parse the class body, find `function_definition` children, mark them as `kind: method`, and add `defines` edges from class → method.

### 3.4 Graph Builder (`graph.rs`)

Wraps `petgraph::DiGraph` and handles edge resolution:

```rust
pub fn build(symbols: &[Symbol], raw_edges: &[Edge]) -> CodeGraph {
    // 1. Insert all symbols as nodes
    // 2. Build name_index: HashMap<id, NodeIndex>
    // 3. Build name_to_ids: HashMap<name, Vec<id>>
    // 4. For each raw edge:
    //    - Try direct ID match in name_index → add edge
    //    - If not found, try name match via name_to_ids → add edge
    //    - If still not found → skip (cross-module, could be external)
}
```

This resolution step is crucial because function calls within the same file only have the local name. The `name_to_ids` index bridges "name" to "full ID" so intra-file calls are resolved.

### 3.5 SQLite Persistence (`storage.rs`)

Straightforward batch insert:

```rust
// 1. Delete old DB (full reindex — fast enough for most repos)
// 2. PRAGMA journal_mode=WAL, synchronous=NORMAL
// 3. CREATE TABLE symbols, edges, meta
// 4. Batch INSERT symbols (prepared statement)
// 5. Batch INSERT edges (prepared statement, INSERT OR IGNORE for dedup)
// 6. INSERT meta (schema_version, project_root, indexed_at, file_count, symbol_count)
```

---

## 4. Python MCP Server — Code Walkthrough

### 4.1 Server Entry (`server.py`)

Implements the MCP protocol via the `mcp` Python package. Exposes **7 tools**:

| Tool | Input | Output | SQL |
|------|-------|--------|-----|
| `search_symbol` | `query`, `kind?`, `fuzzy?` | List of matching symbols | `WHERE name = ?` or `LIKE %?%` |
| `trace_callers` | `symbol_id`, `depth?` | Symbols that call this one | `JOIN edges WHERE target_id = ?` (+ recursive CTE for depth>1) |
| `trace_callees` | `symbol_id`, `depth?` | Symbols called by this one | `JOIN edges WHERE source_id = ?` (+ recursive CTE) |
| `get_architecture` | `path` | Symbols grouped by kind, within path scope | `WHERE file_path LIKE 'path%'` |
| `list_symbols_in_file` | `file_path` | All symbols in one file, by line order | `WHERE file_path = ? ORDER BY line_start` |
| `index_status` | — | Meta stats | `SELECT * FROM meta` + aggregate counts |
| `reindex` | — | Re-run Rust indexer as subprocess | `asyncio.create_subprocess_exec` |

### 4.2 Query Engine (`QueryEngine` class)

Pure SQL. No ORM. The only "advanced" SQL feature is **recursive CTE** for multi-depth tracing:

```sql
-- trace_callers with depth=3
WITH RECURSIVE callers(id, name, kind, file_path, line_start, signature, depth) AS (
    -- Base: direct callers
    SELECT s.*, 1 FROM edges e JOIN symbols s ON s.id = e.source_id
    WHERE e.target_id = ? AND e.kind = 'calls'
    UNION ALL
    -- Recursive: callers of callers
    SELECT s.*, c.depth + 1
    FROM edges e JOIN symbols s ON s.id = e.source_id
    JOIN callers c ON e.target_id = c.id
    WHERE e.kind = 'calls' AND c.depth < 3
)
SELECT DISTINCT * FROM callers ORDER BY depth, name
```

This is the same algorithm CBM's `trace_path` uses, but expressed in declarative SQL. No custom query language needed.

### 4.3 Optional: NetworkX Enricher (`query.py`)

If `pip install networkx`, you get:

| Query | NetworkX call | Use case |
|-------|--------------|----------|
| Shortest call path | `nx.shortest_path(G, A, B)` | "How does A reach B?" |
| Cycle detection | `nx.simple_cycles(G)` | Find circular dependencies |
| BFS depth map | `nx.single_source_shortest_path_length(G, A)` | Full call chain depth |
| Top callers | `sorted(G.in_degree(), ...)` | Most-called functions ("hot paths") |

```python
# Usage
from query import load_graph
enricher = load_graph(conn)          # loads SQLite → NetworkX DiGraph
path = enricher.shortest_call_path("a.rs::foo", "b.rs::bar")
# → ["a.rs::foo", "c.rs::helper", "b.rs::bar"]
```

---

## 5. How Hermes Discovers It

Add to `~/.hermes/config.yaml`:

```yaml
mcp_servers:
  codebase:
    command: "uv"
    args: ["run", "--directory", "/root/codebase-mcp-hybrid/server", "codebase-mcp"]
    timeout: 30
```

Or if you symlink the entry point:

```yaml
mcp_servers:
  codebase:
    command: "codebase-mcp"
    args: []
```

On Hermes restart, it will:
1. Spawn the Python MCP server
2. Call `list_tools()` → discovers the 7 tools
3. Register them as `mcp_codebase_search_symbol`, `mcp_codebase_trace_callers`, etc.
4. Available in every conversation

---

## 6. Build & Run

### Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Python MCP SDK
pip install mcp

# Optional: advanced graph queries
pip install networkx
```

### Build the indexer

```bash
cd indexer
cargo build --release

# Output: target/release/codebase-indexer (~15-30MB single binary)

# Optional: install to PATH
sudo ln -s $(pwd)/target/release/codebase-indexer /usr/local/bin/
```

### Index a project

```bash
codebase-indexer --project /path/to/your/repo

# With verbose output
codebase-indexer --project /path/to/your/repo -vv
```

### Run the MCP server manually (for testing)

```bash
cd server
uv run python -m server

# Or with explicit project root
PROJECT_ROOT=/path/to/repo uv run python -m server
```

### Integration test

```bash
# 1. Index a project
codebase-indexer --project ./test_project

# 2. Start the MCP server in stdio mode
# 3. Send a JSON-RPC tools/list request via stdin:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | codebase-mcp
```

---

## 7. Full Flow — End to End

### Indexing (happens once, or on reindex)

```
User: "reindex"

Python MCP server
  → asyncio.create_subprocess_exec("codebase-indexer", "--project", repo)
    → Rust indexer
      → walker.rs: discover 342 .py/.ts/.rs files
      → parser.rs: tree-sitter parse × 342
        → Python: extract function/class/method + call edges
        → TypeScript: extract function/class/method + call edges
        → Rust: extract fn/struct + call edges
      → graph.rs: resolve partial edges → full petgraph DiGraph
      → storage.rs: INSERT symbols + edges + meta → index.db
    → returns stdout to Python
  → Python reloads QueryEngine from new index.db
  → Returns "Indexed 1847 symbols, 3210 edges"
```

### Query (happens every time an AI agent needs to understand the code)

```
AI Agent: "Where is the email validation logic?"

→ calls mcp_codebase_search_symbol(query="validat*", fuzzy=true, kind="function")
  → Python: SELECT * FROM symbols WHERE name LIKE '%validat%' AND kind='function'
  → Returns:
    [
      {"id": "src/validators.py::validate_email", "name": "validate_email",
       "file_path": "src/validators.py", "line_start": 45, ...},
      {"id": "src/user_service.py::validate_user_email", ...}
    ]

AI Agent: "Who calls validate_email?"

→ calls mcp_codebase_trace_callers(symbol_id="src/validators.py::validate_email")
  → Python: SELECT s.* FROM edges e JOIN symbols s WHERE e.target_id=? AND kind='calls'
  → Returns:
    [
      {"id": "src/user_service.py::create_user", "name": "create_user", ...},
      {"id": "src/api/handlers.py::register_endpoint", "name": "register_endpoint", ...}
    ]

AI Agent now knows EXACTLY which 2 files to modify.
```

---

## 8. Extending

### Adding a new language

1. Add tree-sitter grammar to `Cargo.toml`:
   ```toml
   tree-sitter-go = "0.22"
   ```

2. Add parse function in `parser.rs`:
   ```rust
   "go" => parse_go(relative_path, source),
   ```

3. Write tree-sitter queries for that language's function/class/call/import patterns.

4. Add extension to `SUPPORTED_EXTENSIONS` in `walker.rs`.

### Adding a new MCP tool

1. Add `Tool(...)` definition in `server.py → list_tools()`
2. Add handler branch in `server.py → call_tool()`
3. Optionally add a query method in `QueryEngine`

No Rust changes needed for query-side tools.

### Adding embeddings / semantic search (v2)

Rust side:
```toml
candle = "0.7"
candle-onnx = "0.7"
```
Bundle `nomic-embed-code-v1.onnx` → generate embeddings during indexing → store in SQLite `embeddings` table.

Python side:
```python
# Read embeddings from SQLite
# Cosine similarity search
# Expose as search_symbol(semantic_query="...")
```

---

## 9. Comparison: This vs CBM

| | CBM | This (Hybrid) |
|---|---|---|
| Binary | 100-150MB (158 languages) | 15-30MB (4 languages: Python, TS, Rust, Go) |
| Code | C — hard to contribute | Rust + Python — each does what it's good at |
| Extensibility | Fork the C codebase | Add Rust grammar or Python tool |
| Semantic search | Bundled ONNX (nomic-embed-code) | Optional v2, delegate to Ollama or skip |
| MCP tools | 15 (scope creep: ADR, cross-service, etc) | 7 focused tools, add as needed |
| Maintainer risk | Solo volunteer | Your own code, your team |
| SQLite schema | Complex — Cypher-like graph model | Simple 3 tables, plain SQL |
| Call tracing | Custom `trace_path` algorithm | SQL recursive CTE (standard, portable) |

---

## 10. Directory Structure

```
codebase-mcp-hybrid/
├── indexer/                        # Rust crate
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                 # CLI orchestrator
│       ├── walker.rs               # File discovery (gitignore-aware)
│       ├── parser.rs               # Tree-sitter: 3 languages, symbol + edge extraction
│       ├── graph.rs                # petgraph wrapper + edge resolution
│       └── storage.rs              # SQLite persistence
├── server/                         # Python package
│   ├── pyproject.toml
│   └── src/
│       ├── __init__.py
│       ├── server.py               # MCP protocol + 7 tools + QueryEngine
│       └── query.py                # Optional NetworkX graph enrichment
├── tests/
│   └── test_e2e.py                 # (future) end-to-end integration tests
├── EXPLANATION.md                  # ← this file
└── README.md
```

---

## Key Design Decisions

1. **Full reindex, not incremental (for now).** Tree-sitter parsing is sub-second per file. Reindexing 342 files takes <2 seconds. Incremental indexing adds complexity (change detection, partial graph updates) that's not worth it until >10K files.

2. **SQLite, not Postgres.** Zero setup. File-based. WAL mode gives concurrent reads (Python MCP server can read while a reindex is writing).

3. **No custom query language.** CBM has Cypher. This uses plain SQL — any developer can read the queries, any BI tool can connect to the DB.

4. **Prepared statements for batch inserts.** SQLite in Rust with `rusqlite::Connection::prepare` + `.execute()` is 10x faster than individual inserts.

5. **MCP server is stateless per query.** It opens the DB, runs the query, returns results. No persistent server state. Reindex? Close, reindex, reopen. Simple.
