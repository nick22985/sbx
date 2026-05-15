# syntax=docker/dockerfile:1
#
# sbx-base — common layer shared by all sbx flavors. Per-language flavors
# `FROM sbx-base:latest` and install only their language tool on top. Keep
# this minimal: anything language-specific belongs in the leaf flavor.
FROM debian:bookworm-slim

ARG USER_UID=1000
ARG USER_GID=1000
ARG SBX_GUARDDOG=1

ENV DEBIAN_FRONTEND=noninteractive

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        git \
        ca-certificates \
        curl \
        less \
        openssh-client \
        ripgrep \
        vim-tiny \
        unzip \
        xz-utils \
        python3 \
        python3-pip \
        python3-venv

RUN if getent group "$USER_GID" >/dev/null; then \
        EXISTING_GROUP="$(getent group "$USER_GID" | cut -d: -f1)"; \
        if [ "$EXISTING_GROUP" != "dev" ]; then groupmod -n dev "$EXISTING_GROUP"; fi; \
    else \
        groupadd -g "$USER_GID" dev; \
    fi \
    && if getent passwd "$USER_UID" >/dev/null; then \
        EXISTING_USER="$(getent passwd "$USER_UID" | cut -d: -f1)"; \
        if [ "$EXISTING_USER" != "dev" ]; then usermod -l dev -d /home/dev -m "$EXISTING_USER"; fi; \
        usermod -g dev dev; \
    else \
        useradd -m -u "$USER_UID" -g dev -s /bin/bash dev; \
    fi \
    && mkdir -p /home/dev /opt/sbx/bin \
    && chown -R dev:dev /home/dev

RUN --mount=type=cache,target=/tmp/sbx-dl,sharing=locked set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) osv_arch=linux_amd64 ;; \
        arm64) osv_arch=linux_arm64 ;; \
        *) echo "unsupported arch $arch"; exit 1 ;; \
    esac; \
    if [ ! -f "/tmp/sbx-dl/osv-scanner-$osv_arch" ]; then \
        curl -fsSL -o "/tmp/sbx-dl/osv-scanner-$osv_arch" \
            "https://github.com/google/osv-scanner/releases/latest/download/osv-scanner_$osv_arch"; \
    fi; \
    install -m 0755 "/tmp/sbx-dl/osv-scanner-$osv_arch" /usr/local/bin/osv-scanner; \
    osv-scanner --version

RUN --mount=type=cache,target=/tmp/sbx-dl,sharing=locked \
    curl -sfL https://raw.githubusercontent.com/aquasecurity/trivy/main/contrib/install.sh \
        | sh -s -- -b /usr/local/bin \
    && trivy --version

RUN --mount=type=cache,target=/root/.cache/pip,sharing=locked \
    if [ "$SBX_GUARDDOG" = "1" ]; then \
        python3 -m venv /opt/guarddog \
        && /opt/guarddog/bin/pip install guarddog \
        && ln -s /opt/guarddog/bin/guarddog /usr/local/bin/guarddog \
        && guarddog --version ; \
    fi

RUN --mount=type=cache,target=/tmp/sbx-dl,sharing=locked set -eux; \
    if [ ! -f /tmp/sbx-dl/mise-install.sh ]; then \
        curl -fsSL https://mise.run -o /tmp/sbx-dl/mise-install.sh; \
    fi; \
    MISE_INSTALL_PATH=/usr/local/bin/mise sh /tmp/sbx-dl/mise-install.sh; \
    mise --version

ARG DOCKER_VERSION=27.4.0
RUN --mount=type=cache,target=/tmp/sbx-dl,sharing=locked set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) docker_arch=x86_64 ;; \
        arm64) docker_arch=aarch64 ;; \
        *) echo "unsupported arch $arch"; exit 1 ;; \
    esac; \
    tgz="/tmp/sbx-dl/docker-${DOCKER_VERSION}-${docker_arch}.tgz"; \
    if [ ! -f "$tgz" ]; then \
        curl -fsSL "https://download.docker.com/linux/static/stable/${docker_arch}/docker-${DOCKER_VERSION}.tgz" \
            -o "$tgz"; \
    fi; \
    tar -xz -C /tmp -f "$tgz" docker/docker; \
    install -m 0755 /tmp/docker/docker /usr/local/bin/docker; \
    rm -rf /tmp/docker; \
    docker --version

RUN printf 'export PATH=/opt/sbx/bin:/home/dev/.local/bin:/home/dev/.local/share/mise/shims:$PATH\nexport MISE_INSTALL_MISSING_TOOLS=1\neval "$(/usr/local/bin/mise activate bash)" 2>/dev/null || true\n' > /etc/profile.d/sbx.sh

# Bundled audit entry point — runs whichever scanners are installed.
COPY sbx-audit /opt/sbx/bin/sbx-audit
RUN chmod +x /opt/sbx/bin/sbx-audit

USER dev
WORKDIR /home/dev

ENV SHELL=/bin/bash \
    MISE_INSTALL_MISSING_TOOLS=1 \
    PATH=/opt/sbx/bin:/home/dev/.local/bin:/home/dev/.local/share/mise/shims:/usr/local/bin:/usr/bin:/bin

CMD ["/bin/bash"]
