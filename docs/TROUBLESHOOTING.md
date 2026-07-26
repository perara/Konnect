# Troubleshooting

## "KiCAD IPC socket path not configured"

Any tool that talks to a live KiCAD session (`save_project`, PCB editing,
`check_kicad_ui`, …) needs the IPC socket address. Two separate configurations
must both be correct — neither happens automatically:

1. **The socket path in Konnect's plugin settings** (inside KiCAD)
2. **The Konnect server registration in your AI client's MCP config**

Step by step (based on the diagnostic guide contributed in
[#18](https://github.com/mixelpixx/Konnect/issues/18)):

1. Open KiCAD normally.
2. Go to **Edit → Preferences → Plugins** and check **"Enable KiCad API"**.
   Confirm a line like this appears:

   ```
   Listening on ipc://C:\Users\<you>\AppData\Local\Temp\kicad\api.sock
   ```

   Copy the whole address including the `ipc://` prefix — it is unique to
   your machine and user.
3. In KiCAD, open **Tools → External Plugins → Konnect** to open the settings
   dialog.
4. Paste the address into the **IPC Socket** field and click **Save**.
5. Confirm your AI client (Claude Code, Claude Desktop, …) has the `konnect`
   MCP server registered in its own config (`.mcp.json` or
   `claude_desktop_config.json`) pointing at the `konnect` binary — see
   [examples/](../examples/). This registration is separate from the KiCAD
   plugin settings.
6. Restart the AI client session so it spawns a fresh Konnect process that
   reads the saved settings.
7. Verify: have the AI call `open_project`. Expected:

   ```json
   { "kicad_ui_running": true, "message": "KiCAD is running and IPC is available." }
   ```

Alternative: launching the server from within KiCAD sets `KICAD_API_SOCKET`
automatically, and a `konnect-settings.json` passed via `--config` can carry
`ipc_socket_path` directly.

## PCB tools return "IPC connect failed" / "No PCB document is open"

The IPC tools talk to KiCAD's **running PCB editor**. Open your board file in
KiCAD first, and make sure the API is enabled (previous section).

## "kicad-cli not found"

Common install paths are auto-detected (including the Windows registry). If
your install is somewhere unusual, set the path in the plugin settings dialog
or in `konnect-settings.json` (`kicad_cli`).

## Transaction recovery is blocked by divergent content

Multi-file schematic changes persist a `.konnect-transaction-<id>.json`
write-ahead journal in the project before changing any target. On restart,
Konnect safely completes files that still match either the recorded before
image or intended replacement. It never overwrites a file changed by KiCad or
another process after the journal was written.

Inspect active journals without printing their contents:

```text
konnect transaction status <project-dir>
```

Each target is reported as `pending`, `applied`, or `divergent`. Retry safe
recovery with:

```text
konnect transaction recover <project-dir>
```

If a target is divergent, first inspect the schematic in KiCad and preserve
the version you want. To unblock future transactions without changing any
schematic file, explicitly abandon the journal:

```text
konnect transaction abandon <project-dir> <transaction-id> --force
```

Abandonment renames the journal to
`.konnect-transaction-<id>.abandoned.json`; it does not restore, replace, or
delete a target. The abandoned file is retained as recovery evidence and is
ignored by future transactions. Delete it only after you have made any backup
you need.

Active and abandoned journals contain complete before/after images of every
affected schematic. Treat them as sensitive, do not attach them to bug reports
without reviewing their contents, and do not commit them. Both forms are
ignored by the repository `.gitignore`.

Cooperative document locks are stored outside the project under the platform
local-data directory. Set `KONNECT_STATE_DIR` to an absolute directory to
override that location. A relative override is rejected rather than falling
back to project-local sidecars.

## Tools don't appear after `load_toolset`

After a successful `load_toolset` call the server sends a
`notifications/tools/list_changed` notification, and MCP clients are expected to
re-fetch `tools/list` in response. If newly loaded tools never show up:

1. Check your client honors `notifications/tools/list_changed` (most current MCP
   clients do; some cache the initial tool list forever).
2. Disable any competing tool-search or tool-filter layer sitting between the
   model and the server. A Chrome-extension "tool search" that shadowed the real
   tool list caused exactly this in
   [#67](https://github.com/mixelpixx/Konnect/issues/67).
3. Re-issue `tools/list` (e.g. restart the client session) — the loaded toolset
   state lives in the server process and survives a list refresh.

## Plugin doesn't appear in KiCAD

Install via **Plugin and Content Manager → Install from File** with the
`konnect-pcm-*.zip` release asset (not the bare binary archives), then restart
KiCAD.

## Viewer fails to start on Linux

First confirm which renderer was compiled. The default compatibility renderer uses
Tauri/WebKitGTK and `kicad-cli`; the opt-in native renderer uses winit/Vello and does
not load WebKitGTK:

```bash
cd crates/schematic-viewer
cargo build --release                                  # compatibility
cargo build --release --no-default-features --features renderer-vello
```

### Compatibility renderer

Check its dynamic libraries first:

```bash
ldd /path/to/schematic-viewer | grep 'not found'
```

Install GTK3, WebKitGTK 4.1, and librsvg runtime packages for your distribution.

The viewer supports both native Wayland and X11 and normally selects the active
desktop backend automatically. It also disables WebKitGTK's DMA-BUF renderer before
GTK starts because that path can produce a blank surface or Wayland protocol error
on some GPU/driver combinations. To diagnose a compositor-specific problem, force a
backend explicitly:

```bash
GDK_BACKEND=wayland schematic-viewer path/to/design.kicad_sch
GDK_BACKEND=x11 schematic-viewer path/to/design.kicad_sch
```

An explicitly supplied `WEBKIT_DISABLE_DMABUF_RENDERER` value overrides Konnect's
safe default. Setting it to `0` is useful only when testing whether an updated
WebKitGTK/driver stack has fixed its DMA-BUF path.

### Native Vello renderer

The native renderer selects Wayland or X11 through winit and selects Vulkan, Metal,
Direct3D 12, or another supported graphics backend through wgpu. On Linux, force the
window backend only when diagnosing a compositor problem:

```bash
WINIT_UNIX_BACKEND=wayland schematic-viewer path/to/design.kicad_sch
WINIT_UNIX_BACKEND=x11 schematic-viewer path/to/design.kicad_sch
```

Set `RUST_LOG=wgpu_core=info,wgpu_hal=info` before launching to inspect adapter and
surface selection. The native feature currently requires a usable GPU adapter; use
the default compatibility renderer on systems where wgpu cannot create a surface.

For a renderer mismatch that is difficult to judge by eye, create a KiCad/Vello
pixel diff:

```bash
scripts/compare-schematic-renderers.sh path/to/sheet.kicad_sch /tmp/render-diff.png
scripts/compare-schematic-project.sh path/to/kicad-project
KONNECT_VELLO_SVG_ORACLE=1 scripts/compare-schematic-renderers.sh path/to/sheet.kicad_sch
scripts/test-schematic-renderer-goldens.sh
```

This path is headless and does not use WebKitGTK, but it does require `kicad-cli`,
`rsvg-convert`, and ImageMagick because they produce and compare the reference image.
