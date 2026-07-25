//! MCP Server — serves the indexed codebase knowledge graph to AI agents.
//!
//! Uses rmcp 3.0 with the declarative `#[tool]` macro pattern.
//! Reads the SQLite index created by the Rust indexer.

use std::path::PathBuf;
use std::sync::Mutex;

use rmcp::{
    tool_router, tool, ServiceExt,
    handler::server::wrapper::{Parameters, Json},
};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

/// Shared state — holds the open SQLite connection
pub struct AppState {
    pub conn: Mutex<rusqlite::Connection>,
    pub db_path: PathBuf,
}

// ── Tool parameter types ──────────────────────────────────────────

#[derive(Deserialize, JsonSchema)]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub fuzzy: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SymbolIdParams {
    pub symbol_id: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_depth() -> usize { 1 }

#[derive(Deserialize, JsonSchema)]
pub struct PathParams {
    pub path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct FilePathParams {
    pub file_path: String,
}

// ── Output types ──────────────────────────────────────────────────

#[derive(Serialize, JsonSchema)]
pub struct SymbolRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line_start: usize,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ArchOutput {
    pub path: String,
    pub total_symbols: usize,
    pub by_kind: std::collections::HashMap<String, Vec<SymbolRow>>,
    pub internal_edges: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct StatusOutput {
    pub project_root: Option<String>,
    pub indexed_at: Option<String>,
    pub file_count: Option<String>,
    pub symbol_count: usize,
    pub edge_count: usize,
    pub indexed_file_count: usize,
}

// ── SQL helpers ───────────────────────────────────────────────────

fn query_rows(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[Box<dyn rusqlite::types::ToSql>],
) -> Result<Vec<SymbolRow>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| {
                Ok(SymbolRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    line_start: row.get::<_, i64>(4)? as usize,
                    signature: row.get(5)?,
                    docstring: row.get(6).ok(),
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

// ── Tools ─────────────────────────────────────────────────────────

#[tool_router(server_handler)]
impl AppState {
    #[tool(name = "search_symbol", description = "Search for symbols (functions, classes, methods) by name. Use fuzzy=true for pattern matching.")]
    fn search_symbol(&self, Parameters(p): Parameters<SearchParams>) -> Json<String> {
        let conn = self.conn.lock().unwrap();
        let (op, pattern) = if p.fuzzy.unwrap_or(false) {
            ("LIKE", format!("%{}%", p.query))
        } else {
            ("=", p.query.clone())
        };

        let mut sql = format!(
            "SELECT id, name, kind, file_path, line_start, signature, docstring FROM symbols WHERE name {}",
            op
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pattern)];

        if let Some(ref k) = p.kind {
            sql.push_str(" AND kind = ?2");
            params.push(Box::new(k.clone()));
        }
        sql.push_str(" ORDER BY name LIMIT 50");

        match query_rows(&conn, &sql, &params) {
            Ok(rows) => Json(serde_json::to_string_pretty(&rows).unwrap_or_default()),
            Err(e) => Json(format!("Error: {}", e)),
        }
    }

    #[tool(name = "trace_callers", description = "Find all symbols that CALL the given symbol (inbound call graph). Use depth>1 for recursive tracing.")]
    fn trace_callers(&self, Parameters(p): Parameters<SymbolIdParams>) -> Json<String> {
        let conn = self.conn.lock().unwrap();

        let sql = if p.depth == 1 {
            "SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, s.docstring \
             FROM edges e JOIN symbols s ON s.id = e.source_id \
             WHERE e.target_id = ?1 AND e.kind = 'calls' ORDER BY s.name"
                .to_string()
        } else {
            format!(
                "WITH RECURSIVE callers(id, name, kind, file_path, line_start, signature, depth) AS ( \
                 SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, 1 \
                 FROM edges e JOIN symbols s ON s.id = e.source_id \
                 WHERE e.target_id = ?1 AND e.kind = 'calls' \
                 UNION ALL \
                 SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, c.depth + 1 \
                 FROM edges e JOIN symbols s ON s.id = e.source_id \
                 JOIN callers c ON e.target_id = c.id \
                 WHERE e.kind = 'calls' AND c.depth < {} \
                 ) SELECT DISTINCT id, name, kind, file_path, line_start, signature FROM callers ORDER BY depth, name",
                p.depth
            )
        };

        let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(p.symbol_id)];
        match query_rows(&conn, &sql, &params) {
            Ok(rows) => Json(serde_json::to_string_pretty(&rows).unwrap_or_default()),
            Err(e) => Json(format!("Error: {}", e)),
        }
    }

    #[tool(name = "trace_callees", description = "Find all symbols CALLED BY the given symbol (outbound call graph).")]
    fn trace_callees(&self, Parameters(p): Parameters<SymbolIdParams>) -> Json<String> {
        let conn = self.conn.lock().unwrap();

        let sql = if p.depth == 1 {
            "SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, s.docstring \
             FROM edges e JOIN symbols s ON s.id = e.target_id \
             WHERE e.source_id = ?1 AND e.kind = 'calls' ORDER BY s.name"
                .to_string()
        } else {
            format!(
                "WITH RECURSIVE callees(id, name, kind, file_path, line_start, signature, depth) AS ( \
                 SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, 1 \
                 FROM edges e JOIN symbols s ON s.id = e.target_id \
                 WHERE e.source_id = ?1 AND e.kind = 'calls' \
                 UNION ALL \
                 SELECT s.id, s.name, s.kind, s.file_path, s.line_start, s.signature, c.depth + 1 \
                 FROM edges e JOIN symbols s ON s.id = e.target_id \
                 JOIN callees c ON e.source_id = c.id \
                 WHERE e.kind = 'calls' AND c.depth < {} \
                 ) SELECT DISTINCT id, name, kind, file_path, line_start, signature FROM callees ORDER BY depth, name",
                p.depth
            )
        };

        let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(p.symbol_id)];
        match query_rows(&conn, &sql, &params) {
            Ok(rows) => Json(serde_json::to_string_pretty(&rows).unwrap_or_default()),
            Err(e) => Json(format!("Error: {}", e)),
        }
    }

    #[tool(name = "get_architecture", description = "Get architecture summary: all symbols grouped by kind, within a file or directory.")]
    fn get_architecture(&self, Parameters(p): Parameters<PathParams>) -> Json<String> {
        let conn = self.conn.lock().unwrap();
        let prefix = p.path.trim_end_matches('/');

        let mut stmt = conn
            .prepare(
                "SELECT id, name, kind, file_path, line_start, signature, docstring \
                 FROM symbols WHERE file_path LIKE ?1 || '%' ORDER BY kind, name",
            )
            .unwrap();

        let rows: Vec<SymbolRow> = stmt
            .query_map([&prefix], |row| {
                Ok(SymbolRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    line_start: row.get::<_, i64>(4)? as usize,
                    signature: row.get(5)?,
                    docstring: row.get(6).ok(),
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        let total = rows.len();
        let mut by_kind: std::collections::HashMap<String, Vec<SymbolRow>> =
            std::collections::HashMap::new();
        for row in rows {
            by_kind.entry(row.kind.clone()).or_default().push(row);
        }

        let ids: Vec<String> = by_kind.values().flatten().map(|r| r.id.clone()).collect();
        let edge_count = if ids.is_empty() {
            0
        } else {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT COUNT(*) FROM edges WHERE source_id IN ({}) AND target_id IN ({})",
                placeholders, placeholders
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for id in &ids {
                params.push(Box::new(id.clone()));
            }
            for id in &ids {
                params.push(Box::new(id.clone()));
            }
            conn.query_row(
                &sql,
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
        };

        let output = ArchOutput {
            path: p.path,
            total_symbols: total,
            by_kind,
            internal_edges: edge_count,
        };
        Json(serde_json::to_string_pretty(&output).unwrap_or_default())
    }

    #[tool(name = "list_symbols_in_file", description = "List all symbols defined in a specific file, ordered by line number.")]
    fn list_symbols_in_file(&self, Parameters(p): Parameters<FilePathParams>) -> Json<String> {
        let conn = self.conn.lock().unwrap();
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(p.file_path)];
        match query_rows(
            &conn,
            "SELECT id, name, kind, file_path, line_start, signature, docstring FROM symbols WHERE file_path = ?1 ORDER BY line_start",
            &params,
        ) {
            Ok(rows) => Json(serde_json::to_string_pretty(&rows).unwrap_or_default()),
            Err(e) => Json(format!("Error: {}", e)),
        }
    }

    #[tool(name = "index_status", description = "Get indexing status: project root, file count, symbol count, edge count, last indexed time.")]
    fn index_status(&self) -> Json<String> {
        let conn = self.conn.lock().unwrap();

        let meta: std::collections::HashMap<String, String> = {
            let mut stmt = conn.prepare("SELECT key, value FROM meta").unwrap();
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap();
            rows.filter_map(|r| r.ok()).collect()
        };

        let symbol_count = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize;
        let edge_count = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize;
        let file_count = conn
            .query_row("SELECT COUNT(DISTINCT file_path) FROM symbols", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize;

        let output = StatusOutput {
            project_root: meta.get("project_root").cloned(),
            indexed_at: meta.get("indexed_at").cloned(),
            file_count: meta.get("file_count").cloned(),
            symbol_count,
            edge_count,
            indexed_file_count: file_count,
        };

        Json(serde_json::to_string_pretty(&output).unwrap_or_default())
    }

    #[tool(name = "reindex", description = "Re-run the indexer to pick up code changes.")]
    fn reindex(&self) -> Json<String> {
        let db_path = self.db_path.clone();
        drop(self.conn.lock().unwrap());

        let parent = db_path
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("codebase-mcp"));

        match std::process::Command::new(&exe)
            .args(["index", "--project"])
            .arg(parent)
            .output()
        {
            Ok(out) => {
                let msg = String::from_utf8_lossy(&out.stdout).to_string();
                if let Ok(new_conn) = rusqlite::Connection::open(&db_path) {
                    *self.conn.lock().unwrap() = new_conn;
                }
                Json(msg)
            }
            Err(e) => Json(format!(
                "Reindex failed: {}. Run 'codebase-mcp index --project <path>' manually.",
                e
            )),
        }
    }
}

// ── Entry point for `serve` subcommand ────────────────────────────

pub async fn run_server(
    db_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;

    let state = AppState {
        conn: Mutex::new(conn),
        db_path,
    };

    let transport = (tokio::io::stdin(), tokio::io::stdout());
    eprintln!("[codebase-mcp] MCP server starting on stdio...");
    let service = state.serve(transport).await?;
    eprintln!("[codebase-mcp] Connected. 7 tools registered.");
    service.waiting().await;
    Ok(())
}
