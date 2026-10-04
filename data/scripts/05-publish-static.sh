#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Copies the artifacts that Caddy serves publicly under /static into data/out/public.
# Only that directory is mounted into the proxy: the raw OSM extract, the Valhalla graph and the
# Photon index stay private even though they live next to it in data/out.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
PUBLIC_DIR="${OUT_DIR}/public"
PMTILES="${OUT_DIR}/tiles.pmtiles"

echo "======================================================================"
echo "==> [05] Publicación de artefactos estáticos"
echo "======================================================================"

if [[ ! -f "${PMTILES}" ]]; then
    echo "ERROR: falta ${PMTILES}. Ejecuta antes data/scripts/02-build-pmtiles.sh." >&2
    exit 1
fi

mkdir -p "${PUBLIC_DIR}/styles"
cp "${PMTILES}" "${PUBLIC_DIR}/tiles.pmtiles"
cp -R "${DATA_DIR}/styles/." "${PUBLIC_DIR}/styles/"

echo "==> Publicado en ${PUBLIC_DIR}:"
ls -lh "${PUBLIC_DIR}"
