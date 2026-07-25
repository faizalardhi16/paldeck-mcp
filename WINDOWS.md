# Paldeck MCP — Windows Setup

**1 file. 9MB. Zero dependencies.**

No Python, no Rust toolchain, no pip install. Just download `paldeck-mcp.exe` and go.

## Step 1: Download

```powershell
# Create tools directory
mkdir "$env:USERPROFILE\tools" -Force

# Download the binary
Invoke-WebRequest -Uri "https://github.com/faizalardhi16/paldeck-mcp/releases/download/v0.2.0/paldeck-mcp.exe" -OutFile "$env:USERPROFILE\tools\paldeck-mcp.exe"
```

## Step 2: Add to PATH (optional)

```powershell
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:USERPROFILE\tools", "User")
```

Restart your terminal, then:

```powershell
paldeck-mcp --help
```

Or skip PATH and use the full path:
```powershell
C:\Users\faizal.cahyanto-iu\tools\paldeck-mcp.exe index --project .
```

## Step 3: Index your project

```powershell
cd C:\path\to\your\project
paldeck-mcp index --project .

# Expected output:
# 🔍 Walking project: C:\path\to\your\project
#    Found 87 source files
#    src\main.py → 12 symbols, 8 edges
#    ...
# 📊 Extracted 423 symbols, 612 edges
# 🌐 Graph built: 423 nodes
# 💾 Persisted to C:\path\to\your\project\index.db
```

Result: `index.db` appears in your project root.

## Step 4: Connect to your AI agent

Add this to your MCP config:

```json
{"mcpServers": {"paldeck": {"command": "paldeck-mcp", "args": ["serve"]}}}
```

Or with full path:
```json
{"mcpServers": {"paldeck": {"command": "C:\\Users\\faizal.cahyanto-iu\\tools\\paldeck-mcp.exe", "args": ["serve"]}}}
```

Works with **Cursor**, **Codex CLI**, and **Claude Code**. See [`INTEGRATION.md`](INTEGRATION.md) for platform-specific config locations.

## Step 5: Verify (optional)

Open `index.db` with [DB Browser for SQLite](https://sqlitebrowser.org/) to inspect:

| Table | Contents |
|-------|----------|
| `symbols` | All functions, classes, methods extracted |
| `edges` | Call relationships between symbols |
| `meta` | Indexing stats (file count, symbol count, timestamp) |

---

## Troubleshooting

### "VCRUNTIME140.dll not found"

Install Visual C++ Redistributable:
```
https://aka.ms/vs/17/release/vc_redist.x64.exe
```

### "Access denied" when running .exe

Right-click `paldeck-mcp.exe` → Properties → **Unblock** (checkbox at the bottom of the General tab).

### Windows Defender / SmartScreen blocks the binary

This is expected for a new, unsigned binary. Either:
- Click "More info" → "Run anyway" when the SmartScreen popup appears
- Add `C:\Users\<you>\tools` as a Defender exclusion
- Upload to [VirusTotal](https://www.virustotal.com) to verify

### Binary doesn't run on ARM (Snapdragon) laptops

The current build targets `x86_64` (Intel/AMD). ARM64 builds can be added — file an issue or build from source:

```bash
git clone https://github.com/faizalardhi16/paldeck-mcp.git
cd paldeck-mcp
cargo build --release --target aarch64-pc-windows-msvc
```
