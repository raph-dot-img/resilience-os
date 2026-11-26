//! The map pack builder `resilience-mapmaker` (ADR 0016 §2), for build
//! machines, and `make` (a map made on this computer, [`make`]) in the
//! standard edition.
//!
//! ```text
//! osmium tags-filter | osmium export -f geojsonseq
//!   -> classify   OSM tags -> layer, class, per-feature minimum zoom
//!                 (GeoJSON sequence for tippecanoe) + search/POI rows
//!   -> tippecanoe
//!   -> finish     tiles in (z, x, y) order, metadata, rs_ tables, VACUUM
//! ```
//!
//! - [`tags`]: the readable table from OSM tags to the eight layers of
//!   `resilience-1`, names, `ele` and `population` parsing;
//! - [`geom`]: the few geometry calculations the builder needs;
//! - [`classify`]: the streaming `classify` step;
//! - [`rows`]: the side file of search and POI rows between the two steps;
//! - [`fold`]: names folded for comparison (`rs_place.fold`);
//! - [`poly`]: Geofabrik `.poly` coverage files;
//! - [`finish`]: the `finish` step, which writes the pack;
//! - [`overview`]: Natural Earth to the same two streams as `classify`,
//!   for the built-in world overview;
//! - [`sea`]: OpenStreetMap's water polygons, cut to the region, to `sea`
//!   features.
//!
//! Input is untrusted and messy (OSM tags are free text): strings are
//! bounded, what does not parse is skipped and counted, nothing panics.
//! Output is deterministic: the same input gives the same bytes.

pub mod classify;
pub mod finish;
pub mod fold;
pub mod geom;
pub mod make;
pub mod overview;
pub mod poly;
pub mod rows;
pub mod sea;
pub mod tags;
