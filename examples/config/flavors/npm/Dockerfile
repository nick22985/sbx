# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG SBX_NODE_VERSION=lts
ARG USER_HOME=/home/dev

USER dev
RUN --mount=type=cache,target=${USER_HOME}/.cache/mise,uid=1000,gid=1000,sharing=locked \
    /usr/local/bin/mise use --global "node@${SBX_NODE_VERSION}" \
    && /usr/local/bin/mise which node \
    && /usr/local/bin/mise which npm

RUN --mount=type=cache,target=${USER_HOME}/.npm,uid=1000,gid=1000,sharing=locked \
    mkdir -p ${USER_HOME}/.sbx/socket-cli \
    && cd ${USER_HOME}/.sbx/socket-cli \
    && NPM="$(/usr/local/bin/mise which npm)" \
    && "$NPM" init -y >/dev/null \
    && "$NPM" install @socketsecurity/cli

RUN --mount=type=cache,target=${USER_HOME}/.npm,uid=1000,gid=1000,sharing=locked \
    mkdir -p ${USER_HOME}/.sbx/ncu \
    && cd ${USER_HOME}/.sbx/ncu \
    && NPM="$(/usr/local/bin/mise which npm)" \
    && "$NPM" init -y >/dev/null \
    && "$NPM" install npm-check-updates

USER root
RUN printf '#!/bin/sh\nNODE="$(/usr/local/bin/mise which node 2>/dev/null)" || NODE=node\nexec "$NODE" %s/.sbx/socket-cli/node_modules/@socketsecurity/cli/dist/cli.js "$@"\n' "$USER_HOME" > /opt/sbx/bin/socket \
    && chmod +x /opt/sbx/bin/socket \
    && printf '#!/bin/sh\nNODE="$(/usr/local/bin/mise which node 2>/dev/null)" || NODE=node\nexec "$NODE" %s/.sbx/ncu/node_modules/npm-check-updates/build/cli.js "$@"\n' "$USER_HOME" > /opt/sbx/bin/ncu \
    && chmod +x /opt/sbx/bin/ncu

USER dev

ENV NPM_CONFIG_CACHE=${USER_HOME}/.npm
