FROM mirror.gcr.io/library/rust:1.99.1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69 AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler cmake clang libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY --from=tls-common . /app/utils/tls-common
COPY --from=call . /app/utils/call
COPY --from=decrypt . /app/utils/decrypt
COPY --from=decrypt-client . /app/utils/decrypt-client
COPY --from=stream-ticket . /app/utils/stream-ticket
COPY --from=sc-fingerprint . /app/utils/sc-fingerprint
COPY --from=entity-ref . /app/utils/entity-ref

WORKDIR /app/streaming

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

COPY src ./src
COPY sc_methods ./sc_methods
ARG APP_VERSION
RUN { [ -z "$APP_VERSION" ] || sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" Cargo.toml; } \
    && touch src/main.rs && cargo build --release

FROM mirror.gcr.io/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/streaming/target/release/streaming /usr/local/bin/streaming

RUN mkdir -p /var/cache/acme

ENV PORT=8080
EXPOSE 8080 80 443

VOLUME ["/data"]

CMD ["streaming"]
