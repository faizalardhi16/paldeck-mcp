//! # Codebase Indexer — CLI Entry Point
//!
//! Orchestrates the full indexing pipeline:
//!   1. Walk the project (walker)
//!   2. Parse each source file (parser)
//!   3. Build the in-memory graph (graph)
//!   4. Persist to SQLite (storage)

mod walker;
mod parser;
mod graph;
mod storage;

use clap::Parser;
use anyhow::Result;
use std::path::PathBuf;

/// High-performance AST-based codebase indexer.
///
/// Builds a knowledge graph of symbols (functions, classes, methods, routes)
/// and their relationships (calls, imports, inheritance) into SQLite.
#[derive(Parser)]
#[command(name = "codebase-indexer")]
struct Cli {
    /// Root of the project to index
    #[arg(short, long)]
    project: PathBuf,

    /// Output SQLite database path (default: index.db in project root)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Verbose logging (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Init logging
    let log_level = match cli.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    let output = cli.output.unwrap_or_else(|| cli.project.join("index.db"));

    log::info!("🔍 Walking project: {}", cli.project.display());

    // Step 1: Discover source files
    let files = walker::discover_source_files(&cli.project)?;
    log::info!("   Found {} source files", files.len());

    // Step 2: Parse each file, extract symbols
    let mut all_symbols = Vec::new();
    let mut all_edges = Vec::new();

    for file_path in &files {
        let source = std::fs::read_to_string(file_path)?;
        let relative = file_path.strip_prefix(&cli.project).unwrap_or(file_path);

        match parser::parse_file(relative, &source, file_path) {
            Ok((mut symbols, edges)) => {
                log::debug!("   {} → {} symbols, {} edges", relative.display(), symbols.len(), edges.len());
                all_symbols.append(&mut symbols);
                all_edges.extend(edges);
            }
            Err(e) => {
                log::warn!("   ⚠ {} — parse error: {}", relative.display(), e);
            }
        }
    }

    log::info!("📊 Extracted {} symbols, {} edges", all_symbols.len(), all_edges.len());

    // Step 3: Build in-memory petgraph for cross-file edge resolution
    let g = graph::CodeGraph::build(&all_symbols, &all_edges);
    log::info!("🌐 Graph built: {} nodes", g.node_count());

    // Step 4: Persist to SQLite
    storage::persist(&output, &g, &cli.project, files.len())?;
    log::info!("💾 Persisted to {}", output.display());

    Ok(())
}
