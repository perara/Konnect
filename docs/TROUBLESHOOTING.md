# Troubleshooting

## "KiCAD IPC socket path not configured"

Any tool that talks to a live KiCAD session (`save_project`, PCB editing,
`check_kicad_ui`, …) needs the IPC socket address. The PCM executable action now
registers this automatically; the AI client still needs to launch the Konnect MCP
binary.

Step by step (based on the diagnostic guide contributed in
[#18](https://github.com/mixelpixx/Konnect/issues/18)):

1. Open KiCAD normally.
2. Go to **Edit → Preferences → Plugins** and check **"Enable KiCad API"**.
   Confirm a line like this appears:

   ```
   Listening on ipc://C:\Users\<you>\AppData\Local\Temp\kicad\api.sock
   ```

3. Open the target board in PCB Editor, then click
   **Tools → External Plugins → Konnect**. This records the socket and optional
   API token in Konnect's private per-user cache. If several PCB Editor processes
   are running, click this action in the process you want to control.
4. Confirm your AI client (Claude Code, Claude Desktop, …) has the `konnect`
   MCP server registered in its own config (`.mcp.json` or
   `claude_desktop_config.json`) pointing at the `konnect` binary — see
   [examples/](../examples/). This registration is separate from the KiCAD
   action.
5. Restart the AI client session so it spawns a fresh Konnect process and
   restores the registered instance.
6. Verify: have the AI call `open_project`. Expected:

   ```json
   { "kicad_ui_running": true, "message": "KiCAD is running and IPC is available." }
   ```

Alternative: set `KICAD_API_SOCKET` (and `KICAD_API_TOKEN` when required) in the
MCP process environment. These explicit values take precedence over discovery.
The optional **Konnect Settings** Python action can also start a local Streamable
HTTP endpoint at `http://127.0.0.1:3000/mcp`.

## PCB tools return "IPC connect failed" / "No PCB document is open"

The IPC tools talk to KiCAD's **running PCB editor**. Open your board file in
KiCAD first, and make sure the API is enabled (previous section).

## "kicad-cli not found"

Common install paths are auto-detected (including the Windows registry). If
your install is somewhere unusual, set `KICAD_CLI` or configure `kicad_cli` in
the server config file. On Linux the default is
`$XDG_CONFIG_HOME/konnect/config.toml` (normally `~/.config/konnect/config.toml`).

On Linux, verify that the native CLI and standard symbol libraries are installed:

```bash
kicad-cli --version
test -d /usr/share/kicad/symbols
```

Arch/CachyOS users need the separate `kicad-library` package. Ubuntu users of the
official KiCAD PPA need `kicad-symbols` and `kicad-footprints`. See
[Linux support](LINUX.md).

## Plugin doesn't appear in KiCAD

Install via **Plugin and Content Manager → Install from File** with the
`konnect-pcm-*.zip` release asset (not the bare binary archives), then restart
KiCAD.

If the fork's Releases page has no tagged assets yet, building the branch produces
the server binary but not an installable PCM archive. Assemble one with
`packaging/build-pcm.sh` as documented in [Linux support](LINUX.md) before using
**Install from File**.

## Viewer fails to start on Linux

Check its dynamic libraries first:

```bash
ldd /path/to/schematic-viewer | grep 'not found'
```

Install GTK3, WebKitGTK 4.1, and librsvg runtime packages for your distribution.
If the viewer starts but KiCAD itself has display glitches under Wayland, reproduce
under an X11 session; [KiCAD's Linux guidance](https://www.kicad.org/download/linux-distros/)
currently recommends X11 for GUI issue diagnosis.
