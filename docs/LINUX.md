# Linux support

Konnect supports KiCAD 10 on Linux for the full MCP server, schematic editing,
`kicad-cli` exports/checks, PCB IPC operations, the KiCAD executable-plugin action,
and the schematic viewer. Weekly CI exercises the real CLI design loop, KiCAD's
demo corpus, a protocol-level Unix-socket suite, and real PCB Editor GUI IPC under
Xvfb, including footprint move/rotate geometry-preservation regressions.

## Recommended installation

Use a native KiCAD package where possible. Native packages expose `kicad-cli`, the
standard symbol libraries, and KiCAD's IPC Unix socket directly to Konnect.

### Ubuntu 24.04 and 22.04

KiCAD recommends its stable PPA because Ubuntu's base repository is older:

```bash
sudo add-apt-repository ppa:kicad/kicad-10.0-releases
sudo apt update
sudo apt install kicad kicad-demos kicad-symbols kicad-footprints
```

For a source build:

```bash
sudo apt install build-essential cmake pkg-config protobuf-compiler libprotobuf-dev
```

Building the viewer additionally requires:

```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev librsvg2-dev
```

### Arch Linux and CachyOS

```bash
sudo pacman -Syu kicad kicad-library
# Optional, but used by Konnect's conformance suite:
sudo pacman -S kicad-demos
```

For a source build:

```bash
sudo pacman -S base-devel cmake pkgconf protobuf gtk3 librsvg webkit2gtk-4.1
```

### Fedora

Install KiCAD 10 using Fedora's current packages or the KiCAD-supported COPR, then
verify both commands and the symbol directory:

```bash
kicad-cli --version
test -d /usr/share/kicad/symbols
```

For a source build:

```bash
sudo dnf install gcc gcc-c++ make cmake pkgconf-pkg-config protobuf-compiler \
  protobuf-devel gtk3-devel librsvg2-devel webkit2gtk4.1-devel
```

### Debian

Konnect builds and runs on Debian 12's glibc baseline, but Debian stable may ship an
older KiCAD major version. Install KiCAD 10 from a KiCAD-provided distribution method
before using the PCB IPC features.

## PCM and standalone layouts

The Linux PCM archive includes:

```text
plugins/bin/konnect
plugins/bin/schematic-viewer
plugins/plugin.json
plugins/__init__.py
plugins/settings_dialog.py
```

The executable IPC entrypoint is `bin/konnect`. PCM preserves executable permissions.
KiCAD normally installs IPC plugins below
`~/.local/share/KiCad/10.0/3rdparty/plugins/`.

The standalone Linux archive contains both `konnect` and `schematic-viewer`; keep them
in the same directory so the `open_schematic_viewer` MCP tool can locate the viewer.

## Configuration and discovery

Konnect searches:

- `kicad-cli` and `kicad` on `PATH`;
- `/usr/bin` and `/usr/local/bin`;
- the KiCAD Snap command and library layouts;
- system and per-user Flatpak library export layouts;
- `KICAD_CLI`, `KICAD_BINARY`, and `KICAD10_SYMBOL_DIR` overrides;
- `$XDG_CONFIG_HOME` (falling back to `~/.config`) for configuration.

Linux user files follow the XDG base-directory convention:

| Purpose | Default path |
|---|---|
| Server and user configuration | `~/.config/konnect/` |
| Templates, downloaded databases, install marker | `~/.local/share/konnect/` |
| Call logs | `~/.local/state/konnect/` |
| KiCAD IPC discovery and plugin runtime files | `~/.cache/konnect/` |

When KiCAD launches the PCM executable plugin it supplies `KICAD_API_SOCKET` and an
optional API token. Click **Tools → External Plugins → Konnect** in the PCB Editor
whose board you want to control. Konnect stores that instance's socket/token in a
private `0600` cache record; the next separately launched MCP process restores it.
With multiple PCB Editor processes, clicking the action selects the latest target.
An explicitly supplied `KICAD_API_SOCKET` always takes precedence.

Example standalone MCP configuration:

```json
{
  "mcpServers": {
    "konnect": {
      "command": "/absolute/path/to/konnect",
      "env": {
        "RUST_LOG": "info"
      }
    }
  }
}
```

## Snap and Flatpak

Snap installations are discovered through `/snap/bin/kicad-cli` and the mounted Snap
library directory. If confinement hides your project, grant KiCAD access to the
directory containing it.

Flatpak installations isolate `kicad-cli` and the IPC socket. The most reliable setup
is a native KiCAD package. If Flatpak is required, create a host wrapper and configure
Konnect to use it:

```bash
#!/bin/sh
exec flatpak run --command=kicad-cli org.kicad.KiCad "$@"
```

Set `kicad_cli` to the wrapper's absolute path. A Konnect process launched by KiCAD's
PCM integration inherits the correct `KICAD_API_SOCKET`; a separately launched host
process may need access to the Flatpak runtime socket and is not guaranteed to cross
the sandbox boundary. This sandbox boundary is a Flatpak confinement limitation, not
a native Konnect/KiCAD limitation.

## Diagnostics

```bash
konnect status
kicad-cli --version
test -d /usr/share/kicad/symbols || echo "KiCAD symbol library missing"
ldd /path/to/konnect
```

If placing `Device:R` reports that the library symbol cannot be found, install your
distribution's KiCAD symbol/library package or set `KICAD10_SYMBOL_DIR` explicitly.

The official Linux release is built on Debian 12 and is gated to glibc 2.36 or older.
It uses Rustls for HTTPS and must not depend on system OpenSSL at runtime.
