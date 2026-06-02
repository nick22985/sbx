# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG SBX_NVIM_VERSION=stable
ARG SBX_NODE_VERSION=lts
ARG SBX_COPILOT_NODE_VERSION=24.13.0
ARG USER_HOME=/home/dev
ARG USER_UID=1000

USER root
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        cmake \
        fd-find \
        fzf \
        gettext \
        wl-clipboard \
        xclip \
        luarocks \
        lua5.1 \
        liblua5.1-0-dev \
        lua-dkjson \
        unzip \
        python3-pynvim \
        gh

RUN install -Dm644 /usr/share/lua/5.1/dkjson.lua /usr/local/share/lua/5.1/dkjson.lua

RUN mkdir -p /run/user/${USER_UID} \
    && chown ${USER_UID}:${USER_UID} /run/user/${USER_UID} \
    && chmod 700 /run/user/${USER_UID}

USER dev
RUN --mount=type=cache,target=${USER_HOME}/.cache/mise,uid=1000,gid=1000,sharing=locked \
    /usr/local/bin/mise use --global \
        "neovim@${SBX_NVIM_VERSION}" \
        "node@${SBX_NODE_VERSION}" \
        "node@${SBX_COPILOT_NODE_VERSION}" \
    && /usr/local/bin/mise exec -- npm install -g tree-sitter-cli \
    && /usr/local/bin/mise which nvim \
    && /usr/local/bin/mise which node \
    && test -x "${USER_HOME}/.local/share/mise/installs/node/${SBX_COPILOT_NODE_VERSION}/bin/node" \
    && /usr/local/bin/mise exec -- tree-sitter --version

ENV EDITOR=nvim VISUAL=nvim
