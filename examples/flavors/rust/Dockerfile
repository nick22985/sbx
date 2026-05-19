# syntax=docker/dockerfile:1
FROM sbx-base:latest

# Build tooling cargo needs for native crate compilation.
USER root
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev

USER dev
RUN --mount=type=cache,target=/tmp/sbx-dl,sharing=locked,uid=1000,gid=1000 set -eux; \
    if [ ! -f /tmp/sbx-dl/rustup-init.sh ]; then \
        curl -fsSL https://sh.rustup.rs -o /tmp/sbx-dl/rustup-init.sh; \
    fi; \
    CARGO_HOME=/home/dev/.cargo RUSTUP_HOME=/home/dev/.rustup \
        bash /tmp/sbx-dl/rustup-init.sh -y --no-modify-path --default-toolchain stable --profile minimal; \
    CARGO_HOME=/home/dev/.cargo RUSTUP_HOME=/home/dev/.rustup \
        /home/dev/.cargo/bin/rustup component add rustfmt; \
    /home/dev/.cargo/bin/rustc --version

# cargo-audit — invoke via `cargo audit` for Cargo.lock vulnerability checks.
RUN --mount=type=cache,target=/home/dev/.cargo/registry,uid=1000,gid=1000,sharing=locked \
    /home/dev/.cargo/bin/cargo install cargo-audit --locked --no-default-features

ENV CARGO_HOME=/home/dev/.cargo \
    RUSTUP_HOME=/home/dev/.rustup \
    PATH=/home/dev/.local/bin:/home/dev/.local/share/mise/shims:/home/dev/.cargo/bin:/opt/sbx/bin:/usr/local/bin:/usr/bin:/bin
