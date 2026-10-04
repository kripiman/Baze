#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# [01] Downloads the OpenStreetMap extract every other step is built from.
#
#   OSM_EXTRACT_URL          https URL of a .osm.pbf (default: Chile, from Geofabrik)
#   OSM_EXTRACT_SHA256       optional sha256 the file must have (use it for a URL without a published .md5)
#   OSM_ALLOW_UNVERIFIED=1   accept a file nothing could be checked against (never for production data)
#
# The download is verified before it replaces data/out/extract.osm.pbf, so a truncated or tampered file
# never reaches the build steps. Geofabrik publishes `<file>.md5` next to each extract.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
OUTPUT_FILE="${OUT_DIR}/extract.osm.pbf"
PART_FILE="${OUTPUT_FILE}.part"

DEFAULT_OSM_URL="https://download.geofabrik.de/south-america/chile-latest.osm.pbf"
OSM_EXTRACT_URL="${OSM_EXTRACT_URL:-$DEFAULT_OSM_URL}"

die() {
    echo "ERROR: $*" >&2
    exit 1
}

case "${OSM_EXTRACT_URL}" in
    https://*) ;;
    *) die "OSM_EXTRACT_URL must be an https URL (got '${OSM_EXTRACT_URL}')" ;;
esac
command -v curl >/dev/null 2>&1 || die "curl is required"
command -v md5sum >/dev/null 2>&1 || die "md5sum is required"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"

mkdir -p "${OUT_DIR}"

# https only, also across redirects; transient failures are retried.
CURL=(curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2
    --retry 5 --retry-delay 5 --retry-connrefused)

echo "==> [01] Downloading the OSM extract"
echo "    URL:     ${OSM_EXTRACT_URL}"
echo "    Output:  ${OUTPUT_FILE}"

# Resume a partial download; if the server refuses (or the file was already complete) start over once.
if ! "${CURL[@]}" --continue-at - --output "${PART_FILE}" "${OSM_EXTRACT_URL}"; then
    echo "    resuming failed; downloading from the start"
    rm -f "${PART_FILE}"
    "${CURL[@]}" --output "${PART_FILE}" "${OSM_EXTRACT_URL}" || {
        rm -f "${PART_FILE}"
        die "download failed"
    }
fi

# A PBF starts with a length-prefixed OSMHeader blob: catches an HTML error page saved as a file.
head -c 128 "${PART_FILE}" | grep -aq OSMHeader || {
    rm -f "${PART_FILE}"
    die "the downloaded file is not an OSM PBF"
}

actual_md5="$(md5sum "${PART_FILE}" | awk '{print $1}')"
actual_sha256="$(sha256sum "${PART_FILE}" | awk '{print $1}')"
verified="nothing (unverified)"

if [[ -n "${OSM_EXTRACT_SHA256:-}" ]]; then
    if [[ "${actual_sha256}" != "${OSM_EXTRACT_SHA256}" ]]; then
        rm -f "${PART_FILE}"
        die "sha256 mismatch: expected ${OSM_EXTRACT_SHA256}, got ${actual_sha256}"
    fi
    verified="sha256 (OSM_EXTRACT_SHA256)"
elif expected_md5="$("${CURL[@]}" "${OSM_EXTRACT_URL}.md5" 2>/dev/null | awk 'NR==1 {print $1}')" && [[ -n "${expected_md5}" ]]; then
    if [[ "${actual_md5}" != "${expected_md5}" ]]; then
        rm -f "${PART_FILE}"
        die "md5 mismatch against ${OSM_EXTRACT_URL}.md5: expected ${expected_md5}, got ${actual_md5}"
    fi
    verified="md5 (${OSM_EXTRACT_URL##*/}.md5 published by the server)"
elif [[ "${OSM_ALLOW_UNVERIFIED:-0}" == "1" ]]; then
    echo "WARNING: nothing to verify the download against; continuing because OSM_ALLOW_UNVERIFIED=1" >&2
else
    rm -f "${PART_FILE}"
    die "no checksum to verify against: the server publishes no .md5. Set OSM_EXTRACT_SHA256 (or OSM_ALLOW_UNVERIFIED=1 for throwaway data)"
fi

mv -f "${PART_FILE}" "${OUTPUT_FILE}"

# Provenance: which data the artifacts were built from (it must be attributable, see ATTRIBUTION.md).
{
    echo "url=${OSM_EXTRACT_URL}"
    echo "downloaded_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "bytes=$(wc -c < "${OUTPUT_FILE}" | tr -d ' ')"
    echo "md5=${actual_md5}"
    echo "sha256=${actual_sha256}"
    echo "verified_by=${verified}"
} > "${OUTPUT_FILE}.source"

echo "==> Download verified by ${verified}: ${OUTPUT_FILE}"
