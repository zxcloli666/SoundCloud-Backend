FROM rust:bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler cmake clang libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY --from=tls-common . /app/utils/tls-common
COPY --from=call . /app/utils/call
COPY --from=decrypt . /app/utils/decrypt
COPY --from=decrypt-client . /app/utils/decrypt-client
COPY --from=stream-ticket . /app/utils/stream-ticket
COPY --from=sc-fingerprint . /app/utils/sc-fingerprint

WORKDIR /app/streaming

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

COPY src ./src
COPY sc_methods ./sc_methods
ARG APP_VERSION
RUN { [ -z "$APP_VERSION" ] || sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" Cargo.toml; } \
    && touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/streaming/target/release/streaming /usr/local/bin/streaming

RUN mkdir -p /var/cache/acme

ENV PORT=8080
EXPOSE 8080 80 443

VOLUME ["/data"]

CMD ["streaming"]
