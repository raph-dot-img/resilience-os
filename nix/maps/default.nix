# Map packs of profile resilience-1, built reproducibly (ADR 0016 §2):
#
#   PBF (fixed-output fetch, SHA-256 pinned)
#     -> osmium tags-filter (filter.txt)
#     -> osmium export -f geojsonseq (osmium-export.json)
#     -> resilience-mapmaker classify
#   + the sea, for coastal regions: OpenStreetMap water polygons (pinned
#     copy) -> ogr2ogr cut to the region's rectangle
#     -> resilience-mapmaker sea
#     -> tippecanoe
#     -> resilience-mapmaker finish
#     => $out/map-osm-ID_YYYY-MM.mbtiles (+ .sha256)
#
# For build machines only: none of these tools is in the system image. The
# derivation is the ODbL "method of creation" of the packs it builds.
# Anyone with this flake gets the same bytes: `nix build .#mapPacks.ID`.
# The build log gives the time and peak memory of each step and the sizes
# and largest tiles of the pack (see resilience-mapmaker's report).
{
  lib,
  runCommand,
  fetchurl,
  writeText,
  osmium-tool,
  tippecanoe,
  gdalMinimal,
  time,
  resilience,
}:

let
  # The pack builder alone, from the same sources as the system package
  # (which does not install it). Its tests run in checks.resilience.
  mapmaker = resilience.overrideAttrs (old: {
    pname = "resilience-mapmaker";
    cargoBuildFlags = [
      "-p"
      "resilience-maps"
      "--bin"
      "resilience-mapmaker"
    ];
    doCheck = false;
    postInstall = "";
    meta = old.meta // {
      description = "Map pack builder of Resilience OS (build machines only)";
      mainProgram = "resilience-mapmaker";
    };
  });

  tools = "osmium-tool ${osmium-tool.version}, tippecanoe ${tippecanoe.version}";
  toolsWithSea = "${tools}, GDAL ${gdalMinimal.version}";

  # OpenStreetMap's water polygons (the sea made from the whole planet's
  # coastline by osmcoastline), as osmdata.openstreetmap.de published
  # them on 2026-11-20. osmdata replaces the file every day and keeps no
  # old ones, so the build pins a copy on the project's mirror, unchanged
  # (same SHA-256 as the download). ODbL 1.0, (c) OpenStreetMap
  # contributors, like the packs.
  waterPolygons = fetchurl {
    url = "https://mirror.canevet.eu/files/resilience/packs/maps/sources/water-polygons-split-4326_2026-11-20.zip";
    hash = "sha256-jALKhVSILH6Z7+76j2GOEjlbztQEqXxHploy0l/jnQU=";
  };
  # The shapefile inside the zip, read in place by GDAL.
  waterPolygonsShp = "/vsizip/${waterPolygons}/water-polygons-split-4326/water_polygons.shp";

  # One pack. `dataDate` must match the PBF header's replication
  # timestamp (the build fails otherwise). `singleThread` runs osmium and
  # tippecanoe on one thread: the same bytes must come out (the
  # reproducibility check builds both).
  mkPack =
    {
      id,
      name,
      description,
      coverage,
      pbf,
      sourceName,
      poly,
      dataDate,
      # An OGR data source of sea polygons (null: inland, no sea).
      sea ? null,
      seaSource ? null,
      singleThread ? false,
    }:
    let
      file = "map-osm-${id}_${lib.substring 0 7 dataDate}.mbtiles";
    in
    runCommand file
      {
        nativeBuildInputs = [
          osmium-tool
          tippecanoe
          mapmaker
          time
        ]
        ++ lib.optional (sea != null) gdalMinimal;
        passthru = { inherit file mapmaker; };
        meta = {
          description = "Map pack ${name} (resilience-1, OpenStreetMap ${dataDate})";
          license = lib.licenses.odbl;
        };
      }
      ''
        set -euo pipefail
        ${lib.optionalString singleThread ''
          export TIPPECANOE_MAX_THREADS=1 OSMIUM_POOL_THREADS=1
        ''}
        step() { command time -f "time: $1: %e s, peak %M KiB" "''${@:2}"; }

        ts=$(osmium fileinfo -g header.option.osmosis_replication_timestamp ${pbf})
        if [ "''${ts%%T*}" != "${dataDate}" ]; then
          echo "the PBF header says '$ts', not ${dataDate}" >&2
          exit 1
        fi
        sha=$(sha256sum ${pbf} | cut -d ' ' -f 1)

        step tags-filter osmium tags-filter --no-progress -t \
          -e ${./filter.txt} -o filtered.osm.pbf ${pbf}
        step export osmium export --no-progress -c ${./osmium-export.json} \
          -f geojsonseq -o - filtered.osm.pbf \
          | step classify resilience-mapmaker classify --poly ${poly} \
              --rows rows.jsonl --output features.geojsonseq -
        rm filtered.osm.pbf
        ${lib.optionalString (sea != null) ''
          # The sea polygons meeting the region's rectangle, cut to it.
          read -r w s e n < <(resilience-mapmaker bounds --poly ${poly})
          step sea-cut ogr2ogr -f GeoJSONSeq -spat "$w" "$s" "$e" "$n" \
            -clipsrc spat_extent -lco COORDINATE_PRECISION=7 \
            sea.geojsonseq ${sea}
          step sea resilience-mapmaker sea --poly ${poly} sea.geojsonseq >> features.geojsonseq
          rm sea.geojsonseq
        ''}
        step tippecanoe tippecanoe --quiet --no-progress-indicator \
          --output=tiles.mbtiles --read-parallel \
          --minimum-zoom=0 --maximum-zoom=14 \
          --no-tile-size-limit --no-feature-limit --drop-rate=1 \
          --no-tile-stats --temporary-directory="$TMPDIR" \
          features.geojsonseq
        ls -l features.geojsonseq rows.jsonl tiles.mbtiles >&2
        rm features.geojsonseq
        step finish resilience-mapmaker finish \
          --tiles tiles.mbtiles --rows rows.jsonl --poly ${poly} \
          --output pack.mbtiles \
          --name ${lib.escapeShellArg name} \
          --description ${lib.escapeShellArg description} \
          --data-date ${dataDate} \
          --source "${sourceName} sha256:$sha${lib.optionalString (sea != null) "; sea: ${seaSource}"}" \
          --coverage ${lib.escapeShellArg coverage} \
          --tools ${lib.escapeShellArg (if sea != null then toolsWithSea else tools)}

        mkdir "$out"
        mv pack.mbtiles "$out/${file}"
        (cd "$out" && sha256sum ${file} > ${file}.sha256)
        cat "$out/${file}.sha256" >&2
      '';

  test = import ./test-osm.nix { inherit writeText; };

  testlandPbf =
    runCommand "testland.osm.pbf"
      {
        nativeBuildInputs = [ osmium-tool ];
      }
      ''
        osmium cat --no-progress \
          --output-header=osmosis_replication_timestamp=2026-01-01T00:00:00Z \
          -o "$out" -f pbf ${test.osm}
      '';

  testland = {
    id = "testland";
    name = "Testland";
    description = "Synthetic test region of Resilience OS (not a real place)";
    coverage = "Testland (synthetic): the area west of 6.1 E only";
    pbf = testlandPbf;
    sourceName = "testland.osm.pbf";
    poly = test.poly;
    dataDate = "2026-01-01";
    sea = test.sea;
    seaSource = "testland-sea.geojson";
  };

  # Geofabrik's extract of 1 January 2026 and its boundary. The .poly has
  # no dated name; if Geofabrik changes it, this fetch fails and the hash
  # (and the pack) must be updated together. Geofabrik moved every region
  # into a folder of its own in November 2026 and drops old daily
  # extracts, so the second URL is an unchanged copy on the project's
  # mirror (the hash decides either way).
  luxembourg = {
    id = "luxembourg";
    name = "Luxembourg";
    description = "OpenStreetMap data for Luxembourg (Geofabrik extract of 2026-01-01)";
    coverage = "Luxembourg, whole country (Geofabrik extract boundary)";
    pbf = fetchurl {
      urls = [
        "https://download.geofabrik.de/europe/luxembourg/luxembourg-260101.osm.pbf"
        "https://mirror.canevet.eu/files/resilience/packs/maps/sources/luxembourg-260101.osm.pbf"
      ];
      hash = "sha256-2PPh2rtNnVSJUAw1Qy6zkwicMiIs//jPUHBoUvBpaoA=";
    };
    sourceName = "luxembourg-260101.osm.pbf";
    poly = fetchurl {
      urls = [
        "https://download.geofabrik.de/europe/luxembourg/luxembourg.poly"
        "https://mirror.canevet.eu/files/resilience/packs/maps/sources/luxembourg.poly"
      ];
      hash = "sha256-1fcG5KPSXZ9Rc2ZZuWsBiIPn/Pj+d1LoTWxW2Kv7Uho=";
    };
    dataDate = "2026-01-01";
  };

  # Bretagne (owner's choice for 0.6.0, ADR 0016): Geofabrik's extract of
  # 18 November 2026 and the region's boundary (same caveat for the .poly).
  bretagne = {
    id = "bretagne";
    name = "Bretagne";
    description = "OpenStreetMap data for Bretagne, France (Geofabrik extract of 2026-11-18)";
    coverage = "Bretagne (France), the whole region: Côtes-d'Armor, Finistère, Ille-et-Vilaine, Morbihan (Geofabrik extract boundary)";
    pbf = fetchurl {
      urls = [
        "https://download.geofabrik.de/europe/france/bretagne/bretagne-261118.osm.pbf"
        "https://mirror.canevet.eu/files/resilience/packs/maps/sources/bretagne-261118.osm.pbf"
      ];
      # Re-pinned 2026-11-21: after Geofabrik's move the file of this
      # name has other bytes (same replication timestamp, 2026-11-18
      # 21:21:02Z, sequence 4970); the earlier copy is no longer published.
      hash = "sha256-hsPWzXI9AXtX3dlhSJ2Ywd/j/M1wGgXL2VmZOBNUu1w=";
    };
    sourceName = "bretagne-261118.osm.pbf";
    poly = fetchurl {
      urls = [
        "https://download.geofabrik.de/europe/france/bretagne/bretagne.poly"
        "https://mirror.canevet.eu/files/resilience/packs/maps/sources/bretagne.poly"
      ];
      hash = "sha256-yVBSJ1ZNQbWrAN6BqTzMuFlxeiLQPDvPV4Iqv+r2PU0=";
    };
    dataDate = "2026-11-18";
    sea = waterPolygonsShp;
    seaSource = "OpenStreetMap water polygons of 2026-11-20 sha256:8c02ca8554882c7e99efeefa8f618e12395bced404a97c47a65a32d25fe39d05";
  };
  # The other catalog regions (2026-11-22): one table, made from the
  # fetched extracts by scripts/map-pack-sources.py (nix/maps/regional.nix).
  regional = import ./regional.nix {
    inherit fetchurl;
    sea = waterPolygonsShp;
    seaSource = bretagne.seaSource;
  };
in
{
  inherit mkPack mapmaker;

  packs = {
    testland = mkPack testland;
    luxembourg = mkPack luxembourg;
    bretagne = mkPack bretagne;
  }
  // lib.mapAttrs (_: mkPack) regional;

  # The same test pack built on one thread (TIPPECANOE_MAX_THREADS=1,
  # OSMIUM_POOL_THREADS=1): checks.map-pack-reproducible compares the two.
  testlandSingleThread = mkPack (testland // { singleThread = true; });
}
