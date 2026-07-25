//! # SQLite Persistence
//!
//! Writes the in-memory graph to SQLite — the contract between
//! the Rust indexer and the Python MCP server.

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use crate::graph::CodeGraph;

/// Schema version for future migrations.
const SCHEMA_VERSION: i64 = 1;

/// Persist the graph to SQLite.
pub fn persist(
    db_path: &Path,
    graph: &CodeGraph,
    project_root: &Path,
    file_count: usize,
) -> Result<()> {
    // Remove old DB if exists
    if db_path.exists() {
        std::fs::remove_file(db_path)?;
    }

    let conn = Connection::open(db_path)?;

    // Performance pragmas
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
    ")?;

    // Schema
    conn.execute_batch("
        CREATE TABLE symbols (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            kind        TEXT NOT NULL,
            file_path   TEXT NOT NULL,
            line_start  INTEGER NOT NULL,
            line_end    INTEGER NOT NULL,
            signature   TEXT,
            docstring   TEXT,
            properties  TEXT DEFAULT 'null'
        );

        CREATE TABLE edges (
            source_id   TEXT NOT NULL REFERENCES symbols(id),
            target_id   TEXT NOT NULL REFERENCES symbols(id),
            kind        TEXT NOT NULL,
            PRIMARY KEY (source_id, target_id, kind)
        );

        CREATE TABLE meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE INDEX idx_symbols_name   ON symbols(name);
        CREATE INDEX idx_symbols_kind   ON symbols(kind);
        CREATE INDEX idx_symbols_file   ON symbols(file_path);
        CREATE INDEX idx_edges_source   ON edges(source_id);
        CREATE INDEX idx_edges_target   ON edges(target_id);
    ")?;

    // Insert symbols
    let mut sym_stmt = conn.prepare(
        "INSERT INTO symbols (id, name, kind, file_path, line_start, line_end, signature, docstring, properties)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    )?;

    for (_, sym) in graph.iter_nodes() {
        sym_stmt.execute((
            &sym.id,
            &sym.name,
            sym.kind.as_str(),
            &sym.file_path,
            sym.line_start as i64,
            sym.line_end as i64,
            &sym.signature,
            &sym.docstring,
            &sym.properties.to_string(),
        ))?;
    }

    // Insert edges
    let mut edge_stmt = conn.prepare(
        "INSERT OR IGNORE INTO edges (source_id, target_id, kind) VALUES (?1, ?2, ?3)"
    )?;

    for (node_idx, sym) in graph.iter_nodes() {
        for (target_idx, kind) in graph.outgoing_edges(node_idx) {
            if let Some(target_sym) = graph.get_symbol(target_idx) {
                edge_stmt.execute((&sym.id, &target_sym.id, kind))?;
            }
        }
    }

    // Insert metadata
    let mut meta_stmt = conn.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
    meta_stmt.execute(("schema_version", &SCHEMA_VERSION.to_string()))?;
    meta_stmt.execute(("project_root", &project_root.to_string_lossy().to_string()))?;
    meta_stmt.execute(("indexed_at", &chrono_now()))?;
    meta_stmt.execute(("file_count", &file_count.to_string()))?;
    meta_stmt.execute(("symbol_count", &graph.node_count().to_string()))?;

    Ok(())
}

fn chrono_now() -> String {
    // Avoid chrono dependency — use std
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_default()
}
