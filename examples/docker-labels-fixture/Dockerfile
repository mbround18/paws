# Fixture for `paws docker --label` / `--build-arg`. The build arg is echoed so a
# run that fails to pass it through is visible in the build log, and the labels are
# read back off the built image.
FROM alpine:3.20
ARG BUILD_ARG_PROBE=unset
RUN echo "build-arg-probe=${BUILD_ARG_PROBE}"
CMD ["echo", "docker-labels-fixture"]
