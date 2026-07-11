FROM rust:1-bookworm AS builder

# Shared tls-common crate is provided as a named build context.
# `additional_contexts: tls-common: ../utils/tls-common` in compose,
# or `docker build --build-context tls-common=../utils/tls-common ./gateway`.
COPY --from=tls-common . /build/utils/tls-common

WORKDIR /build/gateway

# Cache dependency compilation on a dummy main.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs && cargo build --release && rm -rf src

# Real build. Version is baked here (not in the deps layer) so a bump doesn't
# invalidate the dependency cache.
COPY src ./src
ARG APP_VERSION
RUN { [ -z "$APP_VERSION" ] || sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" Cargo.toml; } \
    && touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/gateway/target/release/gateway /usr/local/bin/gateway

RUN mkdir -p /var/cache/acme

EXPOSE 80 443
VOLUME ["/var/cache/acme"]

CMD ["gateway"]
