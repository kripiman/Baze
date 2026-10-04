# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later

.PHONY: help dev-up dev-down backend-check android-build data-build backend-test backend-fmt backend-clippy backend-deny toolchain-check dev-env openapi compose-check

help:
	@echo "Baze Monorepo Management"
	@echo "------------------------"
	@echo "dev-env        : Create infra/.env with random secrets (never overwrites an existing one)"
	@echo "dev-up         : Start infrastructure services (PostGIS, Valhalla, Photon, Caddy)"
	@echo "dev-down       : Stop infrastructure services"
	@echo "backend-check  : Format check, clippy (-D warnings) and unit tests for the Rust backend"
	@echo "backend-fmt    : Apply rustfmt to the Rust backend"
	@echo "backend-clippy : Run clippy (-D warnings) on the Rust backend"
	@echo "backend-test   : Run the Rust backend unit tests (no database needed)"
	@echo "compose-check  : Validate the Docker Compose stack and its security invariants"
	@echo "openapi        : Regenerate contracts/openapi.json from the backend code"
	@echo "backend-deny   : cargo-deny checks (bans, licenses, sources, advisories)"
	@echo "toolchain-check: Verify that every Rust toolchain pin agrees"
	@echo "android-build  : Compile Android application (assembleDebug)"
	@echo "data-build     : Run OSM data preparation pipeline (PMTiles, Valhalla, Photon)"

dev-env:
	bash scripts/gen-env.sh

# Local development: the dev overlay publishes the backend (8080) and PostGIS (5432) on 127.0.0.1 only,
# and CADDY_HTTP_BIND keeps the proxy off the local network too.
DEV_COMPOSE = docker compose -f infra/compose.yaml -f infra/compose.dev.yaml --env-file infra/.env

dev-up: dev-env
	CADDY_HTTP_BIND=127.0.0.1 GIT_COMMIT_HASH="$$(git rev-parse HEAD)" $(DEV_COMPOSE) up -d --build

dev-down:
	@if [ -f infra/.env ]; then \
		CADDY_HTTP_BIND=127.0.0.1 $(DEV_COMPOSE) down; \
	else \
		echo "infra/.env not found: nothing to stop (run 'make dev-env' first)"; \
	fi

# The recipes `cd backend` so rustup picks up backend/rust-toolchain.toml
# (it is resolved from the working directory, not from --manifest-path).
backend-check: toolchain-check
	cd backend && cargo fmt --all -- --check
	cd backend && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
	cd backend && cargo test --locked --workspace
	cd backend && cargo run --locked -q -p baze-app --bin export-openapi -- --check

backend-test:
	cd backend && cargo test --locked --workspace

backend-fmt:
	cd backend && cargo fmt --all

backend-clippy:
	cd backend && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

compose-check:
	bash scripts/compose-check.sh

openapi:
	cd backend && cargo run --locked -q -p baze-app --bin export-openapi

backend-deny:
	cd backend && cargo deny check

toolchain-check:
	bash scripts/check-toolchain-sync.sh

android-build:
	cd android && ./gradlew assembleDebug

data-build:
	bash data/scripts/01-download-extract.sh
	bash data/scripts/02-build-pmtiles.sh
	bash data/scripts/03-build-valhalla.sh
	bash data/scripts/04-build-photon.sh
	bash data/scripts/05-publish-static.sh
