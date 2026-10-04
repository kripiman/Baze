#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# [05] Publishes what Caddy serves under /static from data/out/public:
#
#   tiles.pmtiles          the map the app downloads
#   tiles.pmtiles.sha256   its checksum in `sha256sum` format
#   manifest.json          size, checksum and provenance, small enough to fetch before the big file
#   styles/style.json      the map style (the app fills in the local path of the tiles)
#
# Only that directory is mounted into the proxy: the raw OSM extract, the Valhalla graph and the Photon index
# stay private even though they live next to it in data/out.
#
# Every file is replaced atomically and in place (never the directory: Caddy has it bind-mounted, and a swapped
# directory would keep serving the old one). The tiles go first and the manifest last, so a client that sees
# the new manifest always finds the tiles it describes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_DIR="$(cd "${DATA_DIR}/.." && pwd)"
OUT_DIR="${DATA_DIR}/out"
PUBLIC_DIR="${OUT_DIR}/public"
PMTILES="${OUT_DIR}/tiles.pmtiles"
STYLE="${DATA_DIR}/styles/style.json"

die() {
    echo "ERROR: $*" >&2
    exit 1
}

echo "==> [05] Publishing the static artifacts"

[[ -f "${PMTILES}" ]] || die "missing ${PMTILES}. Run data/scripts/02-build-pmtiles.sh first"
[[ -f "${STYLE}" ]] || die "missing ${STYLE}"
python3 "${REPO_DIR}/scripts/check-style.py" "${STYLE}" || die "the map style is not publishable"

mkdir -p "${PUBLIC_DIR}/styles"

# Copy next to the destination, then rename: rename is atomic, a half-written file is never served.
publish() {
    local source="$1" destination="$2"
    cp "${source}" "${destination}.tmp"
    mv -f "${destination}.tmp" "${destination}"
}

publish "${PMTILES}" "${PUBLIC_DIR}/tiles.pmtiles"
publish "${STYLE}" "${PUBLIC_DIR}/styles/style.json"

sha256="$(sha256sum "${PUBLIC_DIR}/tiles.pmtiles" | awk '{print $1}')"
bytes="$(wc -c < "${PUBLIC_DIR}/tiles.pmtiles" | tr -d ' ')"

printf '%s  tiles.pmtiles\n' "${sha256}" > "${PUBLIC_DIR}/tiles.pmtiles.sha256.tmp"
mv -f "${PUBLIC_DIR}/tiles.pmtiles.sha256.tmp" "${PUBLIC_DIR}/tiles.pmtiles.sha256"

# When the OSM data was downloaded (script 01 records it): what the map's date means.
osm_downloaded_at="$(sed -n 's/^downloaded_at=//p' "${OUT_DIR}/extract.osm.pbf.source" 2>/dev/null | head -n1)"
printf '{\n  "tiles": {\n    "file": "tiles.pmtiles",\n    "bytes": %s,\n    "sha256": "%s"\n  },\n  "osm_extract_downloaded_at": "%s",\n  "attribution": "© OpenStreetMap contributors (ODbL), © OpenMapTiles (CC-BY 4.0)",\n  "generated_at": "%s"\n}\n' \
    "${bytes}" "${sha256}" "${osm_downloaded_at:-unknown}" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "${PUBLIC_DIR}/manifest.json.tmp"
mv -f "${PUBLIC_DIR}/manifest.json.tmp" "${PUBLIC_DIR}/manifest.json"

python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "${PUBLIC_DIR}/manifest.json" || die "manifest.json is not valid JSON"

echo "==> Published in ${PUBLIC_DIR}:"
ls -lh "${PUBLIC_DIR}" "${PUBLIC_DIR}/styles"
