#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Security invariants of the production Compose file.

Reads the output of `docker compose config --format json` on stdin and exits non-zero, listing every
violation, if the stack drifts from the rules in AGENTS.md. Used by scripts/compose-check.sh.
"""

import json
import sys


def image_tag(image):
    """The tag of an image reference, or `latest` when it has none (digests count as pinned)."""
    if "@sha256:" in image:
        return "pinned-by-digest"
    last = image.split("/")[-1]
    return last.rsplit(":", 1)[-1] if ":" in last else "latest"


def check(cfg):
    errors = []
    services = cfg["services"]

    for name, svc in services.items():
        image = svc.get("image")
        # An image built from this repository is versioned by the commit baked into it, not by a tag.
        if image and not svc.get("build") and image_tag(image) == "latest":
            errors.append(f"{name}: image {image} is not pinned to a version")
        if name != "caddy" and svc.get("ports"):
            errors.append(f"{name}: publishes ports in the production file (only caddy may)")
        if "ALL" not in (svc.get("cap_drop") or []):
            errors.append(f"{name}: does not drop all capabilities")
        if "no-new-privileges:true" not in (svc.get("security_opt") or []):
            errors.append(f"{name}: missing no-new-privileges")
        if not ((svc.get("logging") or {}).get("options") or {}).get("max-size"):
            errors.append(f"{name}: log size is not bounded")
        if svc.get("privileged"):
            errors.append(f"{name}: runs privileged")
        if name != "caddy" and "public" in (svc.get("networks") or {}):
            errors.append(f"{name}: attached to the public network")

    if not cfg["networks"].get("internal", {}).get("internal"):
        errors.append("network `internal` must be internal: true")

    for mount in services["caddy"].get("volumes", []):
        if mount.get("source", "").rstrip("/").endswith("/data/out"):
            errors.append("caddy mounts the whole data/out directory (only data/out/public is public)")

    backend = services["backend"]
    if not backend.get("read_only"):
        errors.append("backend: root filesystem must be read-only")
    if str(backend.get("user")) != "10001:10001":
        errors.append("backend: must run as 10001:10001")
    if "GIT_COMMIT_HASH" in (backend.get("environment") or {}):
        errors.append("backend: GIT_COMMIT_HASH must come from the image build argument, not the environment")

    # Credential separation (ADR-0008): only the migration service holds the schema owner's credentials.
    backend_env = backend.get("environment") or {}
    migrate = services.get("migrate")
    for forbidden in ("MIGRATION_DATABASE_URL", "POSTGRES_PASSWORD", "POSTGRES_USER"):
        if forbidden in backend_env:
            errors.append(f"backend: must not receive {forbidden}")
    if migrate is None:
        errors.append("the migrate service is missing: nothing would create the schema")
    else:
        if backend_env.get("DATABASE_URL") == (migrate.get("environment") or {}).get("DATABASE_URL"):
            errors.append("backend and migrate use the same database credentials")
        if str(migrate.get("restart")) not in ("no", "None", "false", "False"):
            errors.append("migrate: must be a one-shot job (restart: no)")
        if migrate.get("ports"):
            errors.append("migrate: must not publish ports")
        if "migrate" not in (backend.get("depends_on") or {}):
            errors.append("backend: must wait for the migrate service")
        elif backend["depends_on"]["migrate"].get("condition") != "service_completed_successfully":
            errors.append("backend: must wait for migrate to complete successfully")
    postgis_mounts = [m.get("target", "") for m in services["postgis"].get("volumes", [])]
    if "/docker-entrypoint-initdb.d" not in postgis_mounts:
        errors.append("postgis: the role bootstrap script is not mounted")

    return errors


def main():
    cfg = json.load(sys.stdin)
    errors = check(cfg)
    if errors:
        print("\n".join(f"  - {e}" for e in errors))
        return 1
    print(f"  ok: {len(cfg['services'])} services checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
