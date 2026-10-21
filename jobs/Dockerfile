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
COPY scripts/cargo-skeleton.sh scripts/build-db.sh /scripts/
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
COPY utils/tls-common /utils/tls-common
COPY jobs/Cargo.toml jobs/Cargo.lock /jobs/
COPY jobs/src /jobs/src
COPY jobs/queries /jobs/queries
COPY api/migrations /api/migrations
COPY api/migrations-ops /api/migrations-ops
COPY api/queries /api/queries

FROM toolchain AS skeleton
COPY --from=sources / ./
RUN sh scripts/cargo-skeleton.sh /skeleton jobs backend-contracts utils

FROM toolchain AS public
COPY --from=skeleton /skeleton ./
RUN cargo build --release --manifest-path jobs/Cargo.toml

FROM public AS own
COPY --from=sources /backend-contracts ./backend-contracts
COPY --from=sources /utils ./utils
COPY --from=sources /api/migrations ./api/migrations
COPY --from=sources /api/migrations-ops ./api/migrations-ops
COPY --from=sources /scripts/build-db.sh ./scripts/build-db.sh
RUN find backend-contracts utils -type f -exec touch {} + \
    && sh scripts/build-db.sh \
    && DATABASE_URL=postgres://scd:dev@127.0.0.1:5432/scd cargo build --release --manifest-path jobs/Cargo.toml

FROM own AS app
COPY --from=sources /jobs/Cargo.toml ./jobs/Cargo.toml
COPY --from=sources /jobs/src ./jobs/src
COPY --from=sources /jobs/queries ./jobs/queries
COPY --from=sources /api/queries ./api/queries
ARG APP_VERSION
RUN find jobs api/queries -type f -not -path 'jobs/target/*' -exec touch {} + \
    && sh scripts/build-db.sh \
    && if [ -n "$APP_VERSION" ]; then sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" jobs/Cargo.toml; fi \
    && DATABASE_URL=postgres://scd:dev@127.0.0.1:5432/scd cargo build --release --manifest-path jobs/Cargo.toml

FROM mirror.gcr.io/library/debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home jobs \
    && install -d -o jobs -g jobs -m 0750 /snapshots

COPY --from=app /build/jobs/target/release/jobs /usr/local/bin/jobs
COPY --from=app /build/jobs/target/release/migrate /usr/local/bin/migrate

USER jobs
EXPOSE 3001
HEALTHCHECK --interval=10s --timeout=2s --retries=3 CMD curl --fail --silent http://127.0.0.1:3001/ready || exit 1
ENTRYPOINT ["/usr/local/bin/jobs"]
