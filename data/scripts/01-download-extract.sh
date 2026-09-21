#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"

mkdir -p "${OUT_DIR}"

# URL por defecto para desarrollo (Geofabrik Chile)
# TODO(verify): Modificar esta URL si se requiere otra región o ciudad específica
DEFAULT_OSM_URL="https://download.geofabrik.de/south-america/chile-latest.osm.pbf"
OSM_EXTRACT_URL="${OSM_EXTRACT_URL:-$DEFAULT_OSM_URL}"
OUTPUT_FILE="${OUT_DIR}/extract.osm.pbf"

echo "==> [01] Descargando extracto OSM..."
echo "    URL: ${OSM_EXTRACT_URL}"
echo "    Destino: ${OUTPUT_FILE}"

if command -v curl >/dev/null 2>&1; then
    curl -L -C - --fail --output "${OUTPUT_FILE}" "${OSM_EXTRACT_URL}"
elif command -v wget >/dev/null 2>&1; then
    wget -c -O "${OUTPUT_FILE}" "${OSM_EXTRACT_URL}"
else
    echo "ERROR: Se requiere curl o wget para descargar el extracto." >&2
    exit 1
fi

echo "==> Descarga completada exitosamente: ${OUTPUT_FILE}"
