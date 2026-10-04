#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Runs INSIDE the Valhalla image (started by data/scripts/03-build-valhalla.sh) and builds the routing graph
# with elevation. The paths are the ones the runtime container uses too, so valhalla.json is valid in both:
#
#   /custom_files                   output (becomes data/out/valhalla on the host; /custom_files in the engine)
#   /input/extract.osm.pbf          the OSM extract (read-only)
#
# Order matters: the graph is built up to the `build` stage, the elevation tiles for exactly the area it
# covers are downloaded, and only then the `enhance` stage runs, which is the one that turns heights into
# the grades bicycle costing uses. valhalla_build_elevation does NOT fail when it cannot download a tile
# (it only logs it), and the graph would then be built flat without a word; so the result is checked here and
# the build fails if any elevation tile is missing.
set -euo pipefail

CUSTOM_FILES="${CUSTOM_FILES:-/custom_files}"
INPUT_PBF="${INPUT_PBF:-/input/extract.osm.pbf}"
CONFIG="${CUSTOM_FILES}/valhalla.json"
TILE_DIR="${CUSTOM_FILES}/valhalla_tiles"
TILE_TAR="${CUSTOM_FILES}/valhalla_tiles.tar"
ELEVATION_DIR="${CUSTOM_FILES}/elevation_tiles"

die() {
    echo "ERROR: $*" >&2
    exit 1
}

[[ -f "${INPUT_PBF}" ]] || die "extract not found: ${INPUT_PBF}"

# A rebuild starts from nothing: stale tiles of another extract must never be mixed in. Downloaded elevation
# tiles are kept (they are the same for the same area and are large).
rm -rf "${TILE_DIR}" "${TILE_TAR}" "${CONFIG}"
mkdir -p "${ELEVATION_DIR}"

echo "==> [03.1] Writing the configuration"
valhalla_build_config \
    --mjolnir-tile-dir "${TILE_DIR}" \
    --mjolnir-tile-extract "${TILE_TAR}" \
    --additional-data-elevation "${ELEVATION_DIR}" > "${CONFIG}"

echo "==> [03.2] Building the graph (initialize to build)"
valhalla_build_tiles -c "${CONFIG}" -e build "${INPUT_PBF}"

echo "==> [03.3] Downloading the elevation tiles the graph covers"
valhalla_build_elevation --from-tiles --decompress -c "${CONFIG}" -v

echo "==> [03.4] Checking that every elevation tile is there"
python3 - "${TILE_DIR}" "${ELEVATION_DIR}" <<'PY'
import math
import pathlib
import sys

tile_dir, elevation_dir = (pathlib.Path(arg) for arg in sys.argv[1:3])
LOCAL_TILE_DEGREES = 0.25  # level 2 of the Valhalla hierarchy: the tiles that hold every road
VALID_SIZES = {3601 * 3601 * 2, 1201 * 1201 * 2}  # SRTM1 and SRTM3, as .hgt (big-endian int16)

width = int(360 / LOCAL_TILE_DEGREES)
needed = set()
for graph_tile in (tile_dir / "2").rglob("*.gph"):
    tile_id = int("".join(graph_tile.parent.relative_to(tile_dir / "2").parts) + graph_tile.stem)
    x = math.floor((tile_id % width) * LOCAL_TILE_DEGREES - 180)
    y = math.floor((tile_id // width) * LOCAL_TILE_DEGREES - 90)
    hemisphere, meridian = ("S" if y < 0 else "N"), ("W" if x < 0 else "E")
    needed.add(f"{hemisphere}{abs(y):02d}/{hemisphere}{abs(y):02d}{meridian}{abs(x):03d}.hgt")

if not needed:
    sys.exit("the graph has no level-2 tiles: nothing was built from the extract")

problems = []
for name in sorted(needed):
    path = elevation_dir / name
    if not path.is_file():
        problems.append(f"missing {name}")
    elif path.stat().st_size not in VALID_SIZES:
        problems.append(f"{name} has {path.stat().st_size} bytes, not an SRTM tile")
if problems:
    sys.exit("elevation data is incomplete, the graph would be flat:\n  " + "\n  ".join(problems))
print(f"    {len(needed)} elevation tile(s) present and well-formed")
PY

echo "==> [03.5] Enhancing the graph with the elevation"
valhalla_build_tiles -c "${CONFIG}" -s enhance "${INPUT_PBF}"

echo "==> [03.6] Packing the tiles into one file"
valhalla_build_extract -c "${CONFIG}" -v
[[ -s "${TILE_TAR}" ]] || die "no tile extract was produced"

# The engine reads the extract only; the loose tiles are the same data twice.
if [[ "${KEEP_TILE_DIR:-0}" != "1" ]]; then
    rm -rf "${TILE_DIR}"
fi

echo "==> Valhalla data ready in ${CUSTOM_FILES} ($(wc -c < "${TILE_TAR}" | tr -d ' ') bytes of tiles)"
