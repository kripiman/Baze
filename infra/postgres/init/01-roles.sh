#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Runs once, when the PostgreSQL data directory is first initialised (the official image executes
# everything in /docker-entrypoint-initdb.d). Creates the low-privilege role the backend connects with.
#
# Two roles (ADR-0008):
#   $POSTGRES_USER  owns the schema; used only by `baze-server migrate`.
#   baze_app        read/write rows and nothing else: no DDL, no extensions, no server-side programs.
set -euo pipefail

: "${APP_DB_PASSWORD:?APP_DB_PASSWORD must be set}"
: "${POSTGRES_USER:?}"
: "${POSTGRES_DB:?}"

psql -v ON_ERROR_STOP=1 \
    --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
    -v app_pw="$APP_DB_PASSWORD" -v admin="$POSTGRES_USER" -v dbname="$POSTGRES_DB" <<'SQL'
-- Create the role if it does not exist; always (re)set its password so re-running rotates it.
SELECT format(
    'CREATE ROLE baze_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 25 PASSWORD %L',
    :'app_pw')
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'baze_app') \gexec
SELECT format('ALTER ROLE baze_app PASSWORD %L', :'app_pw') \gexec

-- Bound what one runaway or abusive request can cost.
ALTER ROLE baze_app SET statement_timeout = '5s';
ALTER ROLE baze_app SET idle_in_transaction_session_timeout = '10s';
ALTER ROLE baze_app SET lock_timeout = '3s';

-- Nobody but the owner and baze_app can even connect.
REVOKE ALL ON DATABASE :"dbname" FROM PUBLIC;
GRANT CONNECT ON DATABASE :"dbname" TO baze_app;

-- Rows, not structure: tables created later by the migration role are readable and writable by the
-- application role, but it cannot create or alter anything.
GRANT USAGE ON SCHEMA public TO baze_app;
ALTER DEFAULT PRIVILEGES FOR ROLE :"admin" IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO baze_app;
ALTER DEFAULT PRIVILEGES FOR ROLE :"admin" IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO baze_app;
SQL

echo "baze_app role ready"
