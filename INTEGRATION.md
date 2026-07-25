# Integration Guide — Cursor, Codex CLI, Claude Code, Hermes

## Binary Situation: This vs CBM

**Short answer:** Rust indexer ✅ single binary. MCP server ❌ needs Python.

```
codebase-indexer (Rust)         →  single static binary ~15-30MB  ✓ seperti CBM
codebase-mcp-server (Python)    →  butuh Python 3.11+ + mcp       ✗ tidak seperti CBM
```

| | CBM | This (Hybrid) |
|---|---|---|
| **Indexer** | C binary (100-150MB) | Rust binary (15-30MB) |
| **MCP server** | Embedded in binary (C + libmicrohttpd?) | Python (`mcp` package) |
| **Runtime deps** | Zero | Python 3.11+ + `pip install mcp` |
| **Single file?** | ✅ Ya | ❌ Dua komponen |

### Kenapa nggak satu binary?

Karena trade-off yang lo pilih: **indexer di Rust** (performa parsing) + **server di Python** (iterasi cepat buat tools). SQLite jadi jembatannya.

### Opsi untuk jadi single binary

**Opsi A: Rewrite MCP server di Rust**
```toml
# indexer/Cargo.toml — tambahin
rmcp = "0.3"   # Rust MCP SDK
```
Pindahin semua `server.py` → Rust module. 1 binary, 2 commands:
```bash
codebase-mcp index  --project /repo    # indexing
codebase-mcp serve                     # MCP server (reads index.db)
```
Pro: single binary, zero runtime deps. Cons: 1-2 minggu rewrite, iterasi tool lebih lambat.

**Opsi B: Bundle Python dengan Nuitka**
```bash
pip install nuitka
cd server
python -m nuitka --standalone --onefile src/server.py -o codebase-mcp
# Output: codebase-mcp (single binary, ~50-80MB, no Python needed)
```
Pro: tetep Python codebase, jadi single binary. Cons: compile time ~5 menit, binary gede.

**Opsi C: Hybrid (sekarang)** — Build indexer binary, keep Python server.
```bash
# Yang lo kirim ke user:
codebase-mcp-hybrid/
├── codebase-indexer        # static binary
├── server/                 # Python source
└── install.sh              # pip install mcp && cargo build
```

Rekomendasi: jalanin opsi A kalau udah stabil (tools udah final, nggak sering berubah). Opsi C buat development/iterasi.

---

## Prerequisites

Semua integrasi butuh ini dulu:

```bash
# 1. Build Rust indexer
cd codebase-mcp-hybrid/indexer
cargo build --release
sudo ln -s $(pwd)/target/release/codebase-indexer /usr/local/bin/

# 2. Install Python deps
cd codebase-mcp-hybrid/server
pip install mcp

# 3. Index project lo
codebase-indexer --project /path/to/your/project

# 4. Test MCP server jalan
cd server
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | uv run python -m server
```

---

## Cursor

Cursor baca MCP config dari `~/.cursor/mcp.json`.

### Config

```json
{
  "mcpServers": {
    "codebase": {
      "command": "uv",
      "args": [
        "run",
        "--directory",
        "/root/codebase-mcp-hybrid/server",
        "codebase-mcp"
      ],
      "env": {
        "PROJECT_ROOT": "/path/to/your/project"
      }
    }
  }
}
```

### Verifikasi

Restart Cursor → buka Settings → MCP → "codebase" harusnya muncul dengan status **Connected** + 7 tools listed.

Di chat Cursor, test:
```
search_symbol("validate_email", kind="function")
```

---

## Codex CLI (OpenAI)

Codex CLI baca MCP config dari `~/.codex/config.toml`.

### Config

```toml
[mcp_servers.codebase]
command = "uv"
args = [
    "run",
    "--directory",
    "/root/codebase-mcp-hybrid/server",
    "codebase-mcp"
]
env = { PROJECT_ROOT = "/path/to/your/project" }
```

### Verifikasi

```bash
codex mcp list
# Harusnya muncul: codebase (7 tools)

codex mcp call codebase search_symbol '{"query": "validate_email"}'
```

---

## Claude Code (Anthropic)

Claude Code baca MCP config dari salah satu:
- `~/.claude/mcp.json` (versi baru, rekomendasi)
- `~/.claude.json` → key `mcpServers` (versi lama, fallback)

### Config (`~/.claude/mcp.json`)

```json
{
  "mcpServers": {
    "codebase": {
      "command": "uv",
      "args": [
        "run",
        "--directory",
        "/root/codebase-mcp-hybrid/server",
        "codebase-mcp"
      ],
      "env": {
        "PROJECT_ROOT": "/path/to/your/project"
      }
    }
  }
}
```

### Config alternatif: tanpa `uv` (kalau udah install via pip)

```json
{
  "mcpServers": {
    "codebase": {
      "command": "python",
      "args": [
        "-m",
        "server"
      ],
      "cwd": "/root/codebase-mcp-hybrid/server/src",
      "env": {
        "PROJECT_ROOT": "/path/to/your/project"
      }
    }
  }
}
```

### Verifikasi

```bash
claude mcp list
# codebase: 7 tools
```

Di Claude Code session:
```
> Find where email validation happens in this project
```

Claude akan otomatis manggil `mcp__codebase__search_symbol` sebelum buka file satu-satu.

---

## Hermes Agent (built-in)

Hermes punya MCP client native. Config di `~/.hermes/config.yaml`.

### Config

```yaml
mcp_servers:
  codebase:
    command: "uv"
    args:
      - "run"
      - "--directory"
      - "/root/codebase-mcp-hybrid/server"
      - "codebase-mcp"
    timeout: 30
    env:
      PROJECT_ROOT: "/path/to/your/project"
```

### Kalau udah dibundle jadi single binary (Opsi B — Nuitka)

```yaml
mcp_servers:
  codebase:
    command: "/usr/local/bin/codebase-mcp"
    args: []
    timeout: 30
```

### Verifikasi

Restart Hermes → startup log harusnya muncul:
```
[MCP] Connected to 'codebase': 7 tools discovered
[MCP] Registered mcp_codebase_search_symbol
[MCP] Registered mcp_codebase_trace_callers
...
```

Test:
```
> search_symbol query="createUser"
```

---

## Per-Project Config Pattern

Biar nggak hardcode `PROJECT_ROOT`, lo bisa bikin wrapper script per project:

```bash
# ~/projects/my-api/.codebase-mcp.sh
#!/bin/bash
export PROJECT_ROOT="$(dirname "$0")"
exec uv run --directory /root/codebase-mcp-hybrid/server codebase-mcp
```

Lalu di semua agent config:

```json
{
  "mcpServers": {
    "codebase": {
      "command": "/home/faizal/projects/my-api/.codebase-mcp.sh"
    }
  }
}
```

Atau — approach yang lebih clean — bikin `codebase-indexer` jalan di repo root, dan MCP server auto-detect `index.db` di current directory:

```bash
# Di root project manapun:
cd /path/to/project
codebase-indexer --project .     # creates ./index.db
codebase-mcp                     # auto-discovers ./index.db
```

Server udah support ini — `discover_db()` di `server.py` akan nyari `index.db` di working directory.

---

## Troubleshooting

### "codebase-indexer: command not found"

```bash
which codebase-indexer
# Kalau kosong:
cd codebase-mcp-hybrid/indexer
cargo build --release
sudo ln -s $(pwd)/target/release/codebase-indexer /usr/local/bin/
```

### "No module named 'mcp'"

```bash
pip install mcp
# atau
uv pip install mcp
```

### "No index found at index.db"

Indexer belum dijalanin. Jalankan dulu:
```bash
codebase-indexer --project /path/to/project
```

### "Connection refused" atau tool nggak muncul

1. Cek MCP server bisa jalan standalone:
   ```bash
   echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | uv run --directory server codebase-mcp
   ```
2. Harusnya return JSON list of 7 tools.
3. Kalau hang — kemungkinan `mcp` package version mismatch. Coba `pip install --upgrade mcp`.

### Cursor: MCP server status "Disconnected" terus

1. Buka Cursor → Help → Toggle Developer Tools
2. Console tab → cari error terkait MCP
3. Biasanya: Python path nggak ketemu. Coba full path:
   ```json
   "command": "/home/faizal/.local/bin/uv"
   ```
