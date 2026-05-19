# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG USER_HOME=/home/dev

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
RUN --mount=type=cache,target=${USER_HOME}/.cache/mise,uid=1000,gid=1000,sharing=locked \
    MISE_RUST_PROFILE=minimal /usr/local/bin/mise use --global rust@stable \
    && /usr/local/bin/mise which rustc \
    && /usr/local/bin/mise which cargo

RUN --mount=type=cache,target=${USER_HOME}/.cargo/registry,uid=1000,gid=1000,sharing=locked \
    ${USER_HOME}/.local/share/mise/shims/cargo install cargo-audit --locked --no-default-features

ENV CARGO_HOME=${USER_HOME}/.cargo \
    PATH=${USER_HOME}/.local/bin:${USER_HOME}/.local/share/mise/shims:${USER_HOME}/.cargo/bin:/opt/sbx/bin:/usr/local/bin:/usr/bin:/bin

USER root
RUN printf 'if [ -d %s/.cargo/bin ]; then\n  case ":$PATH:" in *":%s/.cargo/bin:"*) ;; *) PATH="%s/.cargo/bin:$PATH";; esac\n  export PATH\nfi\nexport CARGO_HOME=%s/.cargo\n' "$USER_HOME" "$USER_HOME" "$USER_HOME" "$USER_HOME" > /etc/profile.d/rust.sh \
    && chmod 0644 /etc/profile.d/rust.sh
USER dev
