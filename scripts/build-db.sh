#!/bin/sh
set -eu

pg_ctlcluster "$(ls /etc/postgresql)" main start
if su postgres -c "psql -tAc \"SELECT 1 FROM pg_database WHERE datname = 'scd'\"" | grep -q 1; then
    exit 0
fi
su postgres -c "psql -v ON_ERROR_STOP=1 --command \"CREATE ROLE scd LOGIN SUPERUSER PASSWORD 'dev';\""
su postgres -c "createdb -O scd scd"
for file in api/migrations/*.sql api/migrations-ops/*.sql; do
    PGPASSWORD=dev psql -h 127.0.0.1 -U scd -d scd -v ON_ERROR_STOP=1 -q -f "$file"
done
