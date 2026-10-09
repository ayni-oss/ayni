# syntax=docker/dockerfile:1.7
# The factory client connects to an external engine; it contains no daemon.
ARG DOCKER_CLIENT_IMAGE=docker.io/library/docker:28.3.3-cli@sha256:0135662b510037ea581d99c2e5929c5e01185139c0b86986a418bd4da0b98a44
ARG DEBIAN_IMAGE
FROM ${DOCKER_CLIENT_IMAGE} AS docker-client
FROM ${DEBIAN_IMAGE}
ARG AYNI_VERSION
ARG SOURCE_REVISION
LABEL org.opencontainers.image.title="Ayni image builder" \
      org.opencontainers.image.description="Factory client for certified repository environments" \
      org.opencontainers.image.source="https://github.com/ayni-oss/ayni" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.version="${AYNI_VERSION}" \
      org.opencontainers.image.licenses="Apache-2.0" \
      dev.ayni.executor.lock-schema="0.9.0" \
      dev.ayni.executor.recipe="2"
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 ayni \
    && useradd --uid 10001 --gid 10001 --create-home --shell /bin/sh ayni \
    && mkdir /workspace && chown 10001:10001 /workspace
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-client /usr/local/libexec/docker/cli-plugins/docker-buildx /usr/local/libexec/docker/cli-plugins/docker-buildx
COPY LICENSE /usr/share/doc/ayni/
# Keep the versioned executable in its own layer for environment assembly.
COPY --chmod=0755 ayni /usr/local/bin/ayni
ENV HOME=/home/ayni
USER 10001:10001
WORKDIR /workspace
CMD ["/bin/sh"]
