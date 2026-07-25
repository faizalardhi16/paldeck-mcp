"""
MCP Codebase Server — reads the Rust-indexed SQLite knowledge graph
and exposes graph-traversal tools to Hermes Agent (or any MCP client).

Protocol: stdio (JSON-RPC via mcp package)
Requires: mcp, networkx (optional — enables advanced graph queries)
"""

from __future__ import annotations

import asyncio
import json
import sqlite3
import subprocess
import sys
from pathlib import Path
from typing import Optional

from mcp.server import Server, stdio_server
from mcp.types import Tool, TextContent

# ── Configuration ──────────────────────────────────────────────────

DEFAULT_INDEXER_BINARY = "codebase-indexer"  # on PATH or absolute
DEFAULT_DB_NAME = "index.db"

# ── SQLite Helpers ──────────────────────────────────────────────────

def open_db(db_path: Path) -> sqlite3.Connection:
    """Open (or create) the index SQLite database."""
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode = WAL")
    return conn


def discover_db(project_root: Path) -> Optional[Path]:
    """Find the index.db — either at project root or a configured path."""
    candidate = project_root / DEFAULT_DB_NAME
    if candidate.exists():
        return candidate
    return None


# ── Reindex trigger ────────────────────────────────────────────────

async def run_indexer(project_root: Path, db_path: Path) -> str:
    """Launch the Rust indexer as a subprocess. Returns stdout or error."""
    try:
        proc = await asyncio.create_subprocess_exec(
            DEFAULT_INDEXER_BINARY,
            "--project", str(project_root),
            "--output", str(db_path),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()

        if proc.returncode != 0:
            return f"ERROR: indexer failed (exit {proc.returncode}):\n{stderr.decode()}"
        return stdout.decode()
    except FileNotFoundError:
        return (
            f"ERROR: Rust indexer binary '{DEFAULT_INDEXER_BINARY}' not found.\n"
            f"Build it with: cd indexer && cargo build --release\n"
            f"Then symlink or set PATH: ln -s $(pwd)/target/release/codebase-indexer /usr/local/bin/"
        )


# ── Query Layer ────────────────────────────────────────────────────

class QueryEngine:
    """Answers graph queries from the SQLite index."""

    def __init__(self, conn: sqlite3.Connection):
        self.conn = conn

    def search_symbol(
        self, query: str, kind: Optional[str] = None, fuzzy: bool = False, limit: int = 50
    ) -> list[dict]:
        operator = "LIKE" if fuzzy else "="
        pattern = f"%{query}%" if fuzzy else query

        sql = """
            SELECT id, name, kind, file_path, line_start, line_end, signature, docstring
            FROM symbols
            WHERE name {}
        """.format(operator)

        params: list = [pattern]
        if kind:
            sql += " AND kind = ?"
            params.append(kind)

        sql += " ORDER BY name LIMIT ?"
        params.append(limit)

        rows = self.conn.execute(sql, params).fetchall()
        return [dict(r) for r in rows]

    def trace_callers(self, symbol_id: str, depth: int = 1) -> list[dict]:
        """Find all symbols that CALL the given symbol (inbound edges)."""
        if depth == 1:
            sql = """
                SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature
                FROM edges e
                JOIN symbols s ON s.id = e.source_id
                WHERE e.target_id = ? AND e.kind = 'calls'
                ORDER BY s.name
            """
            return [dict(r) for r in self.conn.execute(sql, (symbol_id,)).fetchall()]

        # Depth > 1: Recursive CTE
        sql = """
            WITH RECURSIVE callers(id, name, kind, file_path, line_start, signature, depth) AS (
                SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, 1
                FROM edges e
                JOIN symbols s ON s.id = e.source_id
                WHERE e.target_id = ? AND e.kind = 'calls'
                UNION ALL
                SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, c.depth + 1
                FROM edges e
                JOIN symbols s ON s.id = e.source_id
                JOIN callers c ON e.target_id = c.id
                WHERE e.kind = 'calls' AND c.depth < ?
            )
            SELECT DISTINCT * FROM callers ORDER BY depth, name
        """
        return [dict(r) for r in self.conn.execute(sql, (symbol_id, depth)).fetchall()]

    def trace_callees(self, symbol_id: str, depth: int = 1) -> list[dict]:
        """Find all symbols CALLED BY the given symbol (outbound edges)."""
        if depth == 1:
            sql = """
                SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature
                FROM edges e
                JOIN symbols s ON s.id = e.target_id
                WHERE e.source_id = ? AND e.kind = 'calls'
                ORDER BY s.name
            """
            return [dict(r) for r in self.conn.execute(sql, (symbol_id,)).fetchall()]

        sql = """
            WITH RECURSIVE callees(id, name, kind, file_path, line_start, signature, depth) AS (
                SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, 1
                FROM edges e
                JOIN symbols s ON s.id = e.target_id
                WHERE e.source_id = ? AND e.kind = 'calls'
                UNION ALL
                SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, c.depth + 1
                FROM edges e
                JOIN symbols s ON s.id = e.target_id
                JOIN callees c ON e.source_id = c.id
                WHERE e.kind = 'calls' AND c.depth < ?
            )
            SELECT DISTINCT * FROM callees ORDER BY depth, name
        """
        return [dict(r) for r in self.conn.execute(sql, (symbol_id, depth)).fetchall()]

    def get_architecture(self, path: str) -> dict:
        """Summarize all symbols under a file or directory path."""
        prefix = path.rstrip("/")

        symbols = self.conn.execute(
            "SELECT id, name, kind, file_path, line_start, signature FROM symbols WHERE file_path LIKE ? || '%' ORDER BY kind, name",
            (prefix,),
        ).fetchall()

        # Group by kind
        by_kind: dict[str, list[dict]] = {}
        for row in symbols:
            r = dict(row)
            by_kind.setdefault(r["kind"], []).append(r)

        # Count edges within this scope
        symbol_ids = [r["id"] for r in symbols]
        edge_count = 0
        if symbol_ids:
            placeholders = ",".join("?" * len(symbol_ids))
            edge_count = self.conn.execute(
                f"SELECT COUNT(*) FROM edges WHERE source_id IN ({placeholders}) AND target_id IN ({placeholders})",
                (*symbol_ids, *symbol_ids),
            ).fetchone()[0]

        return {
            "path": path,
            "total_symbols": len(symbols),
            "by_kind": by_kind,
            "internal_edges": edge_count,
        }

    def list_symbols_in_file(self, file_path: str) -> list[dict]:
        rows = self.conn.execute(
            "SELECT id, name, kind, line_start, line_end, signature, docstring FROM symbols WHERE file_path = ? ORDER BY line_start",
            (file_path,),
        ).fetchall()
        return [dict(r) for r in rows]

    def index_status(self) -> dict:
        rows = self.conn.execute("SELECT key, value FROM meta").fetchall()
        meta = {r["key"]: r["value"] for r in rows}

        # Additional stats
        symbol_count = self.conn.execute("SELECT COUNT(*) FROM symbols").fetchone()[0]
        edge_count = self.conn.execute("SELECT COUNT(*) FROM edges").fetchone()[0]
        file_count = self.conn.execute("SELECT COUNT(DISTINCT file_path) FROM symbols").fetchone()[0]

        return {
            **meta,
            "symbol_count": symbol_count,
            "edge_count": edge_count,
            "indexed_file_count": file_count,
        }


# ── MCP Server ─────────────────────────────────────────────────────

project_root: Path = Path.cwd()
db_path: Path = project_root / DEFAULT_DB_NAME
engine: Optional[QueryEngine] = None

app = Server("codebase-mcp")


@app.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="search_symbol",
            description="Search for symbols (functions, classes, methods) by name. Use fuzzy=true for pattern matching.",
            inputSchema={
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Name or pattern to search for"},
                    "kind": {"type": "string", "description": "Filter: function, class, method, struct, interface, enum"},
                    "fuzzy": {"type": "boolean", "default": False, "description": "Use LIKE matching instead of exact"},
                },
                "required": ["query"],
            },
        ),
        Tool(
            name="trace_callers",
            description="Find all symbols that CALL the given symbol (inbound call graph). Use depth>1 for recursive tracing.",
            inputSchema={
                "type": "object",
                "properties": {
                    "symbol_id": {"type": "string", "description": "Full symbol ID, e.g. 'src/main.py::calculate_total'"},
                    "depth": {"type": "integer", "default": 1, "description": "How many levels up to trace"},
                },
                "required": ["symbol_id"],
            },
        ),
        Tool(
            name="trace_callees",
            description="Find all symbols CALLED BY the given symbol (outbound call graph).",
            inputSchema={
                "type": "object",
                "properties": {
                    "symbol_id": {"type": "string", "description": "Full symbol ID"},
                    "depth": {"type": "integer", "default": 1},
                },
                "required": ["symbol_id"],
            },
        ),
        Tool(
            name="get_architecture",
            description="Get architecture summary: all symbols grouped by kind, within a file or directory.",
            inputSchema={
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File or directory path relative to project root, e.g. 'src/api/' or 'main.py'"},
                },
                "required": ["path"],
            },
        ),
        Tool(
            name="list_symbols_in_file",
            description="List all symbols defined in a specific file, ordered by line number.",
            inputSchema={
                "type": "object",
                "properties": {
                    "file_path": {"type": "string", "description": "Relative path, e.g. 'src/handlers/user_handler.py'"},
                },
                "required": ["file_path"],
            },
        ),
        Tool(
            name="index_status",
            description="Get indexing status: project root, file count, symbol count, edge count, last indexed time.",
            inputSchema={"type": "object", "properties": {}},
        ),
        Tool(
            name="reindex",
            description="Re-run the Rust indexer to pick up code changes.",
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
    ]


@app.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    global engine, db_path

    # Lazy-load or re-load engine
    if engine is None:
        conn = open_db(db_path)
        engine = QueryEngine(conn)

    try:
        if name == "search_symbol":
            results = engine.search_symbol(
                arguments["query"],
                kind=arguments.get("kind"),
                fuzzy=arguments.get("fuzzy", False),
            )
            return [TextContent(type="text", text=json.dumps(results, indent=2, ensure_ascii=False))]

        elif name == "trace_callers":
            results = engine.trace_callers(
                arguments["symbol_id"],
                depth=arguments.get("depth", 1),
            )
            return [TextContent(type="text", text=json.dumps(results, indent=2, ensure_ascii=False))]

        elif name == "trace_callees":
            results = engine.trace_callees(
                arguments["symbol_id"],
                depth=arguments.get("depth", 1),
            )
            return [TextContent(type="text", text=json.dumps(results, indent=2, ensure_ascii=False))]

        elif name == "get_architecture":
            results = engine.get_architecture(arguments["path"])
            return [TextContent(type="text", text=json.dumps(results, indent=2, ensure_ascii=False))]

        elif name == "list_symbols_in_file":
            results = engine.list_symbols_in_file(arguments["file_path"])
            return [TextContent(type="text", text=json.dumps(results, indent=2, ensure_ascii=False))]

        elif name == "index_status":
            results = engine.index_status()
            return [TextContent(type="text", text=json.dumps(results, indent=2, ensure_ascii=False))]

        elif name == "reindex":
            output = await run_indexer(project_root, db_path)
            # Reload engine after reindex
            conn = open_db(db_path)
            engine = QueryEngine(conn)
            return [TextContent(type="text", text=output)]

        else:
            return [TextContent(type="text", text=f"Unknown tool: {name}")]

    except Exception as e:
        return [TextContent(type="text", text=f"Error: {type(e).__name__}: {e}")]


async def main():
    global project_root, db_path

    # Allow overriding project root via env var
    if env_root := Path(__file__).parent.parent.parent:
        project_root = env_root.resolve()

    # Attempt to discover or create index
    db_path = discover_db(project_root) or (project_root / DEFAULT_DB_NAME)

    if not db_path.exists():
        print(f"[codebase-mcp] No index found at {db_path}. Run 'reindex' tool or build indexer first.", file=sys.stderr)

    async with stdio_server() as (read, write):
        await app.run(read, write, app.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())
