FROM mirror.gcr.io/library/rust:1.99.1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69 AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build

FROM scratch AS sources
COPY scripts/cargo-skeleton.sh /scripts/cargo-skeleton.sh
COPY utils/entity-ref /utils/entity-ref
COPY utils/redact /utils/redact
COPY utils/sc-fingerprint /utils/sc-fingerprint
COPY utils/telemetry /utils/telemetry
COPY utils/tls-common /utils/tls-common
COPY storage/Cargo.toml storage/Cargo.lock /storage/
COPY storage/src /storage/src

FROM toolchain AS skeleton
COPY --from=sources / ./
RUN sh scripts/cargo-skeleton.sh /skeleton storage utils

FROM toolchain AS public
COPY --from=skeleton /skeleton ./
RUN cargo build --release --manifest-path storage/Cargo.toml

FROM public AS own
COPY --from=sources /utils ./utils
RUN find utils -type f -exec touch {} + \
    && cargo build --release --manifest-path storage/Cargo.toml

FROM own AS app
COPY --from=sources /storage/Cargo.toml ./storage/Cargo.toml
COPY --from=sources /storage/src ./storage/src
RUN find storage -type f -not -path 'storage/target/*' -exec touch {} + \
    && cargo build --release --manifest-path storage/Cargo.toml

FROM mirror.gcr.io/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

COPY --from=app /build/storage/target/release/storage /usr/local/bin/storage

ENV PORT=3000 \
    ADMIN_TOKEN="" \
    STORAGE_BACKEND=local \
    STORAGE_PATH=/data/storage \
    TMP_PATH=/tmp/storage \
    MAX_TRANSCODES=2 \
    TRANSCODE_BATCH_SIZE=8 \
    TRANSCODE_BATCH_WAIT_MS=50 \
    UPLOAD_CONCURRENCY=64 \
    UPLOAD_RETRIES=4 \
    UPLOAD_RETRY_BASE_MS=250 \
    S3_REGION=us-east-1 \
    S3_FORCE_PATH_STYLE=true

RUN mkdir -p /data/storage /var/cache/acme

EXPOSE 3000 80 443

CMD ["storage"]
