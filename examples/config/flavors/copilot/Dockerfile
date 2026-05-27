FROM sbx-base:latest

ARG USER_HOME=/home/dev

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

RUN --mount=type=cache,target=/root/.npm,sharing=locked \
    /usr/bin/npm install -g @github/copilot \
    && copilot --version

USER dev

# Pre-create XDG dirs as dev so bind mounts don't leave parents root-owned.
RUN mkdir -p ${USER_HOME}/.copilot \
    ${USER_HOME}/.local/state \
    ${USER_HOME}/.cache

ENV NPM_CONFIG_CACHE=${USER_HOME}/.npm \
    PATH=/opt/sbx/bin:${USER_HOME}/.local/bin:${USER_HOME}/.local/share/mise/shims:/usr/local/bin:/usr/bin:/bin

CMD ["/bin/bash"]
