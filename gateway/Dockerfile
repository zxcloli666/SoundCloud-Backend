FROM mirror.gcr.io/library/rust:1.99.1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69 AS toolchain
WORKDIR /build

FROM scratch AS sources
COPY scripts/cargo-skeleton.sh /scripts/cargo-skeleton.sh
COPY utils/tls-common /utils/tls-common
COPY gateway/Cargo.toml gateway/Cargo.lock /gateway/
COPY gateway/src /gateway/src

FROM toolchain AS skeleton
COPY --from=sources / ./
RUN sh scripts/cargo-skeleton.sh /skeleton gateway utils

FROM toolchain AS public
COPY --from=skeleton /skeleton ./
RUN cargo build --release --manifest-path gateway/Cargo.toml

FROM public AS own
COPY --from=sources /utils ./utils
RUN find utils -type f -exec touch {} + \
    && cargo build --release --manifest-path gateway/Cargo.toml

FROM own AS app
COPY --from=sources /gateway/Cargo.toml ./gateway/Cargo.toml
COPY --from=sources /gateway/src ./gateway/src
ARG APP_VERSION
RUN find gateway -type f -not -path 'gateway/target/*' -exec touch {} + \
    && { [ -z "$APP_VERSION" ] || sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" gateway/Cargo.toml; } \
    && cargo build --release --manifest-path gateway/Cargo.toml

FROM mirror.gcr.io/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=app /build/gateway/target/release/gateway /usr/local/bin/gateway

RUN mkdir -p /var/cache/acme

EXPOSE 80 443
VOLUME ["/var/cache/acme"]

CMD ["gateway"]
