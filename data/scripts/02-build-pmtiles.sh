#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
INPUT_PBF="${OUT_DIR}/extract.osm.pbf"
OUTPUT_PMTILES="${OUT_DIR}/tiles.pmtiles"

if [[ ! -f "${INPUT_PBF}" ]]; then
    echo "ERROR: No se encontró el extracto '${INPUT_PBF}'. Ejecuta primero 01-download-extract.sh" >&2
    exit 1
fi

echo "==> [02] Generando PMTiles con Planetiler..."
echo "    Entrada: ${INPUT_PBF}"
echo "    Salida:  ${OUTPUT_PMTILES}"

# Versión fijada de Planetiler
# TODO(verify): Verificar última versión estable compatible de Planetiler (0.8.2)
PLANETILER_IMAGE="ghcr.io/onthegomap/planetiler:0.8.2"

if command -v docker >/dev/null 2>&1; then
    docker run -e JAVA_TOOL_OPTIONS="-Xmx4g" \
        -v "${OUT_DIR}:/data" \
        --rm "${PLANETILER_IMAGE}" \
        --osm-path="/data/extract.osm.pbf" \
        --output="/data/tiles.pmtiles" \
        --download
elif command -v java >/dev/null 2>&1 && [[ -f "${OUT_DIR}/planetiler.jar" ]]; then
    java -Xmx4g -jar "${OUT_DIR}/planetiler.jar" \
        --osm-path="${INPUT_PBF}" \
        --output="${OUTPUT_PMTILES}" \
        --download
else
    echo "ERROR: Se requiere Docker o 'planetiler.jar' en ${OUT_DIR} para ejecutar la compilación de tiles." >&2
    exit 1
fi

echo "==> PMTiles generado exitosamente: ${OUTPUT_PMTILES}"
