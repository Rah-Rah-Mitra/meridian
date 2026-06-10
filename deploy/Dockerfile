# meridiand image — SPEC §4/§14: FROM scratch + CA certs + static musl binary +
# models. Build context is a staging directory assembled by CI:
#   stage/linux/arm64/meridiand      (aarch64-musl release binary)
#   stage/linux/amd64/meridiand      (x86_64-musl release binary; release builds)
#   stage/models/...                 (fetched + SHA256-verified by fetch-models.sh)
# Size gate (<120MB) is enforced by the CI job, not here. Multi-arch: buildx
# expands $TARGETPLATFORM per platform and picks the matching staged binary.

FROM alpine:3.22 AS certs
RUN apk add --no-cache ca-certificates

FROM alpine:3.22 AS iddb
# Minimal user database for uid 65534: arti's fs-mistrust resolves the current
# user via getpwuid even with permission checks disabled, and musl's lookup
# errors outright when /etc/passwd is absent (glibc soft-fails). Three lines of
# passwd/group beat carrying a base image.
RUN printf 'meridian:x:65534:65534:meridian:/:/sbin/nologin\n' > /tmp/passwd \
    && printf 'meridian:x:65534:\n' > /tmp/group

FROM scratch
ARG TARGETPLATFORM
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=iddb /tmp/passwd /etc/passwd
COPY --from=iddb /tmp/group /etc/group
COPY stage/${TARGETPLATFORM}/meridiand /meridiand
COPY stage/models /models
# meridian (65534) — containers also run no-new-privileges + cap_drop ALL (compose)
USER 65534:65534
ENTRYPOINT ["/meridiand"]
