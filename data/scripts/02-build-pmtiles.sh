#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# [02] Builds data/out/tiles.pmtiles (OpenMapTiles schema) from the extract with Planetiler.
#
#   PLANETILER_HEAP   JVM heap (default 4g; Planetiler recommends about half the size of the .pbf)
#   PLANETILER_ARGS   extra Planetiler arguments, split on spaces. The Monaco smoke test uses it to read the
#                     tiny data sources Planetiler keeps for its own tests instead of the ~1 GB real ones.
#   PLANETILER_JAR    path of a planetiler.jar to use with a local Java instead of Docker
#
# Planetiler downloads its other inputs (ocean polygons and Natural Earth, ~1 GB) into data/out/sources.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
INPUT_PBF="${OUT_DIR}/extract.osm.pbf"
OUTPUT_PMTILES="${OUT_DIR}/tiles.pmtiles"

# Pinned by digest: the same tiles come out of the same data, and a moved tag cannot change the build.
PLANETILER_IMAGE="ghcr.io/onthegomap/planetiler:0.10.2@sha256:cf32202dbc001a9ab4bc11534b642b13de3798179817da8558e567a3d13dd403"
PLANETILER_HEAP="${PLANETILER_HEAP:-4g}"

die() {
    echo "ERROR: $*" >&2
    exit 1
}

[[ -f "${INPUT_PBF}" ]] || die "extract not found: ${INPUT_PBF}. Run 01-download-extract.sh first"

# shellcheck disable=SC2206  # intentional word splitting of the extra arguments
EXTRA_ARGS=(${PLANETILER_ARGS:-})

echo "==> [02] Building PMTiles with Planetiler"
echo "    Input:  ${INPUT_PBF}"
echo "    Output: ${OUTPUT_PMTILES}"

if [[ -n "${PLANETILER_JAR:-}" ]]; then
    command -v java >/dev/null 2>&1 || die "PLANETILER_JAR needs a Java runtime"
    (cd "${OUT_DIR}" && java "-Xmx${PLANETILER_HEAP}" -jar "${PLANETILER_JAR}" \
        --osm-path="${INPUT_PBF}" --output="${OUTPUT_PMTILES}" --download --force "${EXTRA_ARGS[@]}")
elif command -v docker >/dev/null 2>&1; then
    # The container works in /data: sources and temporary files land in data/out/{sources,tmp}. Running as
    # the invoking user keeps every output owned by you instead of root.
    docker run --rm --user "$(id -u):$(id -g)" \
        -e JAVA_TOOL_OPTIONS="-Xmx${PLANETILER_HEAP}" \
        -v "${OUT_DIR}:/data" \
        "${PLANETILER_IMAGE}" \
        --osm-path=/data/extract.osm.pbf \
        --output=/data/tiles.pmtiles \
        --download --force "${EXTRA_ARGS[@]}"
else
    die "Docker (or PLANETILER_JAR with Java) is required to run Planetiler"
fi

[[ -s "${OUTPUT_PMTILES}" ]] || die "Planetiler produced no ${OUTPUT_PMTILES}"
# A PMTiles v3 archive starts with the bytes "PMTiles" followed by the version number 3.
magic="$(head -c 8 "${OUTPUT_PMTILES}" | od -An -c | tr -d ' \n')"
[[ "${magic}" == 'PMTiles003' ]] || die "${OUTPUT_PMTILES} is not a PMTiles v3 archive (header: ${magic})"

echo "==> PMTiles ready: ${OUTPUT_PMTILES} ($(wc -c < "${OUTPUT_PMTILES}" | tr -d ' ') bytes)"
