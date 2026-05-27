# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG USER_HOME=/home/dev

# Node.js (LTS) + npm — opencode ships as an npm package.
USER root
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    --mount=type=cache,target=/tmp/sbx-dl,sharing=locked \
    if [ ! -f /tmp/sbx-dl/nodesource-setup.sh ]; then \
        curl -fsSL https://deb.nodesource.com/setup_lts.x -o /tmp/sbx-dl/nodesource-setup.sh; \
    fi \
    && bash /tmp/sbx-dl/nodesource-setup.sh \
    && apt-get install -y --no-install-recommends nodejs \
    && node --version \
    && npm --version

# opencode — terminal AI coding agent (https://opencode.ai). Auth + config
# live under the user's home (~/.local/share/opencode, ~/.config/opencode),
# which sbx mounts from the host at run time, so no setup is needed here.
RUN --mount=type=cache,target=/root/.npm,sharing=locked \
    /usr/bin/npm install -g opencode-ai \
    && opencode --version

USER dev

# Pre-create the XDG dirs as dev. Without this, bind-mounting the nested
# ~/.local/share/opencode makes Docker create ~/.local + ~/.local/share as
# root, which blocks opencode from writing siblings like ~/.local/state.
RUN mkdir -p ${USER_HOME}/.config/opencode \
    ${USER_HOME}/.local/share/opencode \
    ${USER_HOME}/.local/state \
    ${USER_HOME}/.cache

ENV NPM_CONFIG_CACHE=${USER_HOME}/.npm \
    PATH=/opt/sbx/bin:${USER_HOME}/.local/bin:${USER_HOME}/.local/share/mise/shims:/usr/local/bin:/usr/bin:/bin

CMD ["/bin/bash"]
