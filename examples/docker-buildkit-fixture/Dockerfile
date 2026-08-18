# syntax=docker/dockerfile:1.7
#
# Deliberately uses BuildKit-only features (cache mounts, heredoc COPY) so a
# build against this fixture only succeeds when the build path actually runs
# through BuildKit (e.g. `docker buildx build`), not the legacy builder.
FROM alpine:3.20 AS base

RUN --mount=type=cache,target=/var/cache/apk \
    apk add --no-cache curl

COPY <<'EOF' /app/hello.txt
hello from buildkit
EOF

CMD ["cat", "/app/hello.txt"]
