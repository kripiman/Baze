#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
VALHALLA_DIR="${OUT_DIR}/valhalla"
INPUT_PBF="${OUT_DIR}/extract.osm.pbf"

mkdir -p "${VALHALLA_DIR}"

if [[ ! -f "${INPUT_PBF}" ]]; then
    echo "ERROR: No se encontró el extracto '${INPUT_PBF}'. Ejecuta primero 01-download-extract.sh" >&2
    exit 1
fi

echo "==> [03] Construyendo grafo de Valhalla con elevación para perfiles ciclistas..."
echo "    Directorio de salida: ${VALHALLA_DIR}"

# Imagen fijada de Valhalla
VALHALLA_IMAGE="gisops/valhalla:3.4.0"

if command -v docker >/dev/null 2>&1; then
    # 1. Construir configuración base de Valhalla
    docker run --rm \
        -v "${OUT_DIR}:/data" \
        "${VALHALLA_IMAGE}" \
        valhalla_build_config \
        --tile-extract /data/valhalla/valhalla_tiles.tar \
        --tile-dir /data/valhalla/valhalla_tiles \
        --conf /data/valhalla/valhalla.json

    # 2. Descargar e incorporar datos de elevación para cálculo de pendientes ciclistas
    # TODO(verify): Ajustar proveedores de elevación si no se dispone de token Mapzen/SRTM
    echo "==> [03.1] Descargando mosaicos de elevación..."
    docker run --rm \
        -v "${OUT_DIR}:/data" \
        "${VALHALLA_IMAGE}" \
        valhalla_build_elevation \
        -c /data/valhalla/valhalla.json \
        /data/extract.osm.pbf || echo "ADVERTENCIA: Elevación no disponible sin credenciales externas, continuando con grafo plano."

    # 3. Compilar teselas del grafo de ruteo
    echo "==> [03.2] Compilando teselas del grafo..."
    docker run --rm \
        -v "${OUT_DIR}:/data" \
        "${VALHALLA_IMAGE}" \
        valhalla_build_tiles \
        -c /data/valhalla/valhalla.json \
        /data/extract.osm.pbf

    # 4. Empaquetar extracto para servicio rápido
    echo "==> [03.3] Generando paquete final de teselas..."
    docker run --rm \
        -v "${OUT_DIR}:/data" \
        "${VALHALLA_IMAGE}" \
        valhalla_build_extract \
        -c /data/valhalla/valhalla.json -v
else
    echo "ERROR: Se requiere Docker con la imagen ${VALHALLA_IMAGE} para compilar el grafo de Valhalla." >&2
    exit 1
fi

echo "==> Grafo de Valhalla generado exitosamente en: ${VALHALLA_DIR}"
