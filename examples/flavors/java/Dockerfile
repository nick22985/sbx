# syntax=docker/dockerfile:1
FROM sbx-base:latest

ARG SBX_JAVA_VERSION=21
ARG SBX_MAVEN_VERSION=latest

USER dev
RUN --mount=type=cache,target=/home/dev/.cache/mise,uid=1000,gid=1000,sharing=locked \
    /usr/local/bin/mise use --global "java@${SBX_JAVA_VERSION}" "maven@${SBX_MAVEN_VERSION}" \
    && /usr/local/bin/mise which java \
    && /usr/local/bin/mise which mvn

USER root
RUN { \
        echo 'export MAVEN_OPTS="${MAVEN_OPTS:--Dmaven.repo.local=/home/dev/.m2/repository}"'; \
        echo 'export MISE_YES=1'; \
        echo '# mise activate uses a precmd hook that does not fire for bash -lc.'; \
        echo '# eval `mise env` so JAVA_HOME and tool PATHs are set up front.'; \
        echo 'eval "$(/usr/local/bin/mise env -s bash 2>/dev/null)" 2>/dev/null || true'; \
    } >> /etc/profile.d/sbx.sh

USER dev

ENV MISE_YES=1 \
    MAVEN_OPTS=-Dmaven.repo.local=/home/dev/.m2/repository
