# syntax=docker/dockerfile:1
#
# The server of the optional Hangar account (docs/native/cloud.md), behind
# Cloudflare at https://hangar-account.simnjs.fr like the site is at
# https://hangar.simnjs.fr (the Dockerfile at the root).
#
# Built from the root of the repository, which is where the workspace's
# Cargo.toml and Cargo.lock are:
#
#   docker build -f crates/hangar-cloud/Dockerfile -t hangar-cloud .
#
# CI builds it, checks it and publishes it as ghcr.io/hangar-ai/hangar-cloud
# (.github/workflows/cloud-image.yml); compose.yaml, beside this file, runs it.
#
# Dockerfile.dockerignore, beside this file, is what BuildKit reads instead of
# the root's .dockerignore (the site's): it sends the workspace's manifests and
# crates, and nothing else.

# Debian 12 on both sides: the binary links the C library it was built
# against. The `slim` Rust image has the C compiler that SQLite (rusqlite's
# `bundled`) and ring (rustls) are compiled with.
FROM rust:1-slim-bookworm AS build
WORKDIR /src
# The whole workspace: cargo reads every member's manifest before it builds
# one of them. Only hangar-cloud and what it depends on are downloaded and
# compiled, at the versions the lockfile names.
COPY . .
# The registry and the build folder are kept between builds; the binary is
# copied out of the cache, which is not part of the layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p hangar-cloud \
 && cp target/release/hangar-cloud /usr/local/bin/hangar-cloud

FROM debian:bookworm-slim
# No package to install: SQLite is compiled in, and rustls carries its own
# root certificates. A user of its own, which owns the volume and nothing else.
RUN useradd --system --uid 10001 --user-group --no-create-home --home-dir /data hangar \
 && mkdir /data \
 && chown hangar:hangar /data
COPY --from=build /usr/local/bin/hangar-cloud /usr/local/bin/hangar-cloud
USER hangar
# The database, and only it. A named volume mounted here starts with the
# folder's owner, so the server can write to it.
VOLUME /data
ENV HANGAR_CLOUD_BIND=0.0.0.0:8080 \
    HANGAR_CLOUD_DB=/data/hangar-cloud.sqlite
EXPOSE 8080
# The binary checks itself (`hangar-cloud health`, src/health.rs): the image
# has no curl to call /health with. `docker ps` shows the result, and the
# Compose file's tunnel waits for it.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/hangar-cloud", "health"]
# `docker stop` sends SIGTERM, which the server answers by finishing the
# requests in flight (main.rs).
ENTRYPOINT ["/usr/local/bin/hangar-cloud"]
