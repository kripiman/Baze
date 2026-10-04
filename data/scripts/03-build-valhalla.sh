#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# [03] Builds the Valhalla routing graph, with elevation, into data/out/valhalla.
#
# The work happens inside the official Valhalla image (data/scripts/container/valhalla-build.sh), with the
# output mounted at the same /custom_files the engine uses at runtime, so valhalla.json is valid for both. The
# build fails if the elevation tiles cannot be downloaded (SRTM from the public AWS terrain-tiles bucket):
# a graph without elevation would route as if the whole city were flat and say nothing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
VALHALLA_DIR="${OUT_DIR}/valhalla"
INPUT_PBF="${OUT_DIR}/extract.osm.pbf"

# Pinned by digest, and the same one infra/compose.yaml runs: graph and engine always agree.
VALHALLA_IMAGE="ghcr.io/valhalla/valhalla:3.9.0@sha256:511c095b8caf393dccceb8b519ec96b6f85a0166b2288ba014a8a748acc5a63c"

die() {
    echo "ERROR: $*" >&2
    exit 1
}

command -v docker >/dev/null 2>&1 || die "Docker is required to run ${VALHALLA_IMAGE}"
[[ -f "${INPUT_PBF}" ]] || die "extract not found: ${INPUT_PBF}. Run 01-download-extract.sh first"

mkdir -p "${VALHALLA_DIR}"

echo "==> [03] Building the Valhalla graph with elevation"
echo "    Extract: ${INPUT_PBF}"
echo "    Output:  ${VALHALLA_DIR}"

# As the invoking user, so the output is yours and the engine (which runs unprivileged) can read it.
docker run --rm --user "$(id -u):$(id -g)" \
    -e HOME=/tmp \
    --tmpfs /tmp \
    -v "${VALHALLA_DIR}:/custom_files" \
    -v "${INPUT_PBF}:/input/extract.osm.pbf:ro" \
    -v "${SCRIPT_DIR}/container/valhalla-build.sh:/build.sh:ro" \
    "${VALHALLA_IMAGE}" bash /build.sh

echo "==> Valhalla graph ready in ${VALHALLA_DIR}"
