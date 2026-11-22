#!/usr/bin/env python3
"""Geofabrik's index-v1-nogeom.json -> nix/maps/regions.json (ROADMAP Phase 5b).

Usage: python3 -I scripts/map-regions.py INDEX.json > nix/maps/regions.json
Fetch the index first: curl -o INDEX.json https://download.geofabrik.de/index-v1-nogeom.json
Keeps the id, name, parent, extract and boundary URLs of every region, sorted by id.
"""
import json, sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
out = []
for f in d["features"]:
    p = f["properties"]
    u = p.get("urls", {})
    if "pbf" not in u or "poly" not in u:
        continue
    # Local ids are file-name parts: Geofabrik's "us/texas" becomes "us-texas".
    r = {"id": p["id"].replace("/", "-"), "name": p["name"]}
    if p.get("parent"):
        r["parent"] = p["parent"].replace("/", "-")
    r["pbf"], r["poly"] = u["pbf"], u["poly"]
    out.append(r)
out.sort(key=lambda r: r["id"])
sys.stdout.write("[\n" + ",\n".join(json.dumps(r, ensure_ascii=False) for r in out) + "\n]\n")
