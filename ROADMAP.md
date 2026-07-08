# Roadmap

The near-term direction for Konnect. No dates — items ship when they're solid.
Opening an issue is the best way to influence priority.

## Platform

- **Linux and macOS builds.** The code already carries `#[cfg]` branches and Unix
  paths for both platforms, and CI checks all three OSes — what remains is release
  packaging, per-platform QA against a running KiCAD, and macOS code signing /
  notarization.
- **KiCAD PCM publication** — submit the plugin to the official KiCAD addon
  repository once the first tagged release is out.

## Tools

- **`import_svg_logo`** — import an SVG file as silkscreen / copper artwork
  (path parsing + polygon tessellation, placed via the IPC API).
- **Hierarchical sheets** — create and manage multi-sheet schematics
  (hierarchical sheets, sheet pins, cross-sheet nets).
- **Symbol & footprint creation** — author new library parts from scratch, not
  just search and place existing ones.
- **Eagle project import** — migrate legacy Eagle designs.
- **Multi-sheet schematic viewer** — `kicad-cli sch export svg` emits one SVG
  per sheet; the live viewer currently shows only the root sheet. Add a sheet
  selector for hierarchical designs.

## Infrastructure

- **Retry/backoff for external services** (JLCPCB catalog, datasheet fetches).
- **Component search caching** for repeated queries against the local parts DB.
- **Deeper end-to-end tests** — tool-handler tests against a mocked IPC endpoint.

## Done

- ~~HTTP transport~~ — Streamable HTTP (MCP spec 2025-06-18) available via
  `transport = "http"` (or `"both"`): POST + GET (SSE) on a single `/mcp`
  endpoint, Origin validation, and a `/health` probe.
- ~~Additional export formats~~ — IPC-2581, ODB++, GenCAD, and DXF are now
  available via `export_ipc2581`, `export_odb`, `export_gencad`, and
  `export_dxf` in the `pcb_export` toolset (all backed by native `kicad-cli`
  subcommands, verified against KiCAD 10.0).
