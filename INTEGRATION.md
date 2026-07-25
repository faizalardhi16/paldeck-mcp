# Integration Guide — Cursor, Codex CLI, Claude Code, Hermes

Paldeck MCP is a **single binary**. No Python. No pip. No runtime dependencies. Just download and configure your agent.

## Prerequisites

```bash
# 1. Download the binary
# Windows (PowerShell):
mkdir "$env:USERPROFILE\tools" -Force
Invoke-WebRequest -Uri "https://github.com/faizalardhi16/paldeck-mcp/releases/download/v0.2.0/paldeck-mcp.exe" -OutFile "$env:USERPROFILE\tools\paldeck-mcp.exe"

# Linux/macOS (build from source):
git clone https://github.com/faizalardhi16/paldeck-mcp.git
cd paldeck-mcp
cargo build --release
sudo ln -s $(pwd)/target/release/paldeck-mcp /usr/local/bin/

# 2. Index your project
paldeck-mcp index --project /path/to/your/project

# 3. Test the server standalone
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | paldeck-mcp serve
# Should return JSON with 7 tools.
```

---

## Cursor

Config at `~/.cursor/mcp.json` (Windows: `%USERPROFILE%\.cursor\mcp.json`):

```json
{
  "mcpServers": {
    "paldeck": {
      "command": "paldeck-mcp",
      "args": ["serve"]
    }
  }
}
```

**Verification:** Restart Cursor → Settings → MCP → "paldeck" should show **Connected** with 7 tools.

---

## Codex CLI (OpenAI)

Config at `~/.codex/config.toml` (Windows: `%USERPROFILE%\.codex\config.toml`):

```toml
[mcp_servers.paldeck]
command = "paldeck-mcp"
args = ["serve"]
```

**Verification:**
```bash
codex mcp list
# paldeck: 7 tools

codex mcp call paldeck search_symbols '{"query": "validate_email"}'
```

---

## Claude Code (Anthropic)

Config at `~/.claude/mcp.json` (Windows: `%USERPROFILE%\.claude\mcp.json`):

```json
{
  "mcpServers": {
    "paldeck": {
      "command": "paldeck-mcp",
      "args": ["serve"]
    }
  }
}
```

> **Note:** Claude Code also reads `~/.claude.json` → key `mcpServers` (legacy format). Prefer `~/.claude/mcp.json` for the newer path.

**Verification:**
```bash
claude mcp list
# paldeck: 7 tools
```

---

## Hermes Agent

Config at `~/.hermes/config.yaml`:

```yaml
mcp_servers:
  paldeck:
    command: "paldeck-mcp"
    args: ["serve"]
    timeout: 30
```

**Verification:** Restart Hermes → startup log should show:
```
[MCP] Connected to 'paldeck': 7 tools discovered
[MCP] Registered mcp_paldeck_search_symbols
[MCP] Registered mcp_paldeck_find_callers
...
```

---

## Per-Project Setup Pattern

Run the indexer in any project directory:

```bash
cd /path/to/any-project
paldeck-mcp index --project .
# Creates: ./index.db
```

When `paldeck-mcp serve` runs, it auto-discovers `index.db` in the current working directory. This means you can keep the same agent config and just change directories:

```bash
cd ~/projects/api-server
paldeck-mcp index --project .
paldeck-mcp serve     # serves api-server's index.db

cd ~/projects/frontend
paldeck-mcp index --project .
paldeck-mcp serve     # serves frontend's index.db
```

No config changes needed — the server reads `./index.db` by default.

---

## Troubleshooting

### "paldeck-mcp: command not found"

The binary isn't on PATH. Use the full path in your MCP config:

```json
{"command": "C:\\Users\\faizal.cahyanto-iu\\tools\\paldeck-mcp.exe", "args": ["serve"]}
```

or

```json
{"command": "/usr/local/bin/paldeck-mcp", "args": ["serve"]}
```

### "No index found at index.db"

Run the indexer first:
```bash
paldeck-mcp index --project /path/to/project
```

### Agent shows "Disconnected" or tools don't appear

1. Test the server standalone:
   ```bash
   echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | paldeck-mcp serve
   ```
2. Should return a JSON list of 7 tools.
3. If it hangs or errors, check that `index.db` exists in the current directory (or pass `--db` flag).

### Cursor: MCP server stuck on "Disconnected"

1. Open Cursor → Help → Toggle Developer Tools
2. Check the Console tab for MCP-related errors
3. Common fix: use the full path to the binary in the config.

### Windows: "VCRUNTIME140.dll not found"

Install Visual C++ Redistributable:
```
https://aka.ms/vs/17/release/vc_redist.x64.exe
```

### Windows: "Access denied" on .exe

Right-click `paldeck-mcp.exe` → Properties → Unblock (if the checkbox is visible).

### Windows: Anti-virus blocking

Windows Defender sometimes flags new binaries. Upload to VirusTotal for verification, or add an exclusion for the tools folder.
