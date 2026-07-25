# Paldeck MCP — Architecture

## 1. Overview

Paldeck MCP is a **single Rust binary** that indexes codebases into a queryable SQLite knowledge graph and serves 7 MCP tools over stdio.

```
User runs: paldeck-mcp index --project /repo
  → walker.rs       discovers source files (gitignore-aware)
  → parser.rs       tree-sitter AST → symbols + edges (4 languages)
  → graph.rs        resolves partial edges → full call graph
  → storage.rs      persists graph → index.db (SQLite, WAL mode)

User runs: paldeck-mcp serve
  → server.rs       reads index.db, starts MCP stdio server (rmcp)
  → AI agent calls  search_symbols, find_callers, find_references…
```

## 2. The SQLite Schema

Three tables. Simple, portable, queryable with plain SQL.

### `symbols`

```sql
CREATE TABLE symbols (
    id          TEXT PRIMARY KEY,     -- "src/main.rs::calculate_total"
    name        TEXT NOT NULL,        -- "calculate_total"
    kind        TEXT NOT NULL,        -- function | class | method | struct | interface
    file_path   TEXT NOT NULL,        -- "src/main.rs"
    line_start  INTEGER NOT NULL,     -- 1-indexed
    line_end    INTEGER NOT NULL,
    signature   TEXT,                 -- "fn calculate_total(items: &[Item]) -> f64"
    docstring   TEXT,                 -- extracted docstring / JSDoc / /// comment
    properties  TEXT DEFAULT 'null'   -- JSON: {"route": "/api/items", "decorators": […]}
);
```

### `edges`

```sql
CREATE TABLE edges (
    source_id   TEXT NOT NULL REFERENCES symbols(id),
    target_id   TEXT NOT NULL REFERENCES symbols(id),
    kind        TEXT NOT NULL,        -- calls | imports | defines
    PRIMARY KEY (source_id, target_id, kind)
);
```

Edge directions:
- `calls`: `function_a` → `function_b` (A calls B)
- `imports`: file → external module
- `defines`: class → method (class defines method)

### `meta`

```sql
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);
-- Keys: schema_version, project_root, indexed_at, file_count, symbol_count
```

## 3. Module Walkthrough

### 3.1 File Walker (`walker.rs`)

Uses the `ignore` crate (same engine as ripgrep). This means `.gitignore` is **automatically respected** — no manual skip lists. Common noise directories (`node_modules`, `__pycache__`, `target`, `venv`, `.git`, etc.) are skipped even without `.gitignore` entries.

Supported extensions: `.py`, `.ts`, `.tsx`, `.js`, `.jsx`, `.rs`, `.go`.

### 3.2 AST Parser (`parser.rs`)

The heaviest module (~620 lines). Four language backends, each using tree-sitter S-expression queries:

| Language | Grammar Crate | Key Patterns |
|----------|--------------|--------------|
| Python | `tree-sitter-python` | `function_definition`, `class_definition`, `call`, `import_statement` |
| TypeScript/JS | `tree-sitter-typescript` | `function_declaration`, `class_declaration`, `method_definition`, `call_expression`, `import_statement` |
| Rust | `tree-sitter-rust` | `function_item`, `struct_item`, `trait_item`, `call_expression` |
| Go | `tree-sitter-go` | `function_declaration`, `method_declaration`, `type_spec`, `interface_type`, `call_expression`, `import_declaration` |

Each file flows through:

```
Source text
  → Parser::new(language).parse(source) → tree
    → QueryCursor::matches(query, root, bytes)
      → for each capture: build Symbol + partial Edge
```

Call graph extraction happens in a second pass inside function/method bodies — scanning for `call` / `call_expression` nodes. Edges at this stage are **partial** (target is just a name, not a full ID). Resolution happens in the graph builder.

### 3.3 Graph Builder (`graph.rs`)

Wraps `petgraph::DiGraph` and resolves partial edges:

```
1. Insert all symbols as nodes
2. Build name_index: HashMap<full_id, NodeIndex>
3. Build name_to_ids: HashMap<name, Vec<full_id>>
4. For each raw edge:
   - Try direct full-id match → add edge
   - Try name-only match via name_to_ids → add edge
   - Skip if unresolved (external/cross-module)
```

This resolution is critical — intra-file calls only have local names. The `name_to_ids` index bridges name → full ID.

### 3.4 Storage (`storage.rs`)

Straightforward batch insert:

```
1. Delete old index.db (full reindex — fast enough for most repos)
2. PRAGMA journal_mode=WAL, synchronous=NORMAL
3. CREATE TABLE symbols, edges, meta
4. Batch INSERT symbols (prepared statement)
5. Batch INSERT edges (INSERT OR IGNORE for dedup)
6. INSERT meta (schema_version, project_root, indexed_at, file_count, symbol_count)
```

### 3.5 MCP Server (`server.rs`)

Built on `rmcp` (Rust MCP SDK, v3.0.0-beta.2). Implements the full MCP lifecycle:

```
initialize → capabilities exchange
  list_tools → 7 tool schemas (name + description + JSON Schema params)
    call_tool → route to query handler → SQL → JSON response
```

**7 tools:**

| Tool | SQL |
|------|-----|
| `search_symbols` | `WHERE name LIKE ?` or `WHERE name = ?` |
| `get_symbol` | `WHERE id = ?` |
| `find_definition` | `WHERE id = ?` + file content extract |
| `find_references` | `JOIN edges WHERE target_id = ?` |
| `find_callers` | `JOIN edges WHERE target_id = ?` (+ recursive CTE for depth) |
| `list_files` | `SELECT DISTINCT file_path` |
| `get_file_summary` | `WHERE file_path = ? ORDER BY line_start` |

Recursive call tracing uses SQL recursive CTEs — standard, portable, no custom algorithm needed:

```sql
WITH RECURSIVE callers(id, name, kind, file_path, line_start, depth) AS (
    SELECT s.*, 1 FROM edges e JOIN symbols s ON s.id = e.source_id
    WHERE e.target_id = ? AND e.kind = 'calls'
    UNION ALL
    SELECT s.*, c.depth + 1
    FROM edges e JOIN symbols s ON s.id = e.source_id
    JOIN callers c ON e.target_id = c.id
    WHERE e.kind = 'calls' AND c.depth < ?
)
SELECT DISTINCT * FROM callers ORDER BY depth, name
```

## 4. Key Design Decisions

1. **Full reindex, not incremental.** Tree-sitter parsing is sub-second per file. Reindexing 342 files takes <2 seconds. Incremental indexing adds complexity that isn't worth it until >10K files.

2. **SQLite, not Postgres.** Zero setup. File-based. WAL mode allows concurrent reads during reindexing.

3. **No semantic search (v1).** Exact and fuzzy name matching covers the majority of agent queries. Semantic embeddings can be added later via `candle` or `ort` crates.

4. **Single binary.** After v0.1.0 proved the hybrid Rust+Python approach, v0.2.0 merged the Python MCP server into Rust using `rmcp`. One file, zero runtime dependencies.

## 5. Directory Structure

```
paldeck-mcp/
├── Cargo.toml
├── src/
│   ├── main.rs          # CLI (index + serve subcommands)
│   ├── walker.rs        # File discovery (gitignore-aware)
│   ├── parser.rs        # Tree-sitter: 4 languages, symbol + edge extraction
│   ├── graph.rs         # petgraph wrapper + edge resolution
│   ├── storage.rs       # SQLite persistence
│   └── server.rs        # MCP protocol (rmcp) + 7 tools
├── README.md
├── EXPLANATION.md       # ← this file
└── INTEGRATION.md       # Agent setup guides
```
