#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Creates infra/.env from infra/.env.example with freshly generated random secrets.
# Never overwrites an existing infra/.env. The result is gitignored and readable only by you.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
example="$root/infra/.env.example"
target="$root/infra/.env"

if [ -e "$target" ]; then
  echo "infra/.env already exists; leaving it untouched."
  exit 0
fi

# 32 random bytes as 64 hex characters (hex is safe inside a connection URL).
rand_hex() {
  head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n'
}

db_password="$(rand_hex)"
app_password="$(rand_hex)"
jwt_secret="$(rand_hex)"

# Database name and owner come from the template so the URLs always match them.
db_name="$(sed -n 's/^POSTGRES_DB=//p' "$example" | head -n1)"
db_owner="$(sed -n 's/^POSTGRES_USER=//p' "$example" | head -n1)"

umask 077
sed \
  -e "s|^POSTGRES_PASSWORD=.*|POSTGRES_PASSWORD=${db_password}|" \
  -e "s|^APP_DB_PASSWORD=.*|APP_DB_PASSWORD=${app_password}|" \
  -e "s|^DATABASE_URL=.*|DATABASE_URL=postgres://baze_app:${app_password}@postgis:5432/${db_name}|" \
  -e "s|^MIGRATION_DATABASE_URL=.*|MIGRATION_DATABASE_URL=postgres://${db_owner}:${db_password}@postgis:5432/${db_name}|" \
  -e "s|^JWT_SECRET=.*|JWT_SECRET=${jwt_secret}|" \
  -e "s|^ENGINES_UID=.*|ENGINES_UID=$(id -u)|" \
  -e "s|^ENGINES_GID=.*|ENGINES_GID=$(id -g)|" \
  "$example" > "$target"

echo "Created infra/.env with random secrets (mode 600). Review it before using it outside local development."
