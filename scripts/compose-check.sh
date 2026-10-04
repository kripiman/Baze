#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Validates the Docker Compose stack without starting anything:
#   1. both the production file and the development overlay must render;
#   2. the production file must keep the project's security invariants
#      (AGENTS.md: only Caddy publishes ports, no `latest` tags, isolated internal network).
#
# Needs `docker compose` (v2) and python3. The values below are fake and never leave this script.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

env_file="$(mktemp)"
trap 'rm -f "$env_file"' EXIT
cat > "$env_file" <<'ENV'
ENVIRONMENT=production
POSTGRES_PASSWORD=fake-password-for-validation-only
DATABASE_URL=postgres://baze_user:fake-password-for-validation-only@postgis:5432/baze_db
JWT_SECRET=fake-secret-for-validation-only
GIT_COMMIT_HASH=0123456789abcdef0123456789abcdef01234567
ENV

compose() {
  docker compose --env-file "$env_file" --profile '*' "$@"
}

echo "==> production stack renders"
compose -f infra/compose.yaml config -q

echo "==> development overlay renders"
compose -f infra/compose.yaml -f infra/compose.dev.yaml config -q

echo "==> production stack keeps the security invariants"
compose -f infra/compose.yaml config --format json | python3 scripts/compose_policy.py

echo "==> compose files are valid and hardened"
