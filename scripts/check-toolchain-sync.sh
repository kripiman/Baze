#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Verifies that every place that pins the Rust toolchain agrees, and that the pinned
# version is not older than the MSRV declared in Cargo.toml.
#
#   backend/rust-toolchain.toml       channel = "X.Y.Z"
#   backend/Dockerfile                ARG RUST_VERSION=X.Y.Z
#   .github/workflows/backend.yml     toolchain: X.Y.Z   (main job)
#   backend/Cargo.toml                rust-version = "X.Y"  (MSRV floor, used by the msrv job)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

toolchain_file="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' backend/rust-toolchain.toml)"
dockerfile="$(sed -n 's/^ARG RUST_VERSION=\(.*\)$/\1/p' backend/Dockerfile)"
msrv="$(sed -n 's/^rust-version *= *"\([^"]*\)".*/\1/p' backend/Cargo.toml)"
# Every `toolchain:` pin in the workflow except the MSRV job's (which pins the floor on purpose).
workflow="$(sed -n 's/^ *toolchain: *\([0-9][0-9.]*\).*/\1/p' .github/workflows/backend.yml | grep -vx "${msrv}.0" | sort -u || true)"
workflow_msrv="$(sed -n 's/.*cargo +\([0-9][0-9.]*\) .*/\1/p' .github/workflows/backend.yml | sort -u)"

status=0
echo "rust-toolchain.toml : ${toolchain_file:-<missing>}"
echo "Dockerfile          : ${dockerfile:-<missing>}"
echo "backend.yml (main)  : $(echo "${workflow:-<missing>}" | tr '\n' ' ')"
echo "Cargo.toml MSRV     : ${msrv:-<missing>}"
echo "backend.yml (msrv)  : $(echo "${workflow_msrv:-<missing>}" | tr '\n' ' ')"

for v in "$toolchain_file" "$dockerfile" "$msrv"; do
  if [ -z "$v" ]; then
    echo "error: could not read a toolchain version (see above)" >&2
    exit 1
  fi
done

# The Docker image tag and the workflow must use exactly the pinned toolchain.
# Compare as multi-line strings so a second, different pin in the workflow is caught.
if [ "$dockerfile" != "$toolchain_file" ]; then
  echo "error: Dockerfile RUST_VERSION ($dockerfile) != rust-toolchain.toml ($toolchain_file)" >&2
  status=1
fi
if [ "$workflow" != "$toolchain_file" ]; then
  echo "error: backend.yml toolchain pins ($(echo "$workflow" | tr '\n' ' ')) != rust-toolchain.toml ($toolchain_file)" >&2
  status=1
fi

# The msrv job must test the declared floor (X.Y -> X.Y.0).
if [ "$workflow_msrv" != "${msrv}.0" ]; then
  echo "error: backend.yml msrv job uses '$workflow_msrv' but Cargo.toml rust-version is '$msrv' (expected ${msrv}.0)" >&2
  status=1
fi

# The pinned toolchain must satisfy the MSRV (compare major.minor numerically).
pin_minor="$(echo "$toolchain_file" | cut -d. -f1-2)"
if [ "$(printf '%s\n%s\n' "$msrv" "$pin_minor" | sort -V | head -n1)" != "$msrv" ]; then
  echo "error: pinned toolchain $toolchain_file is older than the MSRV $msrv" >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "ok: toolchain pins are consistent"
fi
exit "$status"
