FROM rust:1.97-bookworm AS builder

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
COPY backend-contracts ./backend-contracts
COPY utils ./utils
COPY jobs ./jobs
COPY api/migrations ./api/migrations
COPY api/migrations-ops ./api/migrations-ops
COPY api/queries ./api/queries

ARG APP_VERSION

RUN pg_ctlcluster "$(ls /etc/postgresql)" main start \
    && su postgres -c "psql -v ON_ERROR_STOP=1 --command \"CREATE ROLE scd LOGIN SUPERUSER PASSWORD 'dev';\"" \
    && su postgres -c "createdb -O scd scd" \
    && for file in api/migrations/*.sql api/migrations-ops/*.sql; do PGPASSWORD=dev psql -h 127.0.0.1 -U scd -d scd -v ON_ERROR_STOP=1 -q -f "$file"; done \
    && if [ -n "$APP_VERSION" ]; then sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$APP_VERSION\"/" jobs/Cargo.toml; fi \
    && DATABASE_URL=postgres://scd:dev@127.0.0.1:5432/scd cargo build --manifest-path jobs/Cargo.toml --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home jobs \
    && install -d -o jobs -g jobs -m 0750 /snapshots

COPY --from=builder /build/jobs/target/release/jobs /usr/local/bin/jobs
COPY --from=builder /build/jobs/target/release/migrate /usr/local/bin/migrate

USER jobs
EXPOSE 3001
HEALTHCHECK --interval=10s --timeout=2s --retries=3 CMD curl --fail --silent http://127.0.0.1:3001/ready || exit 1
ENTRYPOINT ["/usr/local/bin/jobs"]
