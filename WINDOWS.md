# Codebase Indexer — Windows Local Test

## Yang lo butuhin

Lo tinggal download **1 file** — `codebase-indexer.exe` — single binary, nggak butuh install Python atau Rust.

## Step 1: Download binary

Dari Linux server ini, binary-nya di:
```
/root/codebase-mcp-hybrid/indexer/target/x86_64-pc-windows-gnu/release/codebase-indexer.exe
```

Copy ke Windows lo. Simpan di folder yang enak, misalnya:
```
C:\Users\faizal.cahyanto-iu\tools\codebase-indexer.exe
```

## Step 2: Add to PATH (opsional, biar bisa dipanggil dari mana aja)

PowerShell (non-admin):
```powershell
[Environment]::SetEnvironmentVariable("Path", $env:Path + ";C:\Users\faizal.cahyanto-iu\tools", "User")
```

Atau tanpa PATH — panggil langsung dengan full path:
```powershell
C:\Users\faizal.cahyanto-iu\tools\codebase-indexer.exe --project C:\path\to\project
```

## Step 3: Test di project lo

```powershell
# Buka PowerShell / CMD, arahkan ke project yang mau di-index
cd C:\path\to\your\project

# Jalankan indexer
codebase-indexer --project .

# Output yang diharapkan:
# 🔍 Walking project: C:\path\to\your\project
#    Found 87 source files
#    src\main.py → 12 symbols, 8 edges
#    ...
# 📊 Extracted 423 symbols, 612 edges
# 🌐 Graph built: 423 nodes
# 💾 Persisted to C:\path\to\your\project\index.db
```

Hasilnya: `index.db` muncul di root project lo.

## Step 4: Cek isinya (optional)

Bisa dibuka pake SQLite browser (https://sqlitebrowser.org/) untuk verifikasi:
```
symbols table → semua function, class, method yang ke-extract
edges table   → call relationships
meta table    → statistik indexing
```

## Step 5: Kalau mau connect ke Codex CLI

Setelah index.db ada, baru MCP server bisa connect. Install Python + MCP di Windows:

```powershell
# Install Python 3.11+ dari python.org kalau belum ada
# Lalu:
pip install mcp

# Copy folder server dari Linux ke Windows:
# /root/codebase-mcp-hybrid/server/ → C:\Users\faizal.cahyanto-iu\tools\codebase-server\
```

Config Codex (`%USERPROFILE%\.codex\config.toml`):
```toml
[mcp_servers.codebase]
command = "python"
args = ["-m", "server"]
cwd = "C:\\Users\\faizal.cahyanto-iu\\tools\\codebase-server\\src"
env = { PROJECT_ROOT = "C:\\path\\to\\your\\project" }
```

## Catatan

- **Binary ini untuk Windows x64**. Kalau laptop lo ARM (Snapdragon), bilang gue — perlu cross-compile target lain.
- File `.exe` sekitar **15-25MB**. Single file, nggak ada dependency.
- Bisa di-copy ke project manapun. Tinggal jalanin `codebase-indexer --project .` di root project itu.

---

## Troubleshooting Windows

### "VCRUNTIME140.dll not found"
Install Visual C++ Redistributable:
```
https://aka.ms/vs/17/release/vc_redist.x64.exe
```

### "Access denied" pas jalanin .exe
Klik kanan → Properties → Unblock (kalau ada checkbox Unblock di bagian bawah)

### Anti-virus blocking
Windows Defender kadang nge-flag binary baru. Upload ke VirusTotal dulu kalau mau verifikasi.
