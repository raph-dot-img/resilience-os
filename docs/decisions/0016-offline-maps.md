# ADR 0016: Offline maps

Status: accepted (2026-11-15).

## Context

Phase 5 brings maps to the text console (ORIGINAL-SPEC §13). The user
must be able to:

- see a map;
- find a place, a settlement or a street;
- read the coordinates;
- see points of interest nearby;
- see relief where the data has it;
- install regional map packs one by one (France, Western Europe, a world
  overview...), each showing its data date, coverage, attribution and
  licence, and never claiming full coverage when the data is partial.

Routing must be studied; implementing it is optional.

The constraints:

- **The display is the Linux console.** It has the 8 basic colours,
  ASCII, and `▀▄█═║╔╗╚╝─│┌┐└┘░▒▓` (CLAUDE.md §4). With `▀`, one cell is
  two square "pixels" stacked, each with its own colour. A typical screen
  is 80x25 to about 170x48 cells, so 80x50 to 170x96 pixels.
- **Old machines**: 2 GiB of RAM or less, slow disks, no tokio.
- **Map files are untrusted input**, parsed as the logged-in user. A pack
  from the catalog is checked against its hash, but files on a stick may
  be opened without a check (ADR 0013 already allows this for ZIM files).
- **No hosting of our own, and no network at run time.**
- **Reuse the resource-pack machinery** (ADR 0013): a catalog pinned by
  hash, the unprivileged store, installs from a download or a stick, and
  reading straight from a stick on the live system.

Evidence for this ADR was gathered on 2026-11-15. Experiments were run on
the development host with the tools of the pinned nixpkgs
(`f1e5c356`, tilemaker 3.1.0, tippecanoe 2.79.0, osmium-tool 1.19.1,
sqlite 3.53.3) on Geofabrik's `luxembourg-260101.osm.pbf` (45.8 MB) and on
Natural Earth 5.1.2. The scripts were throwaway and are not in the
repository. Figures marked *estimate* come from extrapolation, not
measurement.

## Decision

### 1. Pack format: MBTiles holding vector tiles, plus our own search tables

A map pack is **one SQLite file in the MBTiles 1.3 layout**. Its `tiles`
table holds **Mapbox Vector Tiles** (MVT 2.1, gzip), in a small schema of
our own, `resilience-1`. Our own tables, prefixed `rs_`, add the search
index, the points of interest and the coverage polygon.

```text
metadata            name/value (MBTiles): name, format=pbf, minzoom, maxzoom, bounds,
                    center, attribution, json (vector_layers), plus
                    resilience_profile=1, resilience_data_date (from the PBF header),
                    resilience_source (input file + its SHA-256), resilience_coverage
                    (text), resilience_license, resilience_builder (version)
tiles               zoom_level, tile_column, tile_row (TMS), tile_data (gzip MVT)
rs_coverage         rings of the region covered, i32 microdegrees, little-endian blob
rs_place            id, kind (settlement|street|poi|natural), class, name, fold
                    (name folded for comparison), near (nearest settlement), lon, lat,
                    rank, bbox
rs_place_fts        FTS5 over rs_place.fold (external content, unicode61
                    remove_diacritics 2, prefix 2 3)
rs_poi_rtree        rtree_i32 (microdegrees) over POIs and peaks
```

Tile layers of `resilience-1`, with only what a text map can show:

| Layer | Geometry | Attributes |
|---|---|---|
| `water` | polygons (lakes, riverbanks, reservoirs) and lines (rivers, streams, canals, coastline) | `class` |
| `land` | polygons: forest, wood, park, wetland, glacier, sand, farmland | `class` |
| `roads` | lines | `class` (motorway... path), `name`, `ref` |
| `rail` | lines | `class` |
| `boundaries` | lines | `admin_level` (2, 4, 6, 8) |
| `places` | points | `class` (city... hamlet), `name`, `rank` |
| `pois` | points | `category` (our own list, see §4), `name` |
| `peaks` | points | `class` (peak, volcano, saddle), `name`, `ele` |

Buildings, house numbers and land use inside towns are left out. They
cannot be read at console resolution, and buildings are most of the bytes.

Why this format:

- **Vector data is the only kind a block renderer can use well.** We
  choose the colours, decide which classes to draw at each scale, and
  place the labels ourselves. Raster tiles would mean decoding PNG, JPEG
  or WebP (more parsers for untrusted input), then reducing anti-aliased
  pastel styles to 8 colours, which cannot be read. They are also much
  larger, and the OpenStreetMap tile servers forbid bulk and offline
  downloads (operations.osmfoundation.org/policies/tiles, verified).
- **MBTiles is one file, which suits the store.** The store hashes one
  file and never parses it (ADR 0013), so a pack must be one file with one
  SHA-256. In SQLite, the tiles and the search index sit side by side,
  and FTS5 and R*Tree give us prefix search without diacritics and
  "nearby" queries, both already implemented and widely tested. Other
  MBTiles readers ignore the `rs_` tables, so a pack still opens in
  ordinary tools.
- **MVT is a small format to decode.** It is protobuf with eight message
  types. A decoder takes about 300 lines of Rust that we write and test
  ourselves, with every count and index bounded; the experiment's Python
  version was 80 lines. No protobuf library is needed.
- **The tiles are small enough.** The Luxembourg experiment pack
  (tippecanoe, the layers above, z0-z14) is **35.5 MB from a 45.8 MB
  PBF**. By zoom range: z0-z12 is 12.0 MB, z13 is 8.7 MB and z14 is
  13.3 MB. *Estimate* for France (PBF 4.93 GB on 2026-01-01), using the
  same ratio: about **3.8 GB of tiles** for z0-z14 and **1.3 GB** for
  z0-z12, plus about 0.5 GB of search index.
- **Decoding is cheap.** A view covers 1 to 6 tiles (see §3). In Python,
  decoding and drawing the 409 KB of MVT of a full-screen view took
  200 ms. Rust should be 10 to 30 times faster (*estimate*; slice 6
  measures it).

### 2. Where the data comes from

What can be downloaded today (all checked on 2026-11-15):

| Source | What | Size | Dated, stable URL? | Hash published | Licence | Use |
|---|---|---|---|---|---|---|
| **Geofabrik** download.geofabrik.de | OSM extracts (PBF) per continent, country and region | France 4.93 GB (2026-01-01), Île-de-France 328 MB, Luxembourg 45.8 MB, Monaco 673 KB, Europe 33.6 GB | **Yes**: `NAME-YY0101.osm.pbf` kept for every 1 January since 2014, plus recent ones; `-latest` is a redirect (to `france-261114` today) | MD5 only (we compute the SHA-256) | ODbL 1.0, "© OpenStreetMap contributors" | **Input to our pack builder** |
| **Natural Earth** 5.1.2 (GitHub tag `v5.1.2`, raw GeoJSON) | world land, borders, lakes, rivers, 7,342 populated places, 711 elevation points | 10m land 10.2 MB, places 4.9 MB, lakes 5.0 MB, rivers 7.3 MB, borders 2.3 MB | **Yes** (git tag) | No (we pin our own) | Public domain, "Crediting the authors is unnecessary" | **Built-in world overview** |
| **GeoNames** download.geonames.org | gazetteer, `cities15000` 3.2 MB, `cities1000` 11 MB, `FR.zip` 7.0 MB | | **No**: rebuilt every day, no snapshots (Wayback Machine copies exist at irregular dates) | No | CC BY 4.0 | Not used now (open question 4) |
| **Protomaps** basemap builds | planet PMTiles z0-z15, daily | about 120 GB | daily builds kept for one week, plus the latest of each patch version; "URLs may change" | BLAKE3 | ODbL (Produced Work) | Not used: planet only, short retention |
| **VersaTiles** download.versatiles.org | planet MVT, Shortbread schema, z0-z14 | 62.1 GB (2026-10-27) | **Yes**, `osm-vector.YYYYMMDD.versatiles`, about 12 kept since 2024 | **SHA-256 and MD5** | ODbL | Not used by default; an advanced user's import path (`versatiles convert --bbox`, VersaTiles 4.1.0 is in nixpkgs, MIT) |
| **OpenFreeMap** | planet in a Btrfs image (OpenMapTiles schema) | planet only | no regional files | | OSM data ODbL; OpenMapTiles schema credit | Not used |
| **OpenMapTiles / MapTiler** data | regional MBTiles | | account required | | own terms | Not used |
| **Mapsforge** download.mapsforge.org | `.map` vector files per region, and `.poi` (SQLite) | france.map 3.3 GB (2026-08-25), france.poi 1.1 GB | **No**: names without dates, replaced in place | No | ODbL | Not used: cannot be pinned (alternatives) |
| **BRouter** brouter.de/segments4 | routing data, 5x5° tiles | E0_N45 128 MB, W5_N45 64.6 MB | **No**, regenerated weekly | No | MIT program; ODbL data | Routing study only |
| **Copernicus DEM** (AWS open data) | GLO-30 and GLO-90 elevation, 1°x1° COG tiles | GLO-90 5.2 MB, GLO-30 42.3 MB per tile (N45 E006), unchanged since 2022 | **Yes** (static) | No | Copernicus DEM licence (free, with attribution) | Topography later (§5) |

**No third party publishes regional files in a format we can read, with
dates in the names.** Protomaps and VersaTiles publish only the planet
(60-120 GB). Mapsforge files are regional but replaced in place, so a
pinned hash breaks within weeks. Packs are therefore **made by us, with a
reproducible builder**, from Geofabrik's dated extracts:

```text
Geofabrik NAME-YY0101.osm.pbf  (fixed-output fetch, SHA-256 pinned)
  -> osmium tags-filter          (keep only the tags we use)
  -> osmium export -f geojsonseq (areas assembled; pinned export config)
  -> resilience-mapmaker classify   (Rust: tags -> layer/class/minzoom,
                                     search and POI rows)
  -> tippecanoe -P -Z0 -z14 --no-tile-size-limit --no-feature-limit
  -> resilience-mapmaker finish     (new SQLite file: tiles in (z,x,y) order,
                                     fixed page size, our tables and metadata,
                                     no timestamps, VACUUM)
  => map-osm-REGION_YYYY-MM.mbtiles
```

The whole chain is the Nix derivation `packages.x86_64-linux.mapPacks.<id>`.
Anyone with Nix can rebuild a pack and gets **the same bytes**. The
catalog pins the output's size and SHA-256, so a pack made by anyone on
any machine installs from a stick, exactly like a ZIM file.

**Reproducibility was tested, and it decided the tool:**

- **tilemaker 3.1.0 is not reproducible.** Two runs on Luxembourg gave
  different files. Even with `--threads 1`, the tile bytes differed, and
  545 of 1,986 tiles still differed after decoding and sorting their
  features.
- **osmium export is reproducible**: two runs gave the same 432 MB
  GeoJSON.
- **tippecanoe is reproducible**: two runs, and a third with
  `TIPPECANOE_MAX_THREADS=1`, gave identical tile bytes for all 1,903
  tiles. tippecanoe's own MBTiles file still differs (file name in its
  metadata, SQLite page layout).
- **The `finish` step fixes the rest**: copying the tiles in order into a
  fresh database with fixed metadata, then `VACUUM`, gave two
  **byte-identical files** (SHA-256 `e72f1dbc...` both times).

Not verified: the same bytes on a different machine or CPU. The same
nixpkgs gives the same binaries, and the thread count made no
difference, but tippecanoe uses floating point. Slice 7 adds a check that
builds the test pack twice in different ways and compares the two.

Cost of building, measured on Luxembourg (6 cores, about 1 minute in
all): osmium export 8.6 s and 195 MB of RAM; classification 13 s in
Python, which Rust will make shorter; tippecanoe 24 s and 254 MB. For
France the time is perhaps 1 to 2 hours and the memory a few GB
(*estimate*: osmium's node index grows with the extract). That needs a
capable machine, not the target machine.

**What ships by default:**

1. **A built-in world overview** in the system image, also on the live
   ISO. It is built at build time from Natural Earth 5.1.2 (GeoJSON
   files fetched from the `v5.1.2` tag of nvkelso/natural-earth-vector,
   each pinned by hash in Nix) with the same builder:
   - layers: land, borders, regions, lakes, rivers, 7,342 populated
     places, elevation points;
   - zooms 0 to 7;
   - measured with tippecanoe: 7.9 MB for z0-z6 and 21.6 MB for z0-z8,
     so **about 13 MB** for z0-z7 (*estimate*), plus a search index of
     under 1 MB;
   - the overview has no streets and no village-level places. Its
     coverage text says so.

   It is declared like the documentation pack (`[[builtin]]`, ADR 0013)
   and lives in the Nix store.
2. **Catalog entries** (`nix/packs/maps.nix`) for regional packs, with
   their pinned hashes. Their `urls` are empty until there is a place to
   download them from (open question 1), so they install from a stick
   made with `nix build .#mapPacks.<id>`. The first entries:
   - Luxembourg, small and useful for testing on real data;
   - the French régions from Geofabrik's `europe/france/` sub-extracts
     (Île-de-France: 328 MB of PBF, about 270 MB of pack, *estimate*);
   - France as a whole.

   Each hash is computed once by whoever adds the entry, on a capable
   machine. A larger area (Western Europe) is just another derivation
   from a bigger extract.
3. **Self-made and third-party files**: any MBTiles file on a stick can be
   opened, marked "not checked", as with unknown ZIM files. Our own
   builder (`resilience-mapmaker`, plus osmium-tool and tippecanoe from
   nixpkgs) works on any PBF, for example a city from BBBike. A file in
   another schema (OpenMapTiles from tilemaker or OpenFreeMap, Shortbread
   from VersaTiles) is drawn once a style mapping for that schema exists
   (later, optional). It has no search, because it has no `rs_` tables.

### 3. Rendering

The viewer draws on a canvas of **pixels, two per cell, stacked**. Each
cell is printed as `▀`, with the top pixel's colour as foreground and the
bottom pixel's as background. Two equal pixels print as a space with that
background. Text (labels, markers) replaces whole cells.

- **Projection**: Web Mercator, the projection of the tiles. The view
  state is the centre (lat, lon) and the scale in **metres per pixel at
  the centre**, so the zoom does not depend on the window size.
- **Zoom steps** (`+`/`-`): 5, 10, 20, 50, 100, 200, 500 m, 1, 2, 5, 10,
  20, 50, 100 and 200 km per pixel. At 170 columns, 200 km per pixel
  shows the whole world, and 5 m per pixel shows a few streets. A pixel
  is about one standard 256-pixel web-map pixel at tile zoom
  `z = floor(log2(156543.03 * cos(lat) / m))`, clamped to the pack's
  zooms. Below the pack's maxzoom (z14), the z14 tiles are enlarged
  (overzoom). So a view needs 1 to 6 tiles. Decoded tiles are kept in an
  LRU cache (64 tiles, bounded in bytes as well).
- **Panning**: the arrow keys (or `hjkl`) move by a quarter of the screen,
  Shift+arrow by one cell. Longitude wraps; latitude stops at ±85°.
- **What is drawn, by scale** (drawn while the scale in m per pixel is at
  most the figure given; the builder's tippecanoe minzoom per class
  matches these figures at 45° latitude):

  | Class | Shown up to | Colour | Style |
  |---|---|---|---|
  | sea (overview land polygons inverted), lakes | always | blue | fill |
  | coastline, rivers | 500 m | blue | line |
  | streams, canals | 50 m | blue | line |
  | forest, wood | 500 m | green | fill |
  | parks, wetland | 50 m | green | fill |
  | country borders | always | magenta | dashed |
  | regions / départements / communes | 5 km / 1 km / 100 m | magenta | dashed |
  | motorway, trunk | 2 km | red | line |
  | primary, secondary | 500 m / 200 m | yellow | line |
  | tertiary, unclassified, residential | 100 m / 20 m / 20 m | white | line |
  | service, track, paths | 10 m | white | dotted |
  | railways | 200 m | cyan | dashed |
  | cities / towns / villages / hamlets | 20 km / 2 km / 200 m / 50 m | white text, `*` marker | label |
  | peaks (with elevation) | 200 m (100 m) | white `^`, elevation as text | label |
  | points of interest | 20 m (names at 10 m) | cyan category letter | label |
  | crosshair at the centre | always | reverse video | `+` |

  The Python experiment shows why scale rules matter. Luxembourg City at
  about 50 m per pixel, with every residential street drawn, filled the
  screen with lines. The table hides residential streets until 20 m per
  pixel.
  Later features overwrite earlier ones (areas, then lines by rank, then
  labels). Black is the land background. The theme can swap colours
  (light terminals); `l` shows the legend.
- **Labels** are placed greedily by priority (city > town > peak >
  village > POI > street name), with a one-cell margin, and are never
  overlapped. Street names appear only at 5 and 10 m per pixel, along the
  longest nearly horizontal stretch. Names are untrusted:
  `printable()`, at most 40 characters, then folded to the characters
  the console can show (open question 3). Non-Latin names use
  `name:latin` or `name:en` when the builder found one.
- **Readout** (bottom line): the coordinates of the crosshair as
  `49.61160 N  6.13190 E`. `d` switches to degrees, minutes and seconds.
  Then the **scale bar** `└────┘ 500 m`, 8 to 20 cells, with lengths in a
  1-2-5 series, true at the centre's latitude. Then the active pack, and
  always the attribution of what is visible (`© OpenStreetMap
  contributors`, `Natural Earth`). ODbL requires the credit to be visible.
- **Partial coverage is always visible:**
  - every detailed pack carries its coverage polygon (`rs_coverage`,
    from Geofabrik's `.poly`, pinned by hash);
  - at 500 m per pixel or finer, the area outside every installed
    detailed pack is shaded `░`, and the status line says "No detailed
    map here: world overview only (Natural Earth, 1:10 million)";
  - at coarser scales, the edge of each detailed pack is drawn dotted.

  The information screen (`i`) lists, for each pack: the title, the
  coverage text, the data date, the source with its hash, the licence,
  the attribution, and whether the file was checked against the catalog.
  It always adds: "OpenStreetMap is made by volunteers. Roads, places and
  names can be missing, wrong or out of date. Something absent from the
  map may exist."
- **Several packs**: the overview is drawn first, then each detailed pack
  inside its coverage, newest data first where they overlap. At most 8
  packs are open at once.

### 4. Search and points of interest

- **Index**: `rs_place` plus FTS5 `rs_place_fts`, **built into each pack
  by `resilience-mapmaker`**, never on the target machine. Building it
  there would mean decoding every tile of a country.
  - Rows: settlements (`place=*`, ranked by class and population); one
    street row per name and nearest settlement (segments merged; centre
    and bounding box kept); named POIs; named peaks, lakes and rivers.
  - `near` is the nearest settlement within 10 km at build time. The
    interface says "near X", not "in X", because we do not compute
    administrative containment.
- **Size, measured on Luxembourg**: 35,731 rows (2,214 places, 12,942
  street rows, 20,575 POIs) take **4.6 MB** with FTS5 (prefix indexes 2
  and 3) and the R*Tree. *Estimate* for France: about 0.5 GB.
- **Query**: the user's words are split into tokens by our own code.
  Each token is quoted, the last one gets `*` for a prefix match, and at
  most 8 tokens of up to 32 characters are kept. **User text never
  reaches FTS5's query syntax.** Each open pack is queried (`LIMIT 50`)
  and the results are merged and ranked:
  1. exact match on the folded name (for a street, a POI or a natural
     feature only within 10 km of the view's centre: "pharmacie" must
     not put a "Pharmacie" 60 km away before "Pharmacie du Centre" 2 km
     away; amended 2026-11-20);
  2. kind (settlements by rank, then streets, then POIs);
  3. distance from the view's centre.

  Each pack applies the same order in its query (flat distance) before
  `LIMIT 50`, so a pack with hundreds of matches returns the ones that
  count.

  Measured on the Luxembourg index: 0.1 to 1 ms per query. A result shows
  its name, kind, `near` and distance and direction ("Esch-sur-Alzette,
  town, 15 km SW"). Enter centres the map on it at a scale that suits the
  kind. "Not found" lists the packs that were searched and says that
  places outside them cannot be found.
- **Nearby (`n`)**: the R*Tree finds POIs around the crosshair, in
  growing boxes (500 m, 2 km, 10 km) until there are 50. They are sorted
  by distance on a sphere, with the direction. Measured: 1,217 POIs
  within about 500 m of Luxembourg's centre in 1.5 ms. The categories are
  a fixed list chosen at build time, listed most urgent first:
  - health (hospital, clinic, doctors, pharmacy);
  - water (drinking water, water point, spring);
  - shelter (shelter, alpine hut, camp site);
  - safety (police, fire station, defibrillator);
  - food (supermarket, bakery, marketplace);
  - fuel;
  - toilets;
  - post;
  - town hall;
  - other named places.

  Tab filters by category.
- **Go to coordinates (`g`)**: decimal degrees or degrees-minutes-seconds,
  parsed by a pure function with bounds.
- House numbers are left out (Luxembourg alone has 283,504). Addresses
  can become an optional table later.

### 5. Topography

- **Now**: spot heights from the data. OSM peaks, volcanoes and saddles
  with `ele` are drawn as `^` with the elevation at fine scales, and are
  listed under "Nearby" (in Luxembourg, 254 of 516 peaks have `ele`). The
  overview adds Natural Earth's 711 elevation points (major summits,
  ranges). The readout never shows an elevation for the crosshair: we
  have no terrain model, and the interface does not pretend otherwise.
- **Deferred**:
  - contour lines and the elevation under the crosshair, from the
    Copernicus DEM (GLO-90 at 5 MB, or GLO-30 at 42 MB, per 1° square;
    static URLs), turned into contours by `gdal_contour` at build time
    (GDAL 3.12.4 is in nixpkgs, MIT, but its closure is 1.1 GiB: build
    machine only);
  - contours would be a separate, optional "relief" pack per region, or
    a `contours` layer, and probably a few hundred MB for France
    (*estimate*);
  - mapterhorn.com terrain (355 GB, planet) and VersaTiles hillshading
    are too large and are planet-only.

  The exact attribution text the Copernicus licence requires must be
  checked when this is implemented.

### 6. Routing study

| Candidate | In pinned nixpkgs | Run-time memory | Preprocessing | Data | Offline | Fit |
|---|---|---|---|---|---|---|
| **OSRM** 26.4.0 | yes, BSD-2, closure 375 MiB | planet car: about 123 GiB, roughly the size of its files; *estimate* France: about 9 GiB | planet car extract: 415 GiB of RAM; linear in input, so about 30 GiB for France (*estimate*) | about 300 GB for the planet | yes | **No**: far beyond 2 GiB |
| **Valhalla** 3.6.3 | yes, MIT, closure **1.2 GiB** | low (tiles are memory-mapped) (*assumption*) | moderate, a few GB for a country (*assumption*) | tiled graph, perhaps 1-2 GB for France (*assumption*) | yes, runs as an HTTP service or a CLI that takes JSON | **The best external candidate**, but the closure is half again an ISO, and there is a service to manage |
| **GraphHopper** 11.0 | yes, Apache-2.0, closure 1.0 GiB (JVM) | JVM heap, GBs for a country (*assumption*) | moderate | its own graph folder | yes | **No**: JVM on 2 GiB machines |
| **BRouter** | **not in nixpkgs**, MIT, Java | 128-512 MiB heap | none for users; brouter.de rebuilds the planet weekly | 5x5° `.rd5` files (E0_N45: 128 MB), not dated, so they cannot be pinned | yes | **No**: packaging plus a JVM, and data that cannot be pinned |
| **Our own A\*** over a road graph in the pack | n/a | bounded: a search limited to about 100 km, nodes read from SQLite pages | done by `resilience-mapmaker` (graph of intersections with costs per mode) | *estimate*: 10-20% of the pack | yes | **Possible**, but unmeasured, and doing it well is real work (turn restrictions, one-way streets, access per mode) |

**Recommendation: implement later; nothing in Phase 5.** Reasons:

- every external engine is either too heavy for the target machines
  (OSRM, GraphHopper), not packaged and not pinnable (BRouter), or a
  1.2 GiB service (Valhalla);
- a route built from OSM data of a given date, with no closures, traffic
  or one-way errors corrected, is a safety risk if it is presented as
  reliable;
- the map, search and nearby features are worth more to users first.

A follow-up ADR should prototype two options on Luxembourg and measure
memory, time and data size on a 2 GiB VM: our own bidirectional A* (foot,
bicycle, car) over an `rs_graph` table, and Valhalla. Adding a graph
changes the pack bytes, so it would be a new pack version.

### 7. Code layout

- **`resilience-core::maps`**: pure code, no new dependencies, unit tests
  for each part:
  - `geo.rs`: Web Mercator and its inverse, tile maths, sphere
    distance and direction, coordinate formatting and parsing (decimal
    and DMS), scale-bar lengths;
  - `mvt.rs`: the bounded MVT decoder; it takes bytes that are already
    decompressed;
  - `canvas.rs`: the pixel canvas, Cohen-Sutherland clipping, Bresenham
    lines, dashed and dotted lines, scanline fill, composing pixels into
    cells;
  - `style.rs`: the `resilience-1` table (class, scale threshold, colour,
    style) as data;
  - `label.rs`: greedy placement, and folding text to the console's
    characters;
  - `pack.rs`: checks on metadata and coverage (point-in-polygon);
  - `search.rs`: the FTS query builder and ranking.
- **New crate `resilience-maps`**, with two binaries:
  - `resilience-maps` (as the user, in the foreground, like
    `resilience-crypto`):
    ```text
    view [--pack FILE]... [--at LAT,LON] [--scale M] [--search WORDS]
    info FILE        metadata, coverage, checks (for the TUI and tests)
    check-config FILE (build time)
    ```
    It has its own ratatui loop and its own state machine (`MapApp`)
    that returns effects, and it is tested with TestBackend.
  - `resilience-mapmaker` (build machines only): `classify`, `finish`.
    It uses **serde_json** (MIT/Apache-2.0, 1.0.x) to read osmium's
    GeoJSON.

  A separate process keeps untrusted SQLite and MVT parsing **out of the
  TUI**: a crash or hang ends the viewer, never the session. The viewer
  also lowers its own memory limit (RLIMIT_AS, 1 GiB).
- **New dependencies, all in `resilience-maps` only**, so neither
  `resilience-core` nor the TUI links them:
  - **rusqlite** with `default-features = false` and `bundled` (MIT;
    bundled SQLite is public domain, with FTS5 and RTREE enabled in its
    build script, checked). Bundling gives the same SQLite and options on
    the host and in Nix. rusqlite 0.43 needs Rust 1.89, and the workspace
    says 1.85; nixpkgs has rustc 1.95.0. So either raise
    `rust-version` or use an older rusqlite (0.37 states none).
  - **miniz_oxide** (MIT/Zlib/Apache-2.0, pure Rust) for gzip, with an
    output limit.

  Both are vendored by `cargoLock.lockFile`. There is no other new crate:
  no protobuf, geo or tile libraries.
- **SQLite hardening for every map file**:
  - open with `SQLITE_OPEN_READONLY | SQLITE_OPEN_NOFOLLOW` and URI
    `immutable=1` (no journal, WAL or lock files, nothing written to a
    stick);
  - a file in WAL mode is refused;
  - `SQLITE_DBCONFIG_DEFENSIVE`, `trusted_schema=OFF`, `query_only=ON`,
    `cell_size_check=ON`, `mmap_size=0`, `cache_size` 8 MiB;
  - load_extension is never enabled; rusqlite's `load_extension` feature
    stays off, and `LIBSQLITE3_FLAGS=-DSQLITE_OMIT_LOAD_EXTENSION` is
    tried in slice 5;
  - `sqlite3_limit` caps: LENGTH 4 MiB (bounds a tile blob),
    SQL_LENGTH 10 KB, COLUMN 64, EXPR_DEPTH 100, ATTACHED 0,
    VARIABLE_NUMBER 16, TRIGGER_DEPTH 1;
  - a **progress handler** stops any statement after a step budget (tile
    read 200 ms, search 2 s), so a crafted view (for example a recursive
    CTE behind `tiles`) cannot hang the viewer;
  - only fixed statements on known names are prepared; the `rs_` tables
    are used only if their `CREATE` text in `sqlite_schema` is exactly
    the profile's (otherwise "search is not available in this file");
  - an unchecked file under 256 MiB gets `PRAGMA quick_check` before
    use; for a larger one it is offered.
- **MVT and gzip bounds**:
  - compressed tile 4 MiB, decompressed 8 MiB (stopped past that: zip
    bombs);
  - per tile at most 64 layers, 65,536 keys and values, 200,000
    features and 2 million geometry integers; strings 1 KiB;
  - extent between 256 and 65,536;
  - every index is checked and coordinates use checked i64 arithmetic;
  - geometry is clipped to the view before drawing, so drawing costs at
    most what the screen holds;
  - unit tests feed thousands of random and mutated tiles (a small PRNG,
    no fuzzing crate) and require an error, never a panic.
- **TUI** (`resilience-tui`, new `maps_ui.rs`): "Maps & navigation" shows
  - **Map** (opens the viewer at the last view, kept in
    `~/.local/state/resilience/maps.toml`: just a position, no history);
  - **Find a place** (the viewer with search open);
  - **Map packs** (the Resource packs screen, filtered to the `maps`
    category);
  - **Maps on a USB stick**.

  The viewer runs as `Effect::Run` with a fixed argv.
- **Pack plumbing** (`resilience-core::packs`, the store, `packs.nix`):
  - file names may end in `.mbtiles` as well as `.zim`;
  - new category `maps`;
  - optional catalog fields `data_date`, `coverage` and `attribution`,
    shown in the pack details;
  - `resilience-library open` sends `.mbtiles` files to the viewer;
  - the store needs no other change: it never parses content;
  - names follow `map-osm-REGION_YYYY-MM.mbtiles`, so `stem()` already
    recognises older versions;
  - the roadmap's **Geography** set is the regional map packs plus the
    Wikivoyage packs.
- **Modes**: no difference for viewing, searching or "nearby". The viewer
  starts no program, writes no files except its last position, and has no
  shell escape. Installing needs an administrator's password as in ADR
  0013. Opening an unchecked file is allowed in every mode, as for ZIM
  files, with the "not checked" label, and the parsing limits above.
  Making packs (`resilience-mapmaker`, osmium-tool, tippecanoe) is not in
  the image.

### 8. Tests

- **A test map without committed binaries.** A Nix derivation writes a
  small synthetic OSM XML file (`writeText`): a town "Testville", a
  village, "Rue de l'Essai" and a few streets, a café, a pharmacy, a
  drinking-water point, a peak with `ele`, a lake, a forest and a river.
  `osmium cat` turns it into PBF, and the normal pipeline makes
  `map-osm-testland_2026-01.mbtiles`. It is put into the VM test's catalog
  the way vm-packs adds its test ZIM, by appending to
  `config.resilience.packs.configFile`. Rust unit tests build their own
  tiny MBTiles at test time, with rusqlite and a small MVT encoder in the
  test code.
- **Reproducibility check** (`checks.map-pack-reproducible`): build the
  test pack twice (`TIPPECANOE_MAX_THREADS=1` and the default) and
  compare the SHA-256.
- **Unit tests** (core):
  - projection round trips at known points and the poles' clamp;
  - tile choice for each scale;
  - DMS and decimal formatting and parsing, including bad inputs;
  - scale-bar lengths;
  - sphere distance and direction against known pairs;
  - MVT: hand-built tiles, zigzag and varint edge cases, every bound,
    random and mutated input;
  - clipping, lines, fills and half-block cells, compared as text;
  - style visibility per scale;
  - labels never overlap, priority is kept;
  - folding (é→e, ß→ss, Ø→O...);
  - FTS query builder (quotes, `*`, `NEAR`, `-`, very long input) and
    ranking;
  - point-in-polygon coverage;
  - metadata validation;
  - packs: `.mbtiles` names, category, the new fields, `stem()`.
- **Unit tests** (resilience-maps):
  - the hardened opening refuses WAL files, symlinks and wrong schema
    text;
  - the progress handler stops a recursive view;
  - a gzip bomb is refused;
  - a truncated file gives an error, not a panic;
  - search and nearby on a test pack.
- **TestBackend screen tests** (viewer):
  - first screen on the overview: readout, scale bar and attribution;
  - zoom in and out and pan;
  - "No detailed map here" outside the coverage;
  - search: typing, the results list, Enter centres the map;
  - nearby list;
  - info screen with the data date, coverage and "not checked";
  - legend and help;
  - `RESILIENCE_DUMP` support;
  - the console glyph test (the same rule as `only_console_font_glyphs_are_drawn`),
    at 80x24, 60x20 and 160x50.
- **TUI tests**: Maps & navigation lists its entries once a map exists
  (the overview is built in, so always); launching gives the right fixed
  argv; Map packs shows only the `maps` category; the details show the
  data date and coverage.
- **`vm-maps`** (one VM, 2 GiB, no network), assertions through
  `/dev/vcsu1`:
  1. Maps & navigation → Map opens on the overview; the screen shows
     `Natural Earth` and a coordinate readout;
  2. the test pack is installed from a USB stick (QEMU `usb-storage`, as
     vm-backup) with an administrator's password; `/var/lib/resilience/packs`
     holds it, owned by `resilience-packs`;
  3. `/` and `essai` give "Rue de l'Essai, near Testville"; Enter centres
     the map, and the readout matches the coordinates from the OSM
     XML to 4 decimals;
  4. `n` lists the pharmacy and the drinking water with distances;
  5. moving outside the coverage shows "No detailed map here";
  6. hostile files on a second stick, each opened from "Maps on a USB
     stick", each giving an error message within a few seconds, with the
     TUI still alive afterwards:
     - random bytes;
     - an MBTiles file whose tile is a gzip bomb;
     - one whose `tiles` is a recursive view;
     - one in WAL mode;
  7. the viewer's peak RSS stays under 200 MB while panning (read from
     `/proc`);
  8. nothing was written next to the files on the stick.

### 9. Implementation slices

Each slice compiles, passes `cargo test`, `cargo clippy --all-targets`
and `nix flake check --no-build`, and updates docs/TESTING.md. In the
order given:

1. **Packs accept maps**:
   - what: `.mbtiles` names, category `maps`, optional
     `data_date`/`coverage`/`attribution`, details screen,
     `resilience-library open` refusing `.mbtiles` with "open it in
     Maps";
   - files: `crates/resilience-core/src/packs.rs`,
     `crates/resilience-library/src/main.rs`,
     `crates/resilience-library/src/bin/store.rs` (tests),
     `nix/modules/packs.nix`, `crates/resilience-tui/src/library.rs`,
     `crates/resilience-tui/src/tests.rs`.
2. **Geographic maths**:
   - files: `crates/resilience-core/src/maps/{mod,geo}.rs`,
     `crates/resilience-core/src/lib.rs`;
   - covers projection, tiles, distances, coordinates (formatting and
     parsing), scale bar.
3. **MVT decoder**:
   - files: `crates/resilience-core/src/maps/mvt.rs`;
   - bounded, with random-input tests and a test-only encoder.
4. **Canvas, style, labels**:
   - files: `crates/resilience-core/src/maps/{canvas,style,label}.rs`;
   - tests compare text output.
5. **Pack reader**:
   - files: new `crates/resilience-maps/` (`Cargo.toml`, `src/pack.rs`,
     `src/main.rs` with `info` and `check-config`), workspace
     `Cargo.toml`, `Cargo.lock`, possibly `rust-version`,
     `nix/packages/resilience/default.nix` (only if build inputs are
     needed);
   - covers rusqlite with SQLite hardening and the gzip limit;
   - tests use synthetic packs made in the test code.
6. **Viewer**:
   - files: `crates/resilience-maps/src/{app,ui,main}.rs`;
   - covers the `view` command: pan, zoom, readout, scale bar,
     attribution, coverage shading, several packs, tile cache;
   - measure decode and draw time and memory; TestBackend tests.
7. **Pack builder and test pack**:
   - files: `crates/resilience-maps/src/bin/mapmaker.rs` (classify,
     finish, search and POI tables, coverage),
     `nix/maps/{default.nix,osmium-export.json,filter.txt}`,
     `nix/maps/test-osm.nix` (synthetic XML), flake `packages.mapPacks`
     and `checks.map-pack-reproducible`.
8. **Built-in world overview**:
   - files: `nix/packages/resilience-world-map/default.nix` (Natural
     Earth `fetchurl`s pinned at `v5.1.2`), the `[[builtin]]` entry in
     `nix/modules/packs.nix`, `docs/COMPONENTS.md`;
   - check the image size grows by about 15 MB.
9. **TUI integration**:
   - files: `crates/resilience-tui/src/{app,maps_ui,help,tests}.rs`, the
     `[tools]` entry for the viewer in `nix/modules/core.nix`, a new
     page `handbook/NN-maps.md`;
   - covers the Maps & navigation entries and launch effects.
10. **Search**:
    - files: `crates/resilience-core/src/maps/search.rs`, the viewer's
      search dialog, mapmaker rows and `near`; tests.
11. **Nearby and go-to**:
    - files: the viewer, `geo.rs` parsing, and the POI categories in
      mapmaker; tests.
12. **Topography (spot heights)**: the peaks layer and Natural Earth
    elevation points; legend; tests.
13. **VM test and catalog**:
    - files: `tests/vm/maps.nix`, `flake.nix` checks,
      `nix/packs/maps.nix` (Luxembourg, the French régions and France,
      with hashes computed on a capable machine), `nix/packs/sets.nix`
      (Geography), `docs/TESTING.md`, `CLAUDE.md`, `ROADMAP.md`,
      status of this ADR.

Later and optional:

- style mappings for the OpenMapTiles and Shortbread schemas (unchecked
  third-party files);
- an optional house-number table;
- contours (§5);
- the routing prototype ADR (§6);
- making small regions on the target machine (open question 2).

### 10. Maps made on this computer (amendment, 2026-11-22, release 0.7.1)

Catalog packs are rebuilt once a year; people need fresher or other
regions. Maps & navigation → **Make or update a map** builds one on the
computer itself (ROADMAP Phase 5b, owner 2026-11-21):

- **Regions**: Geofabrik's index (`index-v1-nogeom.json`) reduced to id,
  name, parent, extract and boundary URLs by `scripts/map-regions.py`
  into `nix/maps/regions.json` (554 regions, 134 KB), shipped in the
  system; ids with `/` become `-` (`us-texas`). Every entry is checked
  when read: ids `[a-z0-9-]`, URLs under `https://download.geofabrik.de/`
  only, no `..`.
- **Getting the extract**: only after the person chose a region and
  pressed Enter. A HEAD request gives the dated file the "latest" name
  redirects to and its size; the builder then says what it needs and asks
  (`Start? [y/N]`). curl `--proto =https --proto-redir =https`, resumed
  (`--continue-at -`) into `~/.cache/resilience/maps/NAME.part`, checked
  against Geofabrik's `.md5` (integrity against damage; authenticity
  rests on HTTPS, as for the catalog's sources) and deleted on a
  mismatch. Or an `.osm.pbf` on a USB stick (or any file), read in place;
  its `.poly` beside it, else the rectangle of its header.
- **Building**: `resilience-mapmaker make`, in the foreground as the
  person, the same steps and options as the catalog packs (§2) except
  `osmium tags-filter`, whose ID sets take about 2.2 GB whatever the
  region; `osmium export` straight from the extract gives the same
  features and rows (Luxembourg, 2026-11-22: identical sorted output;
  peak 183 MB instead of 2.2 GB, 35 s on an i3-6100). No sea (its input
  is 909 MB and needs GDAL): coastal maps keep their coastline lines over
  the overview's sea. Absolute tool paths from the root-owned
  `/etc/resilience/mapmaker.toml`, a clean environment,
  `oom_score_adj` 1000 (the kernel stops the build first if memory runs
  out). Refused above 2 GiB of extract, or with less than 12 times the
  extract in free disk or 6 times it in memory (at least 200 MB and
  512 MB).
- **Result**: `~/Maps/map-osm-ID_YYYY-MM.mbtiles` (written as a hidden
  `.partial`, renamed when `finish` has checked it); an older map of the
  same region is removed afterwards; the downloaded extract is deleted.
  Listed as "Made here" in the maps list and opened with the installed
  packs. They are the person's own files: not checked against a catalog,
  read by the same hardened reader as a stick's.
- **Where**: standard edition (`resilience.mapmaker.enable`, mkDefault
  on): osmium-tool and tippecanoe join the system. Tests: unit tests
  (region list, needs, names, formats), the TUI flow test, `vm-mapmaker`
  (a fake download.geofabrik.de over HTTPS: resumed download, MD5
  refusal, a file without its `.poly`).

## Alternatives considered

- **PMTiles** (Protomaps; pmtiles 1.31.1 in nixpkgs, BSD-3):
  - for: one file, a pure-Rust reader of the header and directories would
    be simple, and no SQLite would be needed;
  - against: it holds only tiles, so search and POIs would need a second
    file or a format of our own, and the store handles one file per
    pack;
  - the only published PMTiles are planet builds (120 GB) kept for a
    week;
  - kept as a possible later import format, without search.
- **GeoPackage** (OGC, also SQLite): features as WKB with an R-tree, so
  no tiles and no generalisation. Drawing France at 2 km per pixel would
  read millions of features, and adding generalised tables per scale is
  re-inventing tiles. Producing it needs GDAL (1.1 GiB closure).
- **Raster MBTiles**: image decoders, illegible at 8 colours, much
  larger, and no legal bulk source (the OSMF tile policy forbids it).
- **Reading raw OSM extracts (PBF) at run time**: there is no
  generalisation and no index, and every view would assemble geometry.
  That is impossible on 2 GiB for a country.
- **A container format of our own, without SQLite**:
  - for: no C dependency, and a parser entirely ours to bound;
  - against: we would also have to write and maintain a full-text index,
    a spatial index and an interchange story, while SQLite is built for
    hostile database files when hardened as above (the "defense against
    the dark arts" settings);
  - kept as the fallback if a review rejects SQLite.
- **Mapsforge `.map`** (offline Android maps): regional, compact (France
  3.3 GB) and documented, but published under undated names that are
  replaced in place, so a hash cannot be pinned. The map writer (Java,
  Osmosis) is not in nixpkgs, and there is no Rust reader, so we would
  write a reader for a complex format.
- **tilemaker** for building: faster, with a ready OpenMapTiles
  configuration, but its output is **not reproducible** (measured above).
  Without reproducibility, a pack could not be checked from a stranger's
  stick.
- **GeoNames for the overview's search**: more places (cities15000 has
  about 30,000, *assumption* from its description), but there are no
  dated files, so a pinned fetch would break the build within a day.
  Natural Earth's 7,342 places are pinnable and public domain.
- **Drawing inside the TUI process**: a single screen stack and simpler
  tests, but untrusted SQLite and MVT parsing would share the session's
  process. A foreground viewer is the pattern of ADR 0015.
- **OpenStreetMap water polygons** (osmdata.openstreetmap.de) to fill the
  sea at street scales: there are no dated files. Instead we use Natural
  Earth's land for the sea fill at coarse scales, and OSM coastline lines
  at fine scales.

## Consequences

- Maps work offline in every mode, including the live ISO with only the
  built-in overview. Regional detail needs a pack, and making one needs a
  capable machine with Nix, until there is somewhere to download packs
  from (open question 1).
- **A pack is checked by its hash, which anyone can rebuild**: the same
  trust model as ADR 0013, with the reproducible build replacing the
  publisher's file. If a nixpkgs update changes tippecanoe or osmium, the
  hashes change: catalog entries are recomputed when nixpkgs moves, and
  older packs still install from a stick only while their entry is kept.
- **ODbL**: packs are derivative databases. Anyone who publishes them
  must offer them under ODbL with the attribution; the derivation is the
  "method of creation". The owner should confirm this reading (open
  question 5).
- Storage: France is about 4.3 GB (*estimate*). Packs per région keep
  each download small.
- Two vendored crates with C code (SQLite) enter the userland, limited
  to the maps crate.
- No routing, no GPS, no house numbers, no contours in Phase 5.
- Not tested: reproducibility across machines; building France (time,
  memory); decoding speed on old hardware; the extrapolated sizes.

## Decisions on the questions raised by the study

Decided by the coordinator on 2026-11-15:

- **Making packs on the target machine**: not in Phase 5. It stays in the
  "later and optional" list, and needs reproducibility across machines to
  be measured first.
- **Console glyphs for names**: labels are folded to ASCII in Phase 5, so
  the glyph rule of CLAUDE.md §4 and its test stay as they are. `ter-i16n`
  does have the CP437 accented letters; using them is a later improvement
  with its own change to the rule and the test.
- **GeoNames**: not used. The world place search uses Natural Earth,
  which can be pinned. Wayback Machine copies are too fragile a source for
  a build input.
- **rusqlite**: the workspace `rust-version` is raised to 1.89 (the host
  has 1.98 and the pinned nixpkgs 1.95) and the current rusqlite is used.
- **Routing**: later, after a measured prototype and its own ADR.
- **Catalog entries**: only packs that were actually built, with a hash
  computed from that build, are listed. A region nobody built is not
  listed.

Decided during implementation:

- **Characters the console lacks**: `©` and `°` are not in the console
  font's allowed set. Attribution is shown as `(c) OpenStreetMap
  contributors` and the degrees-minutes-seconds readout as
  `49d36'41.8" N  6d07'54.8" E`. The coordinate parser accepts `°`.
- **Map packs need `data_date`, `coverage` and `attribution`** in the
  catalog (a build error otherwise), and the `maps` category holds only
  `.mbtiles` files.
- **Tiles per view**: 1 to 6 holds up to 70° of latitude. Near ±85° at the
  coarsest scales the world repeats sideways and more tiles are needed;
  the hard bound is 64 tiles per view.
- **MVT policy**: a layer whose version is not 2 and a feature of unknown
  geometry type are skipped and counted; any other malformed content
  rejects the whole tile.

Decided while building the world overview (slice 8, 2026-11-16):

- **Its own converter**: `resilience-mapmaker natural-earth` writes the
  same two files as `classify` straight from the Natural Earth GeoJSON
  (`mapmaker/overview.rs`), so no made-up OSM tags pass through the OSM
  tag tables. tippecanoe and `finish` are the same as for any pack.
- **Minimum zooms**: the later of Natural Earth's `min_zoom` attribute
  and the zoom from which the style draws the class. Region borders that
  Natural Earth places after zoom 7 are left out (4,616 of 10,179 kept);
  every place is kept (at zoom 7 at the latest) and searchable.
- **Places**: a national capital or a place of 100,000 inhabitants is a
  `city`, any other a `town`.
- **Borders**: every line of `admin_0_boundary_lines_land` is drawn the
  same way, disputed or not.
- **Elevation points** are not in the overview yet: the style draws
  peaks only up to 200 m per pixel, finer than the overview's data. They
  come with slice 12, which needs a rule for them.
- **Start position**: `finish --center LON,LAT,ZOOM`; the overview opens
  on the whole world (0, 20 N, zoom 0) instead of its largest city.
- **Measured**: 11,735,040 bytes for zooms 0 to 7 with the search index
  (the estimate was 13 MB plus under 1 MB).
- **Licence text**: `Public domain` (Natural Earth's own terms; there is
  no SPDX identifier for it), attribution `Natural Earth`.

Decided while adding maps to the TUI (slice 9, 2026-11-16):

- **Two entries** under Maps & navigation: "Map" runs `resilience-maps
  view --pack F...` on the built-in maps, then the installed catalog
  packs; "Maps on this computer & USB sticks" is the library list for
  `.mbtiles` files (built in, installed, older versions, open sticks),
  where Enter opens the map on the chosen file over the overview and `c`
  checks a stick file. "Map packs" is not a third entry: the Resource
  packs screen already has the `maps` category.
- **Which files**: an older version or a file that is not in the catalog
  is opened only when chosen in the list. At most 8 files are passed,
  the viewer's limit; the built-in overview is always first.
- **No `maps_ui.rs`**: the list is `View::Library` with a pack kind, in
  `library.rs`.
- The entries exist only on a system with resource packs
  (`/etc/resilience/packs.toml`), like the Offline library.

Decided while adding spot heights (slice 12, 2026-11-19):

- **Two new classes in the `peaks` layer**: `summit` (Natural Earth's
  mountains, plateaus and spot elevations: `^`) and `depression` (`v`,
  so that the Dead Sea shore is not drawn as a peak); Natural Earth's two
  passes are `saddle`. A viewer that does not know a class skips it, so
  older viewers still read new packs.
- **The rule**: `summit` and `depression` are drawn, named and with their
  height (`ele`) up to 10,000 m per pixel; which ones appear at the
  coarsest scales is Natural Earth's own `min_zoom` (4 to 8; points beyond
  zoom 7 are kept at 7, like places). OSM peaks keep their rule (200 m per
  pixel, height from 100 m per pixel). The 711 points are search rows and
  in the R*Tree, so they are found with `/` and listed under `n`.
- Heights outside the accepted range (-500 to 9,000 m) are left out; the
  point stays.

Open (found 2026-11-19, to settle before the maps review):

- **Letters `unicode61` does not fold** (settled 2026-11-20): the FTS
  index was built on the raw name with `remove_diacritics 2`, which
  removes combining accents but does not turn `Ł` into `l`, `Ø` into `o`
  or `Đ` into `d`, so "lodz" did not find "Łódź". `rs_place_fts` now
  indexes `rs_place.fold` (folded by the builder with the same function
  as the query). Search was never released before this change, so the
  reader accepts only the new definition: packs built before it open
  with their maps but without search until rebuilt (the two catalog
  packs are rebuilt for 0.6.0).

Decided by the project owner on 2026-11-19 (maps resumed for 0.6.0):

- **Catalog regions**: Luxembourg and Bretagne (built, hashed and
  hosted as GitHub Release assets; slice 13).
- **Spot heights** (slice 12) are in 0.6.0.
- **Accented names**: yes. Labels and search results use the console
  font's CP437 accented letters instead of folding them to ASCII; the
  glyph rule of CLAUDE.md §4 and its test change with it (a slice of its
  own, after search).
- **Order**: search (slice 10) first, then nearby and go-to (11), spot
  heights (12), accents, catalog and VM test (13), and an independent
  review of the pack builder and viewer before release.

Decided while showing accented names (2026-11-19):

- **The letters**: read from the Unicode table of `ter-i16n.psf.gz` in
  the pinned nixpkgs (`terminus_font` 4.49.1; a PSF1 font, 256 glyphs),
  with a small Python parser and checked with kbd's `psfgettable`. Its
  accented Latin letters are exactly the CP437 ones:
  `ÄÅÆÇÉÑÖÜßàáâäåæçèéêëìíîïñòóôöùúûüÿ` (34; `canvas::CONSOLE_LETTERS`).
  `ƒ` and `ⁿ` are in the font too but are not used as letters.
- **`label::fold` keeps them** and folds every other accented letter as
  before: `Orléans`, `Zürich`, `Straße`, `Æblegård` are shown as written;
  `Saint-Étienne` keeps its `É`, but `Île` becomes `Ile`, `Łódź` becomes
  `Lódz`, `São` becomes `Sao`, `Œ` stays `OE` (À, Â, È, Ê, Ë, Î, Ô, Û, Ø,
  ã, õ and the Latin Extended letters are not in the font). Lengths are
  counted in characters (each is one cell), so the 40-character cap is
  unchanged.
- **Search is unchanged**: `search::search_fold` (and the pack builder's
  `rs_place.fold`) uses `label::fold_ascii_to`, the ASCII-only fold, so
  existing packs still match. Results are shown with `fold`.
- **The glyph rule**: `canvas::is_console_char` (every map cell and the
  viewer's screens) allows these 34 letters. The main TUI draws no map
  names, so its test `only_console_font_glyphs_are_drawn` is unchanged.

Decided for the sea inside regional packs (2026-11-21, owner's go-ahead
to settle the open finding before rebuilding the catalog packs):

- **The finding**: at 500 m per pixel and finer, a detailed pack's layer
  starts black (land) so that the overview's coarse coastline never shows
  at street scale; a Geofabrik region's coverage polygon reaches into the
  sea, and an extract has coastline lines but no sea areas, so the sea
  inside Bretagne was black with blue coastline lines.
- **Chosen**: OpenStreetMap's water polygons (`water-polygons-split-4326`,
  osmdata.openstreetmap.de, made from the whole planet's coastline by
  osmcoastline; ODbL like the packs), cut at build time to the rectangle
  around the region with GDAL's `ogr2ogr -spat ... -clipsrc spat_extent`
  and written by `resilience-mapmaker sea` as layer `water`, class `sea`
  (style: filled blue at every scale, under every other area; the legend's
  existing "Sea" line stands for it). The viewer already clips a pack to
  its coverage polygon, so the rectangle is enough.
- **Pinned on the project's mirror**: osmdata replaces the file daily and
  keeps no old version, so one download (2026-11-20, 909 MB, SHA-256
  `8c02ca8554882c7e99efeefa8f618e12395bced404a97c47a65a32d25fe39d05`) is
  published unchanged under `packs/maps/sources/` and fetched from there
  by hash. The pack's `resilience_source` names it next to the PBF. The
  coastline in the water polygons can differ slightly from the extract's
  (two days apart for Bretagne); the extract's coastline lines are still
  drawn.
- **Rejected**: letting the overview's sea show through (its coastline is
  kilometres off at street scale); assembling the sea from the extract's
  coastline (it is cut open at the extract's edge; closing it along the
  boundary is osmcoastline's hard part, and osmcoastline is not in
  nixpkgs and wants the whole planet).
- **Compatible**: the class is new, the profile stays `resilience-1`, and
  a viewer that does not know `sea` skips those features (the sea stays
  black there, as before). Inland regions (Luxembourg) have no sea input.

Decided by the project owner on 2026-11-15:

- **Hosting**: built packs are published as GitHub Release assets of the
  project (at most 2 GiB each, so per-région packs; all of France as one
  file does not fit and stays stick-only or is split by région). Catalog
  `urls` point at those assets once a pack is built and uploaded.
- **ODbL**: the Nix derivation that builds a pack is the offer of the
  method of creation. Packs are distributed under ODbL 1.0 with the
  OpenStreetMap attribution.

Tested by: the experiments above (throwaway, 2026-11-15). The tests
listed in §8 come with the slices.
