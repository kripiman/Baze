# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later

.PHONY: help dev-up dev-down backend-check android-build data-build backend-test backend-fmt backend-clippy backend-deny toolchain-check

help:
	@echo "Baze Monorepo Management"
	@echo "------------------------"
	@echo "dev-up         : Start infrastructure services (PostGIS, Valhalla, Photon, Caddy)"
	@echo "dev-down       : Stop infrastructure services"
	@echo "backend-check  : Format check, clippy (-D warnings) and unit tests for the Rust backend"
	@echo "backend-fmt    : Apply rustfmt to the Rust backend"
	@echo "backend-clippy : Run clippy (-D warnings) on the Rust backend"
	@echo "backend-test   : Run the Rust backend unit tests (no database needed)"
	@echo "backend-deny   : cargo-deny checks (bans, licenses, sources, advisories)"
	@echo "toolchain-check: Verify that every Rust toolchain pin agrees"
	@echo "android-build  : Compile Android application (assembleDebug)"
	@echo "data-build     : Run OSM data preparation pipeline (PMTiles, Valhalla, Photon)"

dev-up:
	@if [ ! -f infra/.env ]; then \
		echo "Notice: infra/.env not found, using infra/.env.example for local development"; \
		docker compose -f infra/compose.yaml --env-file infra/.env.example up -d; \
	else \
		docker compose -f infra/compose.yaml --env-file infra/.env up -d; \
	fi

dev-down:
	@if [ ! -f infra/.env ]; then \
		docker compose -f infra/compose.yaml --env-file infra/.env.example down; \
	else \
		docker compose -f infra/compose.yaml --env-file infra/.env down; \
	fi

# The recipes `cd backend` so rustup picks up backend/rust-toolchain.toml
# (it is resolved from the working directory, not from --manifest-path).
backend-check: toolchain-check
	cd backend && cargo fmt --all -- --check
	cd backend && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
	cd backend && cargo test --locked --workspace

backend-test:
	cd backend && cargo test --locked --workspace

backend-fmt:
	cd backend && cargo fmt --all

backend-clippy:
	cd backend && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

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
