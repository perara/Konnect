# Konnect MCP server -- containerized so it can be run without a local Rust
# toolchain (e.g. `docker run --rm -i konnect`, the same way markitdown ships).
#
# Two stages: a builder with the Rust toolchain + protoc + cmake (nng builds
# libnng via cmake, konnect-ipc runs prost-build via protoc), and a slim
# Debian runtime that carries only the compiled binary.

# ---- Builder ----------------------------------------------------------------
FROM rust:1-bookworm AS builder

# libprotobuf-dev ships the well-known protos (google/protobuf/*.proto) under
# /usr/include; konnect-ipc's build.rs derives that include dir from $PROTOC.
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev cmake \
    && rm -rf /var/lib/apt/lists/*
ENV PROTOC=/usr/bin/protoc

WORKDIR /src
COPY . .

# Only the server binary; schematic-viewer (Tauri) is excluded from the workspace.
RUN cargo build --release -p konnect

# ---- Runtime ----------------------------------------------------------------
FROM debian:bookworm-slim

# ca-certificates for any outbound HTTPS (e.g. parts-DB lookups); curl is used
# by the compose healthcheck to probe the HTTP /health endpoint.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 konnect

COPY --from=builder /src/target/release/konnect /usr/local/bin/konnect
COPY docker/konnect.toml /etc/konnect/konnect.toml

USER konnect
ENV HOME=/home/konnect

# The first-run "install" step copies skills/agents into a host Claude config,
# which is meaningless inside the container (the Claude client lives on the
# host). Pre-create the marker so startup skips it on every launch.
RUN mkdir -p /home/konnect/.konnect && touch /home/konnect/.konnect/.installed

# Mount your KiCAD project here (schematic-edit tools work on file paths).
WORKDIR /work

# Default: stdio MCP transport, for `docker run --rm -i konnect`.
# For HTTP hosting, override with: konnect --config /etc/konnect/konnect.toml
ENTRYPOINT ["konnect"]
