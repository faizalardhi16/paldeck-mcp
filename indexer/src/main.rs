//! # Paldeck MCP — Single binary for indexing + serving
//!
//! Two modes:
//!   paldeck-mcp index --project /path --scope src/auth  → index a module
//!   paldeck-mcp serve                                    → start MCP server (reads index.db)

mod walker;
mod parser;
mod graph;
mod storage;
mod server;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "paldeck-mcp", about = "Paldeck MCP — single-binary codebase indexer + MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a project — parse source files into SQLite knowledge graph
    Index {
        /// Root of the project to index
        #[arg(short, long)]
        project: PathBuf,

        /// Only index files under this subdirectory (e.g., "src/auth")
        #[arg(short, long)]
        scope: Option<PathBuf>,

        /// Output SQLite database path (default: index.db in project root)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Verbose logging (-v, -vv)
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },
    /// Start MCP server — serve the indexed codebase to AI agents over stdio
    Serve {
        /// Path to index.db (default: ./index.db)
        #[arg(short, long, default_value = "index.db")]
        db: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Index { project, scope, output, verbose } => {
            let log_level = match verbose {
                0 => "info",
                1 => "debug",
                _ => "trace",
            };
            env_logger::Builder::from_env(
                env_logger::Env::default().default_filter_or(log_level)
            ).init();

            let output = output.unwrap_or_else(|| project.join("index.db"));
            if let Some(ref s) = scope {
                log::info!("🔍 Walking project: {} (scope: {})", project.display(), s.display());
            } else {
                log::info!("🔍 Walking project: {}", project.display());
            }

            let files = walker::discover_source_files(&project, scope.as_deref())?;
            log::info!("   Found {} source files", files.len());

            let mut all_symbols = Vec::new();
            let mut all_edges = Vec::new();

            for file_path in &files {
                let source = std::fs::read_to_string(file_path)?;
                let relative = file_path.strip_prefix(&project).unwrap_or(file_path);
                match parser::parse_file(relative, &source, file_path) {
                    Ok((mut syms, es)) => {
                        log::debug!("   {} → {} symbols, {} edges", relative.display(), syms.len(), es.len());
                        all_symbols.append(&mut syms);
                        all_edges.extend(es);
                    }
                    Err(e) => {
                        log::warn!("   ⚠ {} — parse error: {}", relative.display(), e);
                    }
                }
            }

            log::info!("📊 Extracted {} symbols, {} edges", all_symbols.len(), all_edges.len());
            let g = graph::CodeGraph::build(&all_symbols, &all_edges);
            log::info!("🌐 Graph built: {} nodes", g.node_count());
            storage::persist(&output, &g, &project, files.len())?;
            log::info!("💾 Persisted to {}", output.display());
        }

        Command::Serve { db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                if let Err(e) = server::run_server(db).await {
                    eprintln!("Server error: {}", e);
                }
            });
        }
    }

    Ok(())
}
