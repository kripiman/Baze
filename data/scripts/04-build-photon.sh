#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# [04] Builds the Photon search index into data/out/photon/photon_data. Photon cannot read an .osm.pbf: its
# data comes from a Nominatim database or from a dump. Choose ONE source:
#
#   PHOTON_IMPORT_FILE=/path/dump.jsonl
#       Imports a Photon JSON dump (docs: komoot/photon, json-dump-format). data/fixtures/photon-monaco.jsonl is
#       a six-place example; a real one is exported from a Nominatim import with `photon dump-nominatim-db`.
#       PHOTON_LANGUAGES (default "es,en") are the languages kept for names; PHOTON_IMPORT_HEAP (default 2g).
#
#   PHOTON_DUMP_URL=https://download1.graphhopper.com/public/.../photon-db-<cc>-1.0-latest.tar.bz2
#   PHOTON_DUMP_SHA256=<sha256 of that file>
#       Downloads a ready-made database (GraphHopper publishes weekly ones per country; the "1.0" must be the
#       database format of the Photon version in infra/photon/Dockerfile). The checksum is mandatory unless
#       PHOTON_ALLOW_UNVERIFIED=1, which is for throwaway data only.
#
# The new index is built next to the old one and swapped in at the end; never unpack in place (Photon's own
# documentation warns it corrupts the data). Restart the photon container afterwards.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_DIR="$(cd "${DATA_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
PHOTON_DIR="${OUT_DIR}/photon"
STAGING_DIR="${OUT_DIR}/photon.staging"
OLD_DIR="${OUT_DIR}/photon.old"
IMAGE="baze-photon"

die() {
    echo "ERROR: $*" >&2
    exit 1
}

command -v docker >/dev/null 2>&1 || die "Docker is required (the Photon image runs the import)"

if [[ -n "${PHOTON_IMPORT_FILE:-}" && -n "${PHOTON_DUMP_URL:-}" ]]; then
    die "set only one of PHOTON_IMPORT_FILE and PHOTON_DUMP_URL"
fi
if [[ -z "${PHOTON_IMPORT_FILE:-}" && -z "${PHOTON_DUMP_URL:-}" ]]; then
    die "no Photon data source: set PHOTON_IMPORT_FILE (a JSON dump) or PHOTON_DUMP_URL and PHOTON_DUMP_SHA256 (see the header of this script)"
fi

echo "==> [04] Building the Photon search index"
rm -rf "${STAGING_DIR}" "${OLD_DIR}"
mkdir -p "${STAGING_DIR}"

if [[ -n "${PHOTON_IMPORT_FILE:-}" ]]; then
    [[ -f "${PHOTON_IMPORT_FILE}" ]] || die "dump not found: ${PHOTON_IMPORT_FILE}"
    import_file="$(cd "$(dirname "${PHOTON_IMPORT_FILE}")" && pwd)/$(basename "${PHOTON_IMPORT_FILE}")"
    echo "    Source: JSON dump ${import_file}"

    docker build --quiet --tag "${IMAGE}" "${REPO_DIR}/infra/photon" >/dev/null
    docker run --rm --user "$(id -u):$(id -g)" \
        -e JAVA_TOOL_OPTIONS="-Xmx${PHOTON_IMPORT_HEAP:-2g} -Duser.home=/tmp" \
        --tmpfs /tmp \
        -v "${STAGING_DIR}:/photon" \
        -v "${import_file}:/import/dump.jsonl:ro" \
        "${IMAGE}" import -import-file /import/dump.jsonl -data-dir /photon \
        -languages "${PHOTON_LANGUAGES:-es,en}"
    source_note="json-dump ${PHOTON_IMPORT_FILE##*/}"
else
    case "${PHOTON_DUMP_URL}" in
        https://*) ;;
        *) die "PHOTON_DUMP_URL must be an https URL" ;;
    esac
    command -v curl >/dev/null 2>&1 || die "curl is required"
    command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"
    command -v bzip2 >/dev/null 2>&1 || die "bzip2 is required to unpack the database"
    echo "    Source: ready-made database ${PHOTON_DUMP_URL}"

    archive="${OUT_DIR}/photon-db.tar.bz2"
    curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --retry 5 --retry-delay 5 --retry-connrefused --output "${archive}" "${PHOTON_DUMP_URL}"
    actual="$(sha256sum "${archive}" | awk '{print $1}')"
    if [[ -n "${PHOTON_DUMP_SHA256:-}" ]]; then
        [[ "${actual}" == "${PHOTON_DUMP_SHA256}" ]] || {
            rm -f "${archive}"
            die "sha256 mismatch: expected ${PHOTON_DUMP_SHA256}, got ${actual}"
        }
    elif [[ "${PHOTON_ALLOW_UNVERIFIED:-0}" == "1" ]]; then
        echo "WARNING: PHOTON_DUMP_SHA256 not set; continuing because PHOTON_ALLOW_UNVERIFIED=1 (sha256 ${actual})" >&2
    else
        rm -f "${archive}"
        die "PHOTON_DUMP_SHA256 is required (or PHOTON_ALLOW_UNVERIFIED=1 for throwaway data); the file's sha256 is ${actual}"
    fi
    bzip2 -cd "${archive}" | tar -x -C "${STAGING_DIR}"
    rm -f "${archive}"
    source_note="ready-made database ${PHOTON_DUMP_URL##*/} (sha256 ${actual})"
fi

[[ -d "${STAGING_DIR}/photon_data" ]] || die "the build produced no ${STAGING_DIR}/photon_data"

{
    echo "source=${source_note}"
    echo "built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "${STAGING_DIR}/SOURCE.txt"

# Swap the new index in; the old one is kept only until the swap has succeeded.
if [[ -d "${PHOTON_DIR}" ]]; then
    mv "${PHOTON_DIR}" "${OLD_DIR}"
fi
mv "${STAGING_DIR}" "${PHOTON_DIR}"
rm -rf "${OLD_DIR}"

echo "==> Photon index ready in ${PHOTON_DIR}. Restart the photon container to load it."
