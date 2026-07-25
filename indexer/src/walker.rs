//! # File Walker
//!
//! Gitignore-aware filesystem traversal. Discovers all source files
//! in a project, respecting `.gitignore` and skipping common non-code dirs.

use anyhow::Result;

use std::path::{Path, PathBuf};

/// File extensions we can parse.
const SUPPORTED_EXTENSIONS: &[&str] = &["py", "ts", "tsx", "rs", "js", "jsx", "go"];

/// Directories to always skip (even without .gitignore entries).
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "__pycache__", "target",    // build artifacts
    "venv", ".venv", ".env", "dist", "build",           // venv + dist
    ".mypy_cache", ".pytest_cache", ".ruff_cache",       // tool caches
    ".next", ".turbo", "coverage",                       // JS tooling
];

/// Walk the project root and return all parseable source file paths.
/// If `scope` is provided, only files under that subdirectory are discovered.
pub fn discover_source_files(root: &Path, scope: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    let scope_abs = scope.map(|s| {
        if s.is_absolute() { s.to_path_buf() } else { root.join(s) }
    });

    // `ignore::Walk` automatically respects `.gitignore` and skips common
    // build-artifact directories (node_modules, target, etc.) out of the box.
    for entry in ignore::Walk::new(root) {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }

        // Scope filter: skip files outside the target subdirectory
        if let Some(ref scope_path) = scope_abs {
            if !path.starts_with(scope_path) {
                continue;
            }
        }

        // Check extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if SUPPORTED_EXTENSIONS.contains(&ext) {
                files.push(path.to_path_buf());
            }
        }
    }

    Ok(files)
}
