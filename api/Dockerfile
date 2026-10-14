FROM mirror.gcr.io/library/rust:1-bookworm AS toolchain
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

FROM toolchain AS skeleton
COPY scripts/cargo-skeleton.sh /usr/local/bin/cargo-skeleton
COPY backend-contracts ./backend-contracts
COPY utils ./utils
COPY api ./api
RUN cargo-skeleton /skeleton api backend-contracts utils

FROM toolchain AS dependencies
COPY --from=skeleton /skeleton ./
RUN cargo build --release --manifest-path api/Cargo.toml

FROM dependencies AS builder
COPY backend-contracts ./backend-contracts
COPY utils ./utils
COPY api ./api
ARG APP_VERSION
RUN find api backend-contracts utils -name '*.rs' -not -path '*/target/*' -exec touch {} + \
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

COPY --from=builder /build/api/target/release/api /usr/local/bin/api

EXPOSE 80 443 3000

ENTRYPOINT ["/usr/local/bin/api"]
