# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG SBX_NVIM_VERSION=stable
ARG SBX_NODE_VERSION=lts
ARG USER_HOME=/home/dev

# Treesitter parsers compile in C; Mason occasionally builds tools from source.
# tree-sitter-cli isn't in Debian trixie apt; install via npm after node.
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
        xclip

USER dev
RUN --mount=type=cache,target=${USER_HOME}/.cache/mise,uid=1000,gid=1000,sharing=locked \
    /usr/local/bin/mise use --global \
        "neovim@${SBX_NVIM_VERSION}" \
        "node@${SBX_NODE_VERSION}" \
    && /usr/local/bin/mise exec -- npm install -g tree-sitter-cli \
    && /usr/local/bin/mise which nvim \
    && /usr/local/bin/mise which node \
    && /usr/local/bin/mise exec -- tree-sitter --version

ENV EDITOR=nvim VISUAL=nvim
