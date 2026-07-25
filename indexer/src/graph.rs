//! # In-Memory Symbol Graph
//!
//! Wraps `petgraph::DiGraph` to hold all symbols + edges.
//! After all files are parsed, the graph builder resolves partial edges
//! (where target is just a name) into full edges (where target is a symbol ID).

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;
use crate::parser::{Symbol, Edge};

/// The in-memory graph before persistence.
pub struct CodeGraph {
    /// The directed graph: nodes = symbols, edges = relationships.
    graph: DiGraph<Symbol, String>,
    /// Fast lookup: symbol ID → node index.
    name_index: HashMap<String, NodeIndex>,
    /// Name-only lookup for resolving call targets.
    name_to_ids: HashMap<String, Vec<String>>,
}

impl CodeGraph {
    /// Build the graph from flat lists of symbols and raw edges.
    pub fn build(symbols: &[Symbol], raw_edges: &[Edge]) -> Self {
        let mut graph = DiGraph::<Symbol, String>::new();
        let mut name_index = HashMap::new();
        let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();

        // Add all symbol nodes
        for sym in symbols {
            let idx = graph.add_node(sym.clone());
            name_index.insert(sym.id.clone(), idx);
            name_to_ids
                .entry(sym.name.clone())
                .or_default()
                .push(sym.id.clone());
        }

        // Add edges — resolve name-only targets where possible
        for edge in raw_edges {
            let source_idx = name_index.get(&edge.source);

            // Try direct ID match first, then name match
            let target_idx = name_index.get(&edge.target).or_else(|| {
                // If target is just a name (not a full ID), try to resolve
                name_to_ids
                    .get(&edge.target)
                    .and_then(|ids| name_index.get(&ids[0]))
            });

            if let (Some(&src), Some(&tgt)) = (source_idx, target_idx) {
                graph.add_edge(src, tgt, edge.kind.as_str().to_string());
            }
        }

        Self {
            graph,
            name_index,
            name_to_ids,
        }
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Iterate all nodes and their edges for persistence.
    pub fn iter_nodes(&self) -> impl Iterator<Item = (NodeIndex, &Symbol)> {
        self.graph.node_indices().map(|i| (i, &self.graph[i]))
    }

    /// Get all outgoing edges for a node.
    pub fn outgoing_edges(&self, node: NodeIndex) -> Vec<(NodeIndex, &str)> {
        self.graph
            .edges_directed(node, petgraph::Direction::Outgoing)
            .map(|e| (e.target(), e.weight().as_str()))
            .collect()
    }

    /// Get all incoming edges for a node.
    pub fn incoming_edges(&self, node: NodeIndex) -> Vec<(NodeIndex, &str)> {
        self.graph
            .edges_directed(node, petgraph::Direction::Incoming)
            .map(|e| (e.source(), e.weight().as_str()))
            .collect()
    }

    /// Get a symbol by its node index.
    pub fn get_symbol(&self, idx: NodeIndex) -> Option<&Symbol> {
        self.graph.node_weight(idx)
    }

    /// Look up a node index by symbol ID.
    pub fn find_node(&self, id: &str) -> Option<NodeIndex> {
        self.name_index.get(id).copied()
    }
}
