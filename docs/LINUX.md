# Linux support

Konnect supports KiCAD 10 on Linux for the full MCP server, schematic editing,
`kicad-cli` exports/checks, PCB IPC operations, the KiCAD executable-plugin action,
and the schematic viewer. Pull-request and weekly CI exercise the real CLI design
loop, KiCAD's demo corpus, a protocol-level Unix-socket suite, and real PCB Editor
GUI IPC under Xvfb, including footprint move/rotate geometry-preservation
regressions.

## What CI proves

The distro matrix deliberately separates portability checks from real-KiCAD tests:

| Environment | Automated coverage |
|---|---|
| Ubuntu 24.04 LTS + KiCAD 10 | Workspace tests, release builds, PCM install smoke, viewer GUI smoke, real CLI design loop and demo corpus, Unix-socket IPC, and live PCB Editor placement/move/rotate |
| Ubuntu 22.04 LTS | Workspace tests, release builds, PCM install smoke, viewer tests/build, and system `protoc` 3.12 compatibility |
| Debian 12 | Workspace tests, release builds, PCM install smoke, and viewer tests/build; also the Linux release glibc baseline |
| Fedora 44 | Workspace tests, release builds, PCM install smoke, and viewer tests/build |
| Arch Linux (rolling container) | Workspace tests, release builds, PCM install smoke, and viewer tests/build |

Windows CI independently runs the real CLI design loop, demo corpus, IPC transport
regressions, and PCM packaging. Only Ubuntu 24.04 currently runs a real Linux KiCAD
GUI session in CI; the other Linux rows prove build and package portability, not
every desktop compositor, packaging sandbox, GPU driver, or KiCAD distribution.
Release artifacts are currently built for x86-64 Linux. Other Linux architectures
may build from source but are not covered by this matrix or the release workflow.
Konnect's CI matrix is not a substitute for KiCAD's own operating-system support
policy; check [KiCAD's current Linux distribution guidance](https://www.kicad.org/download/linux-distros/)
before choosing a host distro.

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
sudo apt install build-essential cmake pkg-config protobuf-compiler libprotobuf-dev \
  python3 zip
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

Fedora's current repositories provide KiCAD. Install the application and packaged
libraries, then verify the CLI and symbol directory:

```bash
sudo dnf install kicad kicad-packages3d kicad-doc
```

If the Fedora release package is not KiCAD 10, use KiCAD's stable COPR:

```bash
sudo dnf install dnf-plugins-core
sudo dnf copr enable @kicad/kicad-stable
sudo dnf install kicad kicad-packages3d kicad-doc
```

Verify the installation:

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
before using the PCB IPC features. The Debian 12 CI row validates Konnect itself; the
real KiCAD 10 CLI and GUI E2E job runs on Ubuntu 24.04.

### Build a local PCM archive

After installing the build dependencies for your distro, plus `python3` and `zip`,
build both binaries and assemble a local PCM archive from the repository root:

```bash
cargo build --locked --release -p konnect
cargo build --locked --release --manifest-path crates/schematic-viewer/Cargo.toml
packaging/build-pcm.sh \
  --binary target/release/konnect \
  --viewer crates/schematic-viewer/target/release/schematic-viewer
```

Install the resulting `dist/konnect-pcm-linux-v<version>.zip` through KiCAD's
**Plugin and Content Manager → Install from File**, then restart KiCAD. The packaging
script preserves executable permissions and stamps the Linux `bin/konnect`
entrypoint into the bundled plugin manifest.

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
The server is a native ELF binary, not a fully static executable. The viewer also
requires GTK3, WebKitGTK 4.1, and librsvg runtime libraries from the host distro.

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
ldd /path/to/schematic-viewer
```

`konnect status` reports the bundled Claude skills/agents/hooks and whether KiCAD is
found in a standard location. It does not open a live PCB IPC connection; use
`open_project` or `check_kicad_ui` from the MCP client for that check.

If placing `Device:R` reports that the library symbol cannot be found, install your
distribution's KiCAD symbol/library package or set `KICAD10_SYMBOL_DIR` explicitly.

The official Linux release is built on Debian 12 and is gated to glibc 2.36 or older.
It uses Rustls for HTTPS and must not depend on system OpenSSL at runtime.
