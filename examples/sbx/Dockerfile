# Optional per-project Dockerfile. If present, sbx builds an image on top of
# the flavor image (sbx-<flavor>:latest) and uses it for this project only.
# Tagged as sbx-<flavor>-<project>:latest. Rebuilt automatically when this
# file changes.
FROM sbx-npm:latest

# Layer in anything project-specific. Run as root; mise/cargo/npm caches
# already live under /home/dev so installs persist across container restarts.
USER root
RUN apt-get update \
 && apt-get install -y --no-install-recommends imagemagick poppler-utils \
 && rm -rf /var/lib/apt/lists/*
USER dev
