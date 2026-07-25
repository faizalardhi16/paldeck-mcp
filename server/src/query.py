"""
Graph enrichment layer using NetworkX (optional).

If networkx is installed, this module loads the SQLite index
into a NetworkX DiGraph for advanced queries:
  - Shortest call path between two symbols
  - Cycle detection (circular dependencies)
  - Subgraph extraction

Fallback: pure SQL queries in server.py cover the basics.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Optional

try:
    import networkx as nx
    HAS_NETWORKX = True
except ImportError:
    HAS_NETWORKX = False


class GraphEnricher:
    """Optional NetworkX-powered advanced graph queries."""

    def __init__(self, conn: sqlite3.Connection):
        if not HAS_NETWORKX:
            raise RuntimeError("networkx is required for GraphEnricher. Install: pip install networkx")
        self.graph = nx.DiGraph()
        self._load(conn)

    def _load(self, conn: sqlite3.Connection):
        """Load all symbols as nodes and all edges from SQLite."""
        # Nodes
        for row in conn.execute("SELECT id, name, kind, file_path, line_start FROM symbols"):
            self.graph.add_node(row["id"], **dict(row))

        # Edges
        for row in conn.execute("SELECT source_id, target_id, kind FROM edges"):
            self.graph.add_edge(row["source_id"], row["target_id"], kind=row["kind"])

    def shortest_call_path(self, from_id: str, to_id: str) -> Optional[list[str]]:
        """Find the shortest chain of function calls from A to B."""
        try:
            path = nx.shortest_path(self.graph, source=from_id, target=to_id)
            return path
        except (nx.NetworkXNoPath, nx.NodeNotFound):
            return None

    def find_cycles(self) -> list[list[str]]:
        """Detect circular call dependencies."""
        try:
            return list(nx.simple_cycles(self.graph))
        except nx.NetworkXError:
            return []

    def call_chain_depth(self, from_id: str, max_depth: int = 10) -> dict:
        """BFS from a symbol — all reachable callees with depth."""
        paths = nx.single_source_shortest_path_length(
            self.graph, from_id, cutoff=max_depth
        )
        return paths

    def top_symbols_by_calls(self, limit: int = 20) -> list[dict]:
        """Most-called symbols (by in-degree)."""
        in_deg = sorted(self.graph.in_degree(), key=lambda x: x[1], reverse=True)
        result = []
        for node_id, count in in_deg[:limit]:
            node = self.graph.nodes[node_id]
            result.append({
                "id": node_id,
                "name": node.get("name", ""),
                "kind": node.get("kind", ""),
                "file_path": node.get("file_path", ""),
                "callers": count,
            })
        return result


def load_graph(conn: sqlite3.Connection) -> Optional[GraphEnricher]:
    """Try to load the NetworkX graph; return None if networkx not installed."""
    if not HAS_NETWORKX:
        return None
    return GraphEnricher(conn)
