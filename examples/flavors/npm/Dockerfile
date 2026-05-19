# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG SBX_NODE_VERSION=lts

USER dev
RUN --mount=type=cache,target=/home/dev/.cache/mise,uid=1000,gid=1000,sharing=locked \
    /usr/local/bin/mise use --global "node@${SBX_NODE_VERSION}" \
    && /usr/local/bin/mise which node \
    && /usr/local/bin/mise which npm

# socket.dev CLI — installed under /home/dev so it can be invoked via the
# shim at /opt/sbx/bin/socket. `socket scan create .` to scan the project.
RUN --mount=type=cache,target=/home/dev/.npm,uid=1000,gid=1000,sharing=locked \
    mkdir -p /home/dev/.sbx/socket-cli \
    && cd /home/dev/.sbx/socket-cli \
    && NPM="$(/usr/local/bin/mise which npm)" \
    && "$NPM" init -y >/dev/null \
    && "$NPM" install @socketsecurity/cli

# npm-check-updates — installed under /home/dev so it can be invoked via the
# shim at /opt/sbx/bin/ncu. `ncu` to list, `ncu -u` to write package.json.
RUN --mount=type=cache,target=/home/dev/.npm,uid=1000,gid=1000,sharing=locked \
    mkdir -p /home/dev/.sbx/ncu \
    && cd /home/dev/.sbx/ncu \
    && NPM="$(/usr/local/bin/mise which npm)" \
    && "$NPM" init -y >/dev/null \
    && "$NPM" install npm-check-updates

USER root
RUN printf '#!/bin/sh\nNODE="$(/usr/local/bin/mise which node 2>/dev/null)" || NODE=node\nexec "$NODE" /home/dev/.sbx/socket-cli/node_modules/@socketsecurity/cli/dist/cli.js "$@"\n' > /opt/sbx/bin/socket \
    && chmod +x /opt/sbx/bin/socket \
    && printf '#!/bin/sh\nNODE="$(/usr/local/bin/mise which node 2>/dev/null)" || NODE=node\nexec "$NODE" /home/dev/.sbx/ncu/node_modules/npm-check-updates/build/cli.js "$@"\n' > /opt/sbx/bin/ncu \
    && chmod +x /opt/sbx/bin/ncu

USER dev

ENV NPM_CONFIG_CACHE=/home/dev/.npm
