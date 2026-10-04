#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Prepares a throwaway database for the end-to-end checks exactly the way the compose stack does:
# creates the database, creates the low-privilege `baze_app` role with infra/postgres/init/01-roles.sh,
# applies the migrations as the owner, and prints the application role's DATABASE_URL on stdout
# (everything else goes to stderr, so `url="$(prepare-db.sh ...)"` captures only the URL).
#
# Usage:  scripts/e2e/prepare-db.sh <admin-database-url> [database-name]
#   admin-database-url  a superuser connection to a disposable PostgreSQL + PostGIS server,
#                       e.g. postgres://postgres:postgres@127.0.0.1:5432/postgres
#   database-name       defaults to baze_e2e; it is DROPPED and recreated
# Environment:
#   BAZE_SERVER  path to the baze-server binary (default: backend/target/debug/baze-server)
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "usage: $0 <admin-database-url> [database-name]" >&2
  exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
admin_url="$1"
db_name="${2:-baze_e2e}"
server="${BAZE_SERVER:-$root/backend/target/debug/baze-server}"

if [[ ! "$db_name" =~ ^[a-z_][a-z0-9_]*$ ]]; then
  echo "invalid database name: $db_name" >&2
  exit 2
fi
[ -x "$server" ] || { echo "baze-server not found at $server (build it first)" >&2; exit 1; }

# Split the URL into the pieces libpq understands through its environment (one per line).
mapfile -t url_parts < <(python3 - "$admin_url" <<'PY'
import sys
from urllib.parse import urlparse, unquote

u = urlparse(sys.argv[1])
print(unquote(u.username or "postgres"))
print(unquote(u.password or ""))
print(u.hostname or "127.0.0.1")
print(u.port or 5432)
PY
)
admin_user="${url_parts[0]}"
admin_password="${url_parts[1]}"
db_host="${url_parts[2]}"
db_port="${url_parts[3]}"

export PGHOST="$db_host" PGPORT="$db_port" PGUSER="$admin_user"
if [ -n "$admin_password" ]; then export PGPASSWORD="$admin_password"; fi

psql -v ON_ERROR_STOP=1 -qX -d postgres \
  -c "DROP DATABASE IF EXISTS ${db_name} WITH (FORCE)" \
  -c "CREATE DATABASE ${db_name}" >&2

app_password="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
POSTGRES_USER="$admin_user" POSTGRES_DB="$db_name" APP_DB_PASSWORD="$app_password" \
  bash "$root/infra/postgres/init/01-roles.sh" >&2

# The owner role migrates; the application role never does.
owner_auth="$admin_user"
if [ -n "$admin_password" ]; then owner_auth="$admin_user:$admin_password"; fi
DATABASE_URL="postgres://${owner_auth}@${db_host}:${db_port}/${db_name}" "$server" migrate >&2

echo "postgres://baze_app:${app_password}@${db_host}:${db_port}/${db_name}"
