# Linux viewer development

Konnect has two separately compiled schematic viewers. The default
`renderer-kicad-cli` build uses Tauri 2, WebKitGTK 4.1, and KiCad SVG exports.
The opt-in `renderer-vello` build uses winit, Vello, and wgpu and does not need
WebKitGTK or KiCad at runtime.

## Compatibility-viewer dependencies

Install the Tauri 2 Linux prerequisites for your distribution. On
Debian/Ubuntu:

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

On Arch Linux:

```bash
sudo pacman -Syu
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl \
  appmenu-gtk-module libappindicator-gtk3 librsvg xdotool
```

On Fedora:

```bash
sudo dnf check-update
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
  libappindicator-gtk3-devel librsvg2-devel libxdo-devel
sudo dnf group install "c-development"
```

These package lists follow the official [Tauri 2 Linux
prerequisites](https://v2.tauri.app/start/prerequisites/). The current Linux
development host was compile-checked with GTK 3.24.52, WebKitGTK 4.1 version
2.52.5, and librsvg 2.62.3; those host versions are evidence for that build,
not project-wide minimum-version promises.

## Native Vello prerequisites

Install the Rust toolchain selected by the repository's `rust-toolchain.toml`
and the development libraries required by winit's Wayland and X11 backends. A
usable wgpu graphics adapter is required for the interactive window. Headless
golden comparisons additionally need `kicad-cli`, `rsvg-convert`, and
ImageMagick because those tools create and compare the reference image.

## KiCad discovery

The compatibility viewer resolves KiCad in this order:

1. `--kicad-cli /absolute/path/to/kicad-cli`;
2. the `KICAD_CLI` environment variable;
3. common platform installation paths;
4. `kicad-cli` on `PATH`.

Use an explicit path when testing against a particular KiCad build:

```bash
cargo run --manifest-path crates/schematic-viewer/Cargo.toml -- \
  --kicad-cli /usr/bin/kicad-cli path/to/design.kicad_sch
```

## Build and test

Run from the repository root:

```bash
# Default compatibility viewer
cargo check --locked --manifest-path crates/schematic-viewer/Cargo.toml
cargo test --locked --manifest-path crates/schematic-viewer/Cargo.toml

# Native Vello viewer
cargo check --locked --manifest-path crates/schematic-viewer/Cargo.toml \
  --no-default-features --features renderer-vello
cargo test --locked --manifest-path crates/schematic-viewer/Cargo.toml \
  --no-default-features --features renderer-vello
cargo clippy --locked --manifest-path crates/schematic-viewer/Cargo.toml \
  --no-default-features --features renderer-vello --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked \
  --manifest-path crates/schematic-viewer/Cargo.toml \
  --no-default-features --features renderer-vello --no-deps
```

## Wayland and X11

GTK normally chooses the active backend for the compatibility viewer. Diagnose
one backend at a time with `GDK_BACKEND=wayland` or `GDK_BACKEND=x11`.

winit 0.30 uses the standard `WAYLAND_DISPLAY` and `DISPLAY` variables. To
exercise only one native backend, launch from a session where only that display
socket is exposed:

```bash
# Wayland only
env -u DISPLAY WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
  cargo run --manifest-path crates/schematic-viewer/Cargo.toml \
  --no-default-features --features renderer-vello -- path/to/design.kicad_sch

# X11/XWayland only
env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY" \
  cargo run --manifest-path crates/schematic-viewer/Cargo.toml \
  --no-default-features --features renderer-vello -- path/to/design.kicad_sch
```

Set `RUST_LOG=wgpu_core=info,wgpu_hal=info` to inspect adapter and surface
selection. A successful compile or headless render is not interactive Wayland,
X11, Windows, or macOS runtime proof; record those checks separately on the
platform and display server actually exercised.
