FROM mirror.gcr.io/library/rust:1.99.1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69 AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler curl ca-certificates gnupg cmake clang libclang-dev pkg-config \
    && install -d /usr/share/postgresql-common/pgdg \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
         -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc \
    && echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" \
         > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends postgresql-17 postgresql-client-17 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build

FROM scratch AS sources
COPY scripts/cargo-skeleton.sh /scripts/cargo-skeleton.sh
COPY backend-contracts /backend-contracts
COPY utils/call /utils/call
COPY utils/catalog-ingest /utils/catalog-ingest
COPY utils/catalog-match /utils/catalog-match
COPY utils/catalog-normalize /utils/catalog-normalize
COPY utils/catalog-sources /utils/catalog-sources
COPY utils/entity-ref /utils/entity-ref
COPY utils/redact /utils/redact
COPY utils/sc-fingerprint /utils/sc-fingerprint
COPY utils/sc-transport /utils/sc-transport
COPY utils/stream-ticket /utils/stream-ticket
COPY utils/tls-common /utils/tls-common
COPY api/Cargo.toml api/Cargo.lock /api/
COPY api/src /api/src
COPY api/queries /api/queries
COPY api/migrations /api/migrations
COPY api/migrations-ops /api/migrations-ops

FROM toolchain AS skeleton
COPY --from=sources / ./
RUN sh scripts/cargo-skeleton.sh /skeleton api backend-contracts utils

FROM toolchain AS public
COPY --from=skeleton /skeleton ./
RUN cargo build --release --manifest-path api/Cargo.toml

FROM public AS own
COPY --from=sources /backend-contracts ./backend-contracts
COPY --from=sources /utils ./utils
RUN find backend-contracts utils -type f -exec touch {} + \
    && cargo build --release --manifest-path api/Cargo.toml

FROM own AS app
COPY --from=sources /api/Cargo.toml ./api/Cargo.toml
COPY --from=sources /api/src ./api/src
COPY --from=sources /api/queries ./api/queries
COPY --from=sources /api/migrations ./api/migrations
COPY --from=sources /api/migrations-ops ./api/migrations-ops
ARG APP_VERSION
RUN find api -type f -not -path 'api/target/*' -exec touch {} + \
    && pg_ctlcluster "$(ls /etc/postgresql)" main start \
    && su postgres -c "psql -v ON_ERROR_STOP=1 --command \"CREATE ROLE scd LOGIN SUPERUSER PASSWORD 'dev';\"" \
    && su postgres -c "createdb -O scd scd" \
    && for f in api/migrations/*.sql api/migrations-ops/*.sql; do PGPASSWORD=dev psql -h 127.0.0.1 -U scd -d scd -v ON_ERROR_STOP=1 -q -f "$f"; done \
    && { [ -z "$APP_VERSION" ] || sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" api/Cargo.toml; } \
    && DATABASE_URL="postgres://scd:dev@127.0.0.1:5432/scd" cargo build --release --manifest-path api/Cargo.toml

FROM mirror.gcr.io/library/debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /var/cache/acme

COPY --from=app /build/api/target/release/api /usr/local/bin/api

EXPOSE 80 443 3000

ENTRYPOINT ["/usr/local/bin/api"]
