FROM mirror.gcr.io/library/rust:1.99.1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69 AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler cmake clang libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build

FROM scratch AS sources
COPY scripts/cargo-skeleton.sh /scripts/cargo-skeleton.sh
COPY utils/call /utils/call
COPY utils/decrypt /utils/decrypt
COPY utils/decrypt-client /utils/decrypt-client
COPY utils/entity-ref /utils/entity-ref
COPY utils/sc-fingerprint /utils/sc-fingerprint
COPY utils/stream-ticket /utils/stream-ticket
COPY utils/telemetry /utils/telemetry
COPY utils/tls-common /utils/tls-common
COPY streaming/Cargo.toml streaming/Cargo.lock /streaming/
COPY streaming/src /streaming/src
COPY streaming/sc_methods /streaming/sc_methods

FROM toolchain AS skeleton
COPY --from=sources / ./
RUN sh scripts/cargo-skeleton.sh /skeleton streaming utils

FROM toolchain AS public
COPY --from=skeleton /skeleton ./
RUN cargo build --release --manifest-path streaming/Cargo.toml

FROM public AS own
COPY --from=sources /utils ./utils
RUN find utils -type f -exec touch {} + \
    && cargo build --release --manifest-path streaming/Cargo.toml

FROM own AS app
COPY --from=sources /streaming/Cargo.toml ./streaming/Cargo.toml
COPY --from=sources /streaming/src ./streaming/src
COPY --from=sources /streaming/sc_methods ./streaming/sc_methods
ARG APP_VERSION
RUN find streaming -type f -not -path 'streaming/target/*' -exec touch {} + \
    && { [ -z "$APP_VERSION" ] || sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" streaming/Cargo.toml; } \
    && cargo build --release --manifest-path streaming/Cargo.toml

FROM mirror.gcr.io/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=app /build/streaming/target/release/streaming /usr/local/bin/streaming

RUN mkdir -p /var/cache/acme

ENV PORT=8080
EXPOSE 8080 80 443

VOLUME ["/data"]

CMD ["streaming"]
