//! # AST Parser
//!
//! Tree-sitter-based source code parser. Extracts symbols (functions, classes,
//! methods, routes) and relationships (calls, imports) from source files.
//!
//! Language dispatch happens here — one function per language grammar.

use anyhow::{anyhow, Result};
use std::path::Path;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

// ── Re-exported types ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Symbol {
    pub id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub signature: String,
    pub docstring: Option<String>,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Struct,
    Interface,
    Enum,
    Route,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Interface => "interface",
            SymbolKind::Enum => "enum",
            SymbolKind::Route => "route",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone)]
pub enum EdgeKind {
    Calls,
    Imports,
    Inherits,
    Defines,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Calls => "calls",
            EdgeKind::Imports => "imports",
            EdgeKind::Inherits => "inherits",
            EdgeKind::Defines => "defines",
        }
    }
}

// ── Parser dispatch ────────────────────────────────────────────────

pub fn parse_file(
    relative_path: &Path,
    source: &str,
    _full_path: &Path,
) -> Result<(Vec<Symbol>, Vec<Edge>)> {
    let ext = relative_path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("no file extension"))?;

    match ext {
        "py" => parse_python(relative_path, source),
        "ts" | "tsx" | "js" | "jsx" => parse_typescript(relative_path, source),
        "rs" => parse_rust(relative_path, source),
        "go" => parse_go(relative_path, source),
        _ => Err(anyhow!("unsupported extension: {}", ext)),
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn extract_node_text(node: tree_sitter::Node, bytes: &[u8]) -> String {
    node.utf8_text(bytes).unwrap_or("").to_string()
}

fn capture_name<'a>(
    query: &tree_sitter::Query,
    m: &tree_sitter::QueryMatch<'_, 'a>,
    name: &str,
) -> Option<tree_sitter::Node<'a>> {
    m.captures
        .iter()
        .find(|c| query.capture_names()[c.index as usize] == name)
        .map(|c| c.node)
}

// ── Python parser ──────────────────────────────────────────────────

fn parse_python(relative_path: &Path, source: &str) -> Result<(Vec<Symbol>, Vec<Edge>)> {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&lang)?;
    let tree = parser.parse(source, None).ok_or_else(|| anyhow!("parse failed"))?;

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let bytes = source.as_bytes();
    let path_str = relative_path.to_string_lossy().to_string();

    // ── Functions ──
    let func_query = Query::new(
        &lang,
        r#"(function_definition name: (identifier) @name parameters: (parameters) @params body: (block) @body) @func"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&func_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(fn_node), Some(name_node), Some(_params_node), Some(body_node)) = (
            capture_name(&func_query, m, "func"),
            capture_name(&func_query, m, "name"),
            capture_name(&func_query, m, "params"),
            capture_name(&func_query, m, "body"),
        ) {
            // Skip methods (functions nested inside a class body)
            if fn_node.parent().map_or(false, |p| p.kind() == "block") {
                continue;
            }
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            let sig = extract_node_text(_params_node, bytes);
            symbols.push(Symbol {
                id: id.clone(), name,
                kind: SymbolKind::Function,
                file_path: path_str.clone(),
                line_start: fn_node.start_position().row + 1,
                line_end: fn_node.end_position().row + 1,
                signature: format!("def {}({})", name_node.utf8_text(bytes)?, sig),
                docstring: extract_docstring_py(body_node, bytes),
                properties: serde_json::Value::Null,
            });
            extract_python_calls(body_node, &id, bytes, &mut edges);
        }
    }

    // ── Classes ──
    let class_query = Query::new(
        &lang,
        r#"(class_definition name: (identifier) @name body: (block) @body) @class"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&class_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(class_node), Some(name_node), Some(body_node)) = (
            capture_name(&class_query, m, "class"),
            capture_name(&func_query, m, "name"),
            capture_name(&func_query, m, "body"),
        ) {
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            symbols.push(Symbol {
                id: id.clone(), name: name.clone(),
                kind: SymbolKind::Class,
                file_path: path_str.clone(),
                line_start: class_node.start_position().row + 1,
                line_end: class_node.end_position().row + 1,
                signature: format!("class {}", name),
                docstring: extract_docstring_py(body_node, bytes),
                properties: serde_json::Value::Null,
            });
            extract_py_methods(body_node, &id, &path_str, bytes, &mut symbols, &mut edges);
        }
    }

    extract_python_imports(tree.root_node(), &path_str, bytes, &mut edges);
    Ok((symbols, edges))
}

fn extract_python_calls(
    node: tree_sitter::Node, caller_id: &str, bytes: &[u8], edges: &mut Vec<Edge>,
) {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let query = Query::new(&lang, "(call function: (identifier) @called)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, node, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "called") {
            let name = cap.node.utf8_text(bytes).unwrap().to_string();
            edges.push(Edge { source: caller_id.to_string(), target: name, kind: EdgeKind::Calls });
        }
    }
}

fn extract_py_methods(
    class_body: tree_sitter::Node, class_id: &str, path_str: &str,
    bytes: &[u8], symbols: &mut Vec<Symbol>, edges: &mut Vec<Edge>,
) {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let query = Query::new(
        &lang,
        r#"(function_definition name: (identifier) @name parameters: (parameters) @params body: (block) @body) @func"#,
    ).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, class_body, bytes);
    while let Some(m) = matches.next() {
        if let (Some(fn_node), Some(name_node), Some(params_node), Some(body_node)) = (
            capture_name(&query, m,
 "func"), capture_name(&query, m, "name"),
            capture_name(&query, m,
 "params"), capture_name(&query, m, "body"),
        ) {
            let name = name_node.utf8_text(bytes).unwrap().to_string();
            let method_id = format!("{}::{}", path_str, name);
            let sig = extract_node_text(params_node, bytes);
            symbols.push(Symbol {
                id: method_id.clone(), name,
                kind: SymbolKind::Method,
                file_path: path_str.to_string(),
                line_start: fn_node.start_position().row + 1,
                line_end: fn_node.end_position().row + 1,
                signature: format!("def {}({})", name_node.utf8_text(bytes).unwrap(), sig),
                docstring: extract_docstring_py(body_node, bytes),
                properties: serde_json::Value::Null,
            });
            edges.push(Edge { source: class_id.to_string(), target: method_id.clone(), kind: EdgeKind::Defines });
            extract_python_calls(body_node, &method_id, bytes, edges);
        }
    }
}

fn extract_python_imports(
    root: tree_sitter::Node, path_str: &str, bytes: &[u8], edges: &mut Vec<Edge>,
) {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let query = Query::new(
        &lang,
        r#"(import_statement name: (dotted_name) @module)
           (import_from_statement module_name: (dotted_name) @source name: (dotted_name) @imported)"#,
    ).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "module") {
            edges.push(Edge {
                source: path_str.to_string(),
                target: cap.node.utf8_text(bytes).unwrap().to_string(),
                kind: EdgeKind::Imports,
            });
        }
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "imported") {
            edges.push(Edge {
                source: path_str.to_string(),
                target: cap.node.utf8_text(bytes).unwrap().to_string(),
                kind: EdgeKind::Imports,
            });
        }
    }
}

fn extract_docstring_py(body_node: tree_sitter::Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = body_node.walk();
    for child in body_node.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            if let Some(inner) = child.child(0) {
                if inner.kind() == "string" {
                    return Some(inner.utf8_text(bytes).unwrap_or("").to_string());
                }
            }
        }
    }
    None
}

// ── TypeScript parser ──────────────────────────────────────────────

fn parse_typescript(relative_path: &Path, source: &str) -> Result<(Vec<Symbol>, Vec<Edge>)> {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = Parser::new();
    parser.set_language(&lang)?;
    let tree = parser.parse(source, None).ok_or_else(|| anyhow!("parse failed"))?;

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let bytes = source.as_bytes();
    let path_str = relative_path.to_string_lossy().to_string();

    // ── Functions ──
    let func_query = Query::new(
        &lang,
        r#"(function_declaration name: (identifier) @name parameters: (formal_parameters) @params body: (statement_block) @body) @func"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&func_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(fn_node), Some(name_node)) = (capture_name(&func_query, m, "func"), capture_name(&func_query, m, "name")) {
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            let sig_params = capture_name(&func_query, m, "params")
                .map(|n| extract_node_text(n, bytes)).unwrap_or_default();
            symbols.push(Symbol {
                id: id.clone(), name,
                kind: SymbolKind::Function,
                file_path: path_str.clone(),
                line_start: fn_node.start_position().row + 1,
                line_end: fn_node.end_position().row + 1,
                signature: format!("function {}({})", name_node.utf8_text(bytes)?, sig_params),
                docstring: None,
                properties: serde_json::Value::Null,
            });
            if let Some(body_node) = capture_name(&func_query, m, "body") {
                extract_ts_calls(body_node, &id, bytes, &mut edges);
            }
        }
    }

    // ── Classes ──
    let class_query = Query::new(
        &lang,
        r#"(class_declaration name: (identifier) @name body: (class_body) @body) @class"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&class_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(class_node), Some(name_node), Some(body_node)) = (
            capture_name(&class_query, m, "class"), capture_name(&func_query, m, "name"), capture_name(&func_query, m, "body"),
        ) {
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            symbols.push(Symbol {
                id: id.clone(), name: name.clone(),
                kind: SymbolKind::Class,
                file_path: path_str.clone(),
                line_start: class_node.start_position().row + 1,
                line_end: class_node.end_position().row + 1,
                signature: format!("class {}", name),
                docstring: None, properties: serde_json::Value::Null,
            });
            extract_ts_methods(body_node, &id, &path_str, bytes, &mut symbols, &mut edges);
        }
    }

    extract_ts_imports(tree.root_node(), &path_str, bytes, &mut edges);
    Ok((symbols, edges))
}

fn extract_ts_calls(node: tree_sitter::Node, caller_id: &str, bytes: &[u8], edges: &mut Vec<Edge>) {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let query = Query::new(&lang, "(call_expression function: (identifier) @called)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, node, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "called") {
            edges.push(Edge {
                source: caller_id.to_string(),
                target: cap.node.utf8_text(bytes).unwrap().to_string(),
                kind: EdgeKind::Calls,
            });
        }
    }
}

fn extract_ts_methods(
    class_body: tree_sitter::Node, class_id: &str, path_str: &str,
    bytes: &[u8], symbols: &mut Vec<Symbol>, edges: &mut Vec<Edge>,
) {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let query = Query::new(
        &lang,
        r#"(method_definition name: (property_identifier) @name parameters: (formal_parameters) @params body: (statement_block) @body) @method"#,
    ).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, class_body, bytes);
    while let Some(m) = matches.next() {
        if let (Some(method_node), Some(name_node)) = (capture_name(&query, m, "method"), capture_name(&query, m, "name")) {
            let name = name_node.utf8_text(bytes).unwrap().to_string();
            let method_id = format!("{}::{}", path_str, name);
            let sig_params = capture_name(&query, m, "params")
                .map(|n| extract_node_text(n, bytes)).unwrap_or_default();
            symbols.push(Symbol {
                id: method_id.clone(), name,
                kind: SymbolKind::Method,
                file_path: path_str.to_string(),
                line_start: method_node.start_position().row + 1,
                line_end: method_node.end_position().row + 1,
                signature: format!("{}({})", name_node.utf8_text(bytes).unwrap(), sig_params),
                docstring: None, properties: serde_json::Value::Null,
            });
            edges.push(Edge { source: class_id.to_string(), target: method_id.clone(), kind: EdgeKind::Defines });
            if let Some(body_node) = capture_name(&query, m, "body") {
                extract_ts_calls(body_node, &method_id, bytes, edges);
            }
        }
    }
}

fn extract_ts_imports(root: tree_sitter::Node, path_str: &str, bytes: &[u8], edges: &mut Vec<Edge>) {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let query = Query::new(&lang, r#"(import_statement source: (string) @source) @import"#).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "source") {
            let source_text = cap.node.utf8_text(bytes).unwrap();
            edges.push(Edge {
                source: path_str.to_string(),
                target: source_text.trim_matches(&['"', '\''] as &[char]).to_string(),
                kind: EdgeKind::Imports,
            });
        }
    }
}

// ── Rust parser ────────────────────────────────────────────────────

fn parse_rust(relative_path: &Path, source: &str) -> Result<(Vec<Symbol>, Vec<Edge>)> {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&lang)?;
    let tree = parser.parse(source, None).ok_or_else(|| anyhow!("parse failed"))?;

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let bytes = source.as_bytes();
    let path_str = relative_path.to_string_lossy().to_string();

    // ── Functions ──
    let func_query = Query::new(
        &lang,
        r#"(function_item name: (identifier) @name parameters: (parameters) @params body: (block) @body) @func"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&func_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(fn_node), Some(name_node), Some(params_node), Some(body_node)) = (
            capture_name(&func_query, m, "func"), capture_name(&func_query, m, "name"),
            capture_name(&func_query, m, "params"), capture_name(&func_query, m, "body"),
        ) {
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            let sig = extract_node_text(params_node, bytes);
            symbols.push(Symbol {
                id: id.clone(), name,
                kind: SymbolKind::Function,
                file_path: path_str.clone(),
                line_start: fn_node.start_position().row + 1,
                line_end: fn_node.end_position().row + 1,
                signature: format!("fn {}({})", name_node.utf8_text(bytes)?, sig),
                docstring: None, properties: serde_json::Value::Null,
            });
            extract_rust_calls(body_node, &id, bytes, &mut edges);
        }
    }

    // ── Structs ──
    let struct_query = Query::new(
        &lang,
        r#"(struct_item name: (type_identifier) @name) @struct"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&struct_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(s), Some(n)) = (capture_name(&struct_query, m, "struct"), capture_name(&struct_query, m, "name")) {
            let name = n.utf8_text(bytes)?.to_string();
            symbols.push(Symbol {
                id: format!("{}::{}", path_str, name), name,
                kind: SymbolKind::Struct,
                file_path: path_str.clone(),
                line_start: s.start_position().row + 1,
                line_end: s.end_position().row + 1,
                signature: format!("struct {}", n.utf8_text(bytes)?),
                docstring: None, properties: serde_json::Value::Null,
            });
        }
    }

    Ok((symbols, edges))
}

fn extract_rust_calls(node: tree_sitter::Node, caller_id: &str, bytes: &[u8], edges: &mut Vec<Edge>) {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = Query::new(&lang, "(call_expression function: (identifier) @called)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, node, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "called") {
            edges.push(Edge {
                source: caller_id.to_string(),
                target: cap.node.utf8_text(bytes).unwrap().to_string(),
                kind: EdgeKind::Calls,
            });
        }
    }
}

// ── Go parser ──────────────────────────────────────────────────────

fn parse_go(relative_path: &Path, source: &str) -> Result<(Vec<Symbol>, Vec<Edge>)> {
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&lang)?;
    let tree = parser.parse(source, None).ok_or_else(|| anyhow!("parse failed"))?;

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let bytes = source.as_bytes();
    let path_str = relative_path.to_string_lossy().to_string();

    // ── Functions ──
    let func_query = Query::new(
        &lang,
        r#"(function_declaration name: (identifier) @name parameters: (parameter_list) @params body: (block) @body) @func"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&func_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(fn_node), Some(name_node), Some(params_node), Some(body_node)) = (
            capture_name(&func_query, m, "func"), capture_name(&func_query, m, "name"),
            capture_name(&func_query, m, "params"), capture_name(&func_query, m, "body"),
        ) {
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            let sig = extract_node_text(params_node, bytes);
            symbols.push(Symbol {
                id: id.clone(), name,
                kind: SymbolKind::Function,
                file_path: path_str.clone(),
                line_start: fn_node.start_position().row + 1,
                line_end: fn_node.end_position().row + 1,
                signature: format!("func {}({})", name_node.utf8_text(bytes)?, sig),
                docstring: None, properties: serde_json::Value::Null,
            });
            extract_go_calls(body_node, &id, bytes, &mut edges);
        }
    }

    // ── Methods ──
    let method_query = Query::new(
        &lang,
        r#"(method_declaration name: (field_identifier) @name parameters: (parameter_list) @params body: (block) @body) @method"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&method_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(method_node), Some(name_node), Some(params_node), Some(body_node)) = (
            capture_name(&method_query, m, "method"), capture_name(&func_query, m, "name"),
            capture_name(&func_query, m, "params"), capture_name(&func_query, m, "body"),
        ) {
            let name = name_node.utf8_text(bytes)?.to_string();
            let id = format!("{}::{}", path_str, name);
            let sig = extract_node_text(params_node, bytes);
            symbols.push(Symbol {
                id: id.clone(), name,
                kind: SymbolKind::Method,
                file_path: path_str.clone(),
                line_start: method_node.start_position().row + 1,
                line_end: method_node.end_position().row + 1,
                signature: format!("func (r) {}({})", name_node.utf8_text(bytes)?, sig),
                docstring: None, properties: serde_json::Value::Null,
            });
            extract_go_calls(body_node, &id, bytes, &mut edges);
        }
    }

    // ── Structs ──
    let struct_query = Query::new(
        &lang,
        r#"(type_declaration (type_spec name: (type_identifier) @name type: (struct_type) @body)) @struct"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&struct_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let (Some(s), Some(n)) = (capture_name(&struct_query, m, "struct"), capture_name(&struct_query, m, "name")) {
            let name = n.utf8_text(bytes)?.to_string();
            symbols.push(Symbol {
                id: format!("{}::{}", path_str, name), name: name.clone(),
                kind: SymbolKind::Struct,
                file_path: path_str.clone(),
                line_start: s.start_position().row + 1,
                line_end: s.end_position().row + 1,
                signature: format!("type {} struct{{...}}", name),
                docstring: None, properties: serde_json::Value::Null,
            });
        }
    }

    // ── Interfaces ──
    let iface_query = Query::new(
        &lang,
        r#"(type_declaration (type_spec name: (type_identifier) @name type: (interface_type) @body)) @iface"#,
    )?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&iface_query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| iface_query.capture_names()[c.index as usize] == "name") {
            let name = cap.node.utf8_text(bytes)?.to_string();
            symbols.push(Symbol {
                id: format!("{}::{}", path_str, name), name,
                kind: SymbolKind::Interface,
                file_path: path_str.clone(),
                line_start: cap.node.start_position().row + 1,
                line_end: cap.node.end_position().row + 1,
                signature: format!("type {} interface{{...}}", cap.node.utf8_text(bytes)?),
                docstring: None, properties: serde_json::Value::Null,
            });
        }
    }

    extract_go_imports(tree.root_node(), &path_str, bytes, &mut edges);
    Ok((symbols, edges))
}

fn extract_go_calls(node: tree_sitter::Node, caller_id: &str, bytes: &[u8], edges: &mut Vec<Edge>) {
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    let query = Query::new(&lang, "(call_expression function: (identifier) @called)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, node, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "called") {
            edges.push(Edge {
                source: caller_id.to_string(),
                target: cap.node.utf8_text(bytes).unwrap().to_string(),
                kind: EdgeKind::Calls,
            });
        }
    }
}

fn extract_go_imports(root: tree_sitter::Node, path_str: &str, bytes: &[u8], edges: &mut Vec<Edge>) {
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    let query = Query::new(
        &lang,
        r#"(import_declaration (import_spec path: (interpreted_string_literal) @path))"#,
    ).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, bytes);
    while let Some(m) = matches.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "path") {
            edges.push(Edge {
                source: path_str.to_string(),
                target: cap.node.utf8_text(bytes).unwrap().trim_matches('"').to_string(),
                kind: EdgeKind::Imports,
            });
        }
    }
}
