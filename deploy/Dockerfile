# meridiand image — SPEC §4/§14: FROM scratch + CA certs + static musl binary +
# models. Build context is a staging directory assembled by CI:
#   stage/meridiand                  (aarch64-musl release binary)
#   stage/models/...                 (fetched + SHA256-verified by fetch-models.sh)
# Size gate (<120MB) is enforced by the CI job, not here.

FROM alpine:3.22 AS certs
RUN apk add --no-cache ca-certificates

FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY stage/meridiand /meridiand
COPY stage/models /models
# nobody:nogroup — containers also run no-new-privileges + cap_drop ALL (compose)
USER 65534:65534
ENTRYPOINT ["/meridiand"]
