#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
PHOTON_DIR="${OUT_DIR}/photon"
INPUT_PBF="${OUT_DIR}/extract.osm.pbf"

mkdir -p "${PHOTON_DIR}"

echo "======================================================================"
echo "==> [04] Configuración e Ingesta del Geocodificador Photon"
echo "======================================================================"
cat << 'INFO'
IMPORTANTE SOBRE PHOTON Y .OSM.PBF:
Photon (Komoot) NO contiene un parser nativo para indexar un archivo
.osm.pbf directamente en su binario estándar. Photon fue diseñado para
consumir datos estructurados desde una base de datos Nominatim
(PostgreSQL + osm2pgsql) o cargar dumps pre-indexados.

Existen dos estrategias soportadas:

[Estrategia A] Descarga de extracto precalculado (Recomendado para Dev)
  Descargar el archivo photon_data pre-indexado para el país/región
  publicado por el proyecto Photon: https://photon.komoot.io/data/

[Estrategia B] Pipeline completo desde extract.osm.pbf vía Nominatim
  1. Levantar instancia de Nominatim con la base de datos PostgreSQL.
  2. Importar el extracto:
     nominatim import --osm-file data/out/extract.osm.pbf
  3. Ejecutar la herramienta de importación de Photon:
     java -jar photon.jar -nominatim-import \
       -host localhost -port 5432 -database nominatim \
       -user nominatim -password <password> -data-dir data/out/photon
INFO
echo "======================================================================"

# TODO(verify): URL del dump precalculado para la región elegida (ej. Chile / Sudamérica)
PHOTON_DUMP_URL="${PHOTON_DUMP_URL:-}"

if [[ -n "${PHOTON_DUMP_URL}" ]]; then
    echo "==> Descargando dump precalculado de Photon desde: ${PHOTON_DUMP_URL}"
    curl -L --fail -o "${OUT_DIR}/photon_dump.tar.gz" "${PHOTON_DUMP_URL}"
    tar -xzf "${OUT_DIR}/photon_dump.tar.gz" -C "${PHOTON_DIR}"
    echo "==> Dump de Photon descomprimido en: ${PHOTON_DIR}"
else
    echo "AVISO: No se definió PHOTON_DUMP_URL."
    echo "Para desarrollo local, se genera un marcador esqueleto en ${PHOTON_DIR}."
    echo "Para habilitar búsqueda real, descarga un dump o corre el pipeline Nominatim."
    touch "${PHOTON_DIR}/.photon_data_placeholder"
fi

echo "==> Proceso de Photon finalizado."
