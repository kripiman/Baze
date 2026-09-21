# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later

.PHONY: help dev-up dev-down backend-check android-build data-build backend-test backend-fmt backend-clippy

help:
	@echo "Baze Monorepo Management"
	@echo "------------------------"
	@echo "dev-up         : Start infrastructure services (PostGIS, Valhalla, Photon, Caddy)"
	@echo "dev-down       : Stop infrastructure services"
	@echo "backend-check  : Validate, format check and clippy for Rust backend"
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

backend-check:
	cargo fmt --manifest-path backend/Cargo.toml --all -- --check
	cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
	cargo test --manifest-path backend/Cargo.toml --workspace

backend-fmt:
	cargo fmt --manifest-path backend/Cargo.toml --all

backend-clippy:
	cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings

android-build:
	cd android && ./gradlew assembleDebug

data-build:
	bash data/scripts/01-download-extract.sh
	bash data/scripts/02-build-pmtiles.sh
	bash data/scripts/03-build-valhalla.sh
	bash data/scripts/04-build-photon.sh
