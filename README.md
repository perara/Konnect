<a name="top"></a>

<div align="center">

<img src="resources/images/KiCAD-MCP-Server-rust.svg" alt="KiCAD-MCP-Server Logo" height="240" />


# Konnect *BETA Release

**AI-assisted PCB design for KiCad 10.** Konnect is a native Rust MCP server,
packaged as a KiCad plugin, that lets Claude and other AI assistants design
schematics and PCBs through the
[Model Context Protocol](https://modelcontextprotocol.io) (MCP).

**185 tools across 18 on-demand toolsets.** Schematic capture, PCB layout and
routing, ERC/DRC, design-review audits, JLCPCB part search, Freerouting, reference
circuits, and a full manufacturing export pipeline — with bundled skills and agents
that teach Claude KiCad conventions out of the box.

> **Status: beta.** The core toolchain is tested and working, but this is a young
> release and it wants real-world mileage and review. Issues and PRs are welcome —
> see [CONTRIBUTING.md](CONTRIBUTING.md).

## Why Konnect exists

Konnect is the successor to [KiCAD-MCP-Server](https://github.com/mixelpixx/KiCAD-MCP-Server),
a Python/TypeScript project that proved AI-driven PCB design works — and, in the
process, showed exactly where that architecture runs out of road. Konnect was built
to fix those specific problems:

**The call path was too long.** In the original server, a single tool call travels
through TypeScript, schema validation, a spawned Python subprocess, JSON over
stdin/stdout, a command router, and finally SWIG-generated C++ proxy objects before
anything touches your board. That's four language and serialization boundaries, each
with its own failure modes — subprocess lifecycle management, stdout parsing that
filters out warnings KiCAD leaks into the stream, chunked-JSON reassembly. In
Konnect, a tool call is a function call. One process, one language, no plumbing.

**The dependency surface was enormous.** Running the original means carrying Node.js
and its npm tree, Python and its pip packages, wxPython, kicad-skip, and KiCAD's
SWIG bindings — two package ecosystems plus a binding layer, every one of them a
moving target that can break an install. The Konnect MCP server ships as one native
binary and needs no Node/npm or separate Python/pip runtime. KiCAD, its standard
libraries, and the optional viewer's host GUI libraries are still required for the
features that use them.

**SWIG is a dead end.** The original's PCB backend depends on KiCAD's SWIG Python
bindings, which KiCAD is deprecating in favor of its IPC API. SWIG also carried
real operational scars: a zone-fill call that can segfault the backend, proxy-object
comparison bugs, and a fallback path that can silently swap backends mid-session.
Konnect talks to KiCAD 10 through the official IPC API (protobuf over NNG) — the
interface KiCAD is investing in — with real-time board edits that integrate with
KiCAD's own undo/redo.

**Schematic edits should not corrupt files.** Konnect edits `.kicad_sch` files
through its own S-expression engine with atomic writes (write, fsync, rename), UUID
preservation, and round-trip tests — no third-party schematic library with known
gaps, no text-manipulation workarounds.

**Context economy is a feature.** Exposing ~180 tools to an LLM costs roughly 23K
tokens of context on every listing. Konnect's router loads a starter kit (~2K
tokens) and lets the model pull in toolsets on demand — plus built-in observability
(`get_recent_calls`, `server_stats`, JSONL call logs) so the model can diagnose its
own tool failures.

The result is smaller, faster to install, and aligned with where KiCAD is going.
The original project remains
open, maintained, and useful — see [the comparison below](#relationship-to-kicad-mcp-server).

## What it does

Instead of describing changes and applying them by hand, the AI works your project
directly:

- **Place and wire schematic components** — add resistors, ICs, connectors; inspect
  their pins and wire them by pin number or exact endpoint
- **Lay out the PCB** — place, move, rotate, and route footprints in real time via
  KiCAD's IPC API, with full undo/redo integration
- **Run design checks** — ERC, DRC, connectivity validation, decoupling audits,
  power-rail review, BOM health checks
- **Export production files** — Gerbers, drill, BOM, pick-and-place, 3D models, PDF
- **Search JLCPCB parts** — query components in a downloaded local catalog and
  suggest alternatives
- **Start from reference circuits** — curated USB-C, LDO, buck converter, STM32,
  I2C, and LED templates; verify values against the selected parts and requirements
- **Watch it happen** — a live schematic viewer auto-refreshes as the AI edits

The full tool catalog is documented in [tool-directory.md](tool-directory.md).

## How it works

| Layer | Mechanism |
|-------|-----------|
| Schematic editing | Direct `.kicad_sch` S-expression editing with atomic writes (no KiCAD required) |
| PCB editing | KiCAD 10 IPC API (NNG + protobuf) — real-time, undo-aware, requires KiCAD running |
| Exports & checks | `kicad-cli` subprocess (Gerber, PDF, ERC, DRC, …) |
| Transport | MCP JSON-RPC over stdio (default), or Streamable HTTP (`transport = "http"` / `"both"`) |

## Installation

### From a tagged release (recommended when available)

> **Release availability:** PCM archives are created only for `v*` tags. If the
> [Releases](https://github.com/perara/Konnect/releases) page has no assets yet,
> build the current branch from source; do not substitute an older upstream archive
> when testing these Linux parity changes.

1. Download the PCM archive for your operating system from
   [Releases](https://github.com/perara/Konnect/releases):
   - Windows: `konnect-pcm-windows-v<version>.zip`
   - Linux: `konnect-pcm-linux-v<version>.zip`
   - The `.tar.gz`/standalone archives are for MCP clients that do not need the
     KiCAD toolbar integration.
2. Open KiCAD 10 → **Plugin and Content Manager**
3. Click **Install from File** and select the zip
4. Restart KiCAD

Verify: open the **PCB Editor** → **Tools → External Plugins** → you should see
**Konnect**. Enable **Edit → Preferences → Plugins → Enable KiCad API**, open
the board you want to control, and click **Konnect** once to register that PCB Editor
instance with the separately launched MCP server.

### Build from source

```bash
# protoc is required (protobuf code generation)
# Windows: choco install protoc / macOS: brew install protobuf
# Debian/Ubuntu: apt install protobuf-compiler libprotobuf-dev
cargo build --release -p konnect
```

Linux source builds also need a C/C++ toolchain, CMake, and `pkg-config`. Building
the schematic viewer requires GTK3 and WebKitGTK 4.1 development packages. See
[Linux support](docs/LINUX.md) for per-distribution commands.

## Setup with Claude Desktop

After a PCM install, the server binary lives in your KiCAD user-data folder. Typical
locations are:

```
Windows: C:\Users\<YOU>\Documents\KiCad\10.0\3rdparty\plugins\com_github_mixelpixx_konnect\bin\konnect.exe
Linux:  ~/.local/share/KiCad/10.0/3rdparty/plugins/com_github_mixelpixx_konnect/bin/konnect
```

Edit `%APPDATA%\Claude\claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "konnect": {
      "command": "C:\\Users\\<YOU>\\Documents\\KiCad\\10.0\\3rdparty\\plugins\\com_github_mixelpixx_konnect\\bin\\konnect.exe"
    }
  }
}
```

On Linux, use the same JSON shape with the absolute path to the `konnect` ELF binary
(no `.exe`). Restart the MCP client and the Konnect tools appear. For Claude Code,
drop the snippet into a `.mcp.json` in your project root. Platform-specific examples
are in [examples/](examples/).

## Schematic viewer

A standalone viewer that auto-refreshes as the schematic file changes:

```bash
# Windows
schematic-viewer.exe path\to\your\root_schematic.kicad_sch

# Linux
schematic-viewer path/to/your/root_schematic.kicad_sch
```

Point it at the root sheet of a hierarchical design and every sub-sheet is rendered
too, with a depth-indented sheet selector in the toolbar. Edits saved from KiCAD (or
made by the AI through the schematic tools) re-render only the sheets that changed
and refresh the view live — rendering runs against temp-folder snapshots, so the
viewer never blocks KiCAD from saving. Pan with click-drag, zoom with the wheel,
`0` to fit, `R` to refresh, drag-and-drop to open a different file. Also launchable
by the AI via the `open_schematic_viewer` tool.

Needs WebView2 on Windows or GTK3 + WebKitGTK 4.1 on Linux, plus a KiCAD install for
`kicad-cli` (auto-discovered, or pass `--kicad-cli <path>`). Built separately from
the main workspace — see [DEV.md](DEV.md) for build steps.

## Requirements

- KiCAD 10 on Windows or Linux
- `kicad-cli` (ships with KiCAD — used for exports, ERC, DRC)
- Standard KiCAD symbols and footprints (`kicad-library` on Arch/CachyOS;
  `kicad-symbols` and `kicad-footprints` from the official KiCAD Ubuntu PPA)
- For PCB tools: KiCAD running with the target board open (IPC API)

Linux installation, distro compatibility, Flatpak/Snap notes, and diagnostics are
documented in [docs/LINUX.md](docs/LINUX.md).

## License: free for the little guys

Konnect is licensed under the **[GNU AGPL-3.0](LICENSE)**.

Individuals and organizations may use Konnect under the AGPL at no license fee,
including for commercial PCB design. Distribution, modification, combination, and
remote-network use can create source-availability obligations. If those terms do not
fit your intended use, **commercial licenses are available**. See
[COMMERCIAL.md](COMMERCIAL.md) for a careful summary; it is not legal advice.

## Relationship to KiCAD-MCP-Server

The original [Python/TypeScript project](https://github.com/mixelpixx/KiCAD-MCP-Server)
remains fully open (MIT) and maintained. Konnect is where new development happens —
the architecture it proved, rebuilt for production:

| | KiCAD-MCP-Server | Konnect |
|---|---|---|
| Runtime | Node.js + Python + SWIG bindings | Single native Rust server process |
| Tool call path | TS → subprocess → Python → SWIG C++ | Direct function call |
| PCB backend | SWIG (deprecated by KiCAD) + experimental IPC | KiCAD 10 IPC API |
| Schematic backend | kicad-skip + custom loaders | Native S-expression engine, atomic writes |
| Context cost | Router pattern | Load/unload toolsets + observability |
| Skills / agents | — | 6 skills + 2 agents bundled |
| License | MIT | AGPL-3.0 + commercial |

## Troubleshooting

**Plugin doesn't appear in KiCAD** — install via the Plugin and Content Manager (not
manual copy), then restart KiCAD.

**PCB tools return "IPC connect failed"** — open KiCAD with your board file first;
PCB tools talk to the running PCB editor.

**"kicad-cli not found"** — common install paths are auto-detected; set `KICAD_CLI`
or set `kicad_cli` in `~/.config/konnect/config.toml` on Linux if yours is elsewhere.

## Support

- Issues & feature requests: [GitHub Issues](https://github.com/perara/Konnect/issues)
- Roadmap: [ROADMAP.md](ROADMAP.md)
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)
