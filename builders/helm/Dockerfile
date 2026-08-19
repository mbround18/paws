# syntax=docker/dockerfile:1.7
#
# Builder for `paws helm` (lint/package Helm charts). No maintained
# "official" Helm runtime image exists to just pull (unlike `rust:1-bookworm`
# or `astral/uv:*` for the rust/python toolchains) — `alpine/helm` on Docker
# Hub is a third party's own versioning scheme, not Helm's — so this installs
# Helm's real upstream binary via its own official install script
# (https://helm.sh/docs/intro/install/#from-script), pinned to a specific
# released version via HELM_VERSION rather than trusting whatever "latest"
# resolves to on a given day. Built through Dagger, not `docker build`
# directly — see builders/README.md.

FROM alpine:3.20

ARG HELM_VERSION=v3.16.4

RUN apk add --no-cache bash curl ca-certificates git openssl && \
    curl -fsSL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 -o /tmp/get-helm-3.sh && \
    chmod +x /tmp/get-helm-3.sh && \
    /tmp/get-helm-3.sh --version "${HELM_VERSION}" && \
    rm /tmp/get-helm-3.sh && \
    helm version

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately — see
# builders/linux-gnu/Dockerfile's comment on why (BuildKit cache
# invalidation cascades downstream from whichever layer's inputs changed;
# BUILDER_CREATED changes every release, and the Helm install above is the
# only slow step here).
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-helm" \
      org.opencontainers.image.description="Helm CLI builder for linting/packaging Helm charts" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="helm"
