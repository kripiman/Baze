#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Smoke test of the REAL routing and search engines through the backend, on the Monaco data.

The `data-smoke` workflow builds the Monaco extract with the project's own pipeline (data/scripts), starts the
stack with `--profile engines` and runs this script against it. It proves what the simulated engines of the
unit and e2e tests cannot: that Valhalla (with elevation) and Photon, built from real data by our scripts, answer
what the backend's clients expect, and that a confirmed closure is avoided by the real engine.

    python3 scripts/e2e/engines_smoke.py --backend http://127.0.0.1:8080 \\
        --psql "docker exec -i baze-postgis psql -U baze_user -d baze_db -X -q -t -A"

`--psql` is the command (split like a shell would) that runs SQL given with `-c` against the stack's database;
it is only used to store and remove road closures, which the public API does not let one account confirm alone.
Standard library only.
"""

import argparse
import json
import math
import shlex
import subprocess
import sys
import urllib.error
import urllib.request

# Places of data/fixtures/photon-monaco.jsonl, and the streets around them.
CASINO = (7.4276, 43.7394)
PORT = (7.4215, 43.7345)

FAILURES = []


def haversine_m(a, b):
    lat1, lat2 = math.radians(a[1]), math.radians(b[1])
    dlat, dlon = lat2 - lat1, math.radians(b[0] - a[0])
    h = math.sin(dlat / 2) ** 2 + math.cos(lat1) * math.cos(lat2) * math.sin(dlon / 2) ** 2
    return 2 * 6371008.8 * math.asin(math.sqrt(h))


class Api:
    def __init__(self, base):
        self.base = base.rstrip("/")

    def request(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.base + path, data, {"Content-Type": "application/json"}, method=method)
        try:
            with urllib.request.urlopen(req, timeout=60) as response:
                return response.status, json.loads(response.read() or b"null")
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                return error.code, json.loads(raw or b"null")
            except ValueError:
                return error.code, raw.decode(errors="replace")

    def route(self, origin, destination):
        point = lambda c: {"type": "Point", "coordinates": list(c)}
        return self.request("POST", "/api/v1/routing/route", {"origin": point(origin), "destination": point(destination)})

    def search(self, text, limit=5):
        from urllib.parse import quote

        return self.request("GET", f"/api/v1/geocoding/search?q={quote(text)}&limit={limit}")


class Database:
    def __init__(self, psql):
        self.command = shlex.split(psql)

    def run(self, sql):
        out = subprocess.run(self.command + ["-v", "ON_ERROR_STOP=1", "-c", sql], capture_output=True, text=True, timeout=60)
        if out.returncode != 0:
            raise RuntimeError(f"psql failed: {out.stderr.strip()[:300]}")
        return out.stdout.strip()

    def add_closure(self, point, status="confirmed"):
        """A road closure of the given status, created by a new account (the voting rules are covered elsewhere)."""
        return self.run(
            "WITH a AS (INSERT INTO accounts DEFAULT VALUES RETURNING id) "
            "INSERT INTO hazards (creator_account_id, category, hazard_type, status, geom, expires_at) "
            f"SELECT a.id, 'road_closed', 'blocking', '{status}', ST_SetSRID(ST_MakePoint({point[0]}, {point[1]}), 4326), "
            "now() + interval '1 day' FROM a RETURNING id"
        ).splitlines()[0]

    def remove(self, hazard_id):
        self.run(f"DELETE FROM hazards WHERE id = '{hazard_id}'")


def check(label, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'}  {label}" + (f"  [{detail}]" if detail else ""))
    if not ok:
        FAILURES.append(label)
    return ok


def min_distance_to(route, point):
    return min(haversine_m(tuple(vertex), point) for vertex in route["geometry"]["coordinates"])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--backend", required=True, help="base URL of the backend, e.g. http://127.0.0.1:8080")
    parser.add_argument("--psql", required=True, help="command that runs SQL with -c against the stack's database")
    args = parser.parse_args()
    api, db = Api(args.backend), Database(args.psql)

    status, body = api.request("GET", "/health")
    check("the backend is up", status == 200, f"status={status}")

    # ----------------------------------------------------------------- address search (Photon)
    status, found = api.search("Casino")
    hit = next((item for item in found if "Casino" in item["name"]), None) if status == 200 else None
    check("search finds the casino", hit is not None, f"status={status} {str(found)[:100]}")
    if hit:
        distance = haversine_m(tuple(hit["location"]["coordinates"]), CASINO)
        check("the casino is where the data says it is", distance < 200, f"{distance:.0f} m away")
    status, found = api.search("Monaco")
    check("search by city name answers", status == 200 and len(found) >= 1, f"status={status} results={len(found) if status == 200 else '-'}")
    status, found = api.search("zzzzzzqqqq")
    check("a search without matches is an empty list, not an error", status == 200 and found == [], f"status={status}")
    status, found = api.search("casino & limit=1000")
    check("hostile search text is just text", status == 200, f"status={status}")

    # ----------------------------------------------------------------- routing (Valhalla)
    status, base = api.route(CASINO, PORT)
    if not check("a bicycle route is found between two places in Monaco", status == 200, f"status={status} {str(base)[:120]}"):
        return finish()
    check(
        "the route has a plausible length and detail",
        300 <= base["distance_meters"] <= 4000 and len(base["geometry"]["coordinates"]) >= 20 and len(base["maneuvers"]) >= 2,
        f"{base['distance_meters']:.0f} m, {len(base['geometry']['coordinates'])} points, {len(base['maneuvers'])} maneuvers",
    )
    climb = base["ascent_meters"] + base["descent_meters"]
    check(
        "the elevation is real: the route climbs and descends (a flat graph would report zero)",
        climb > 10,
        f"ascent={base['ascent_meters']:.1f} m descent={base['descent_meters']:.1f} m",
    )
    check("maneuver text is in Spanish", any(word in base["maneuvers"][0]["instruction"].lower() for word in ("hacia", "gire", "siga", "salga", "bicicleta")), base["maneuvers"][0]["instruction"])

    vertices = base["geometry"]["coordinates"]
    middle = tuple(vertices[len(vertices) // 2])

    # An unconfirmed closure must not move anybody's route.
    pending = db.add_closure(middle, status="unconfirmed")
    try:
        status, same = api.route(CASINO, PORT)
        check("an unconfirmed closure does not change the route", status == 200 and same["geometry"] == base["geometry"], f"status={status}")
    finally:
        db.remove(pending)

    # A confirmed closure must be avoided by the real engine. Which streets have an alternative depends on the
    # data, so several places along the route are tried: what must hold is that no answer ever crosses the
    # closure, and that at least one place yields a real detour.
    detours = 0
    for fraction in (0.5, 0.3, 0.7, 0.15, 0.85):
        spot = tuple(vertices[int(len(vertices) * fraction)])
        closure = db.add_closure(spot)
        try:
            status, answer = api.route(CASINO, PORT)
            if status == 200:
                nearest = min_distance_to(answer, spot)
                crosses = nearest <= 15
                check(f"closure at {int(fraction * 100)}% of the route: the new route stays clear of it", not crosses, f"{nearest:.0f} m away, {answer['distance_meters']:.0f} m long")
                if not crosses and answer["geometry"] != base["geometry"]:
                    detours += 1
            elif status == 404:
                print(f"INFO  closure at {int(fraction * 100)}% of the route: no alternative exists ({answer['error']})")
            else:
                check(f"closure at {int(fraction * 100)}% of the route is answered with a route or a 404", False, f"status={status} {str(answer)[:100]}")
        finally:
            db.remove(closure)
    check("at least one closure was avoided with a real detour", detours >= 1, f"detours={detours}")

    # A closure on the road at the destination: there is no way to get there without crossing it.
    end = tuple(vertices[-1])
    closure = db.add_closure(end)
    try:
        status, answer = api.route(CASINO, PORT)
        served_through_it = status == 200 and min_distance_to(answer, end) <= 15
        check("a closure at the destination is never routed through", not served_through_it, f"status={status}")
    finally:
        db.remove(closure)

    # The extract is Monaco: a point 100 km out at sea is within the distance limit but off the map.
    status, answer = api.route(CASINO, (7.43, 42.84))
    check("a point off the map is a 404", status == 404, f"status={status}")

    return finish()


def finish():
    print()
    if FAILURES:
        print(f"{len(FAILURES)} check(s) failed:")
        for label in FAILURES:
            print(f"  - {label}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
