//! `resilience-mapmaker` — builds `resilience-1` map packs (ADR 0016 §2).
//! The Nix derivations of `nix/maps` run it between osmium and tippecanoe;
//! the standard edition installs it for `make` (a map made on this computer).
//!
//! ```text
//! resilience-mapmaker classify --poly COVERAGE.poly --rows ROWS.jsonl
//!                              [--output FEATURES.geojsonseq] [INPUT|-]
//!     osmium's GeoJSON sequence (a file, or standard input) to the
//!     features for tippecanoe (standard output by default) and the rows
//!     for finish.
//! resilience-mapmaker finish --tiles TIPPECANOE.mbtiles --rows ROWS.jsonl
//!                            --poly COVERAGE.poly --output PACK.mbtiles
//!                            --name NAME --data-date YYYY-MM-DD
//!                            --source TEXT --coverage TEXT
//!                            [--description TEXT] [--license SPDX]
//!                            [--attribution TEXT] [--tools TEXT]
//!                            [--center LON,LAT,ZOOM]   (whole numbers)
//!     writes the pack (which must not exist) and checks it with the
//!     viewer's reader.
//! resilience-mapmaker natural-earth --rows ROWS.jsonl --output FEATURES.geojsonseq
//!                                   [--land F] [--countries F] [--regions F]
//!                                   [--lakes F] [--rivers F] [--places F] [--elevations F]
//!     Natural Earth GeoJSON files to the same two files as classify, for
//!     the built-in world overview.
//! resilience-mapmaker bounds --poly COVERAGE.poly
//!     prints "WEST SOUTH EAST NORTH" of the coverage, in degrees (for
//!     ogr2ogr -spat when cutting the sea to the region).
//! resilience-mapmaker sea --poly COVERAGE.poly [--output FEATURES.geojsonseq] [INPUT|-]
//!     OpenStreetMap water polygons cut by ogr2ogr (GeoJSON sequence) to
//!     features of class sea for tippecanoe (standard output by default).
//! resilience-mapmaker make [--config FILE] (--region ID | --pbf FILE [--poly FILE] [--region ID]) [--yes]
//!     downloads (on request) and builds a map into ~/Maps, in the foreground.
//! ```
//!
//! A summary goes to standard error. Exit status: 0 success, 1 failure,
//! 64 usage error.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use resilience_maps::mapmaker::{classify, finish, make, overview, poly, sea};

const USAGE: &str = "\
Usage:
  resilience-mapmaker classify --poly FILE --rows FILE [--output FILE] [INPUT|-]
  resilience-mapmaker finish --tiles FILE --rows FILE --poly FILE --output FILE
      --name NAME --data-date YYYY-MM-DD --source TEXT --coverage TEXT
      [--description TEXT] [--license SPDX] [--attribution TEXT] [--tools TEXT]
      [--center LON,LAT,ZOOM]
  resilience-mapmaker natural-earth --rows FILE --output FILE [--land FILE]
      [--countries FILE] [--regions FILE] [--lakes FILE] [--rivers FILE] [--places FILE]
      [--elevations FILE]
  resilience-mapmaker bounds --poly FILE
  resilience-mapmaker sea --poly FILE [--output FILE] [INPUT|-]
  resilience-mapmaker make [--config FILE] (--region ID | --pbf FILE [--poly FILE] [--region ID]) [--yes]";

const EXIT_USAGE: u8 = 64;

/// `--key value` pairs and positional arguments. Each key at most once.
fn parse(
    args: &[String],
    keys: &[&str],
) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let mut opts = BTreeMap::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(k) = a.strip_prefix("--") {
            if !keys.contains(&k) {
                return Err(format!("unknown option --{k}"));
            }
            let v = it.next().ok_or(format!("--{k} needs a value"))?;
            if opts.insert(k.to_string(), v.clone()).is_some() {
                return Err(format!("--{k} given twice"));
            }
        } else {
            rest.push(a.clone());
        }
    }
    Ok((opts, rest))
}

fn need<'a>(o: &'a BTreeMap<String, String>, k: &str) -> Result<&'a str, String> {
    o.get(k)
        .map(String::as_str)
        .ok_or(format!("--{k} is required"))
}

fn run_classify(args: &[String]) -> Result<(), String> {
    let (o, rest) = parse(args, &["poly", "rows", "output"])?;
    let text = std::fs::read_to_string(need(&o, "poly")?).map_err(|e| format!("--poly: {e}"))?;
    let rings = poly::parse(&text).map_err(|e| format!("--poly: {e}"))?;
    let opts = classify::Options {
        max_abs_lat: poly::max_abs_lat(&rings),
    };
    let rows =
        BufWriter::new(File::create_new(need(&o, "rows")?).map_err(|e| format!("--rows: {e}"))?);
    let features: Box<dyn Write> = match o.get("output") {
        Some(p) => Box::new(BufWriter::new(
            File::create_new(p).map_err(|e| format!("--output: {e}"))?,
        )),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };
    let stats = match rest.as_slice() {
        [] => classify::run(io::stdin().lock(), features, rows, &opts),
        [p] if p == "-" => classify::run(io::stdin().lock(), features, rows, &opts),
        [p] => {
            let f = File::open(p).map_err(|e| format!("{p}: {e}"))?;
            classify::run(BufReader::with_capacity(1 << 20, f), features, rows, &opts)
        }
        _ => return Err("one input at most".into()),
    }
    .map_err(|e| format!("classify: {e}"))?;
    eprint!(
        "classify: minimum zooms for latitudes up to {:.2}\n{}",
        opts.max_abs_lat,
        stats.report()
    );
    Ok(())
}

fn read_poly(o: &BTreeMap<String, String>) -> Result<Vec<poly::PolyRing>, String> {
    let text = std::fs::read_to_string(need(o, "poly")?).map_err(|e| format!("--poly: {e}"))?;
    poly::parse(&text).map_err(|e| format!("--poly: {e}"))
}

fn run_bounds(args: &[String]) -> Result<(), String> {
    let (o, rest) = parse(args, &["poly"])?;
    if !rest.is_empty() {
        return Err("bounds takes no positional arguments".into());
    }
    let (w, s, e, n) = poly::bounds(&read_poly(&o)?).ok_or("--poly: no points")?;
    println!("{w:.6} {s:.6} {e:.6} {n:.6}");
    Ok(())
}

fn run_sea(args: &[String]) -> Result<(), String> {
    let (o, rest) = parse(args, &["poly", "output"])?;
    let max_abs_lat = poly::max_abs_lat(&read_poly(&o)?);
    let out: Box<dyn Write> = match o.get("output") {
        Some(p) => Box::new(BufWriter::new(
            File::create_new(p).map_err(|e| format!("--output: {e}"))?,
        )),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };
    let mut stats = sea::Stats::default();
    match rest.as_slice() {
        [] => sea::convert(io::stdin().lock(), out, max_abs_lat, &mut stats),
        [p] if p == "-" => sea::convert(io::stdin().lock(), out, max_abs_lat, &mut stats),
        [p] => {
            let f = File::open(p).map_err(|e| format!("{p}: {e}"))?;
            sea::convert(
                BufReader::with_capacity(1 << 20, f),
                out,
                max_abs_lat,
                &mut stats,
            )
        }
        _ => return Err("one input at most".into()),
    }
    .map_err(|e| format!("sea: {e}"))?;
    eprintln!("{}", stats.report());
    if stats.written == 0 {
        return Err("sea: no polygon written (is the region inland? leave the sea out)".into());
    }
    Ok(())
}

fn run_finish(args: &[String]) -> Result<(), String> {
    let (o, rest) = parse(
        args,
        &[
            "tiles",
            "rows",
            "poly",
            "output",
            "name",
            "data-date",
            "source",
            "coverage",
            "description",
            "license",
            "attribution",
            "tools",
            "center",
        ],
    )?;
    if !rest.is_empty() {
        return Err("finish takes no positional arguments".into());
    }
    let mut builder = format!("resilience-mapmaker {}", resilience_core::VERSION);
    if let Some(t) = o.get("tools") {
        builder.push_str(&format!(" ({t})"));
    }
    let center = match o.get("center") {
        None => None,
        Some(c) => {
            let parts: Vec<&str> = c.split(',').collect();
            match parts.as_slice() {
                [lon, lat, z] => match (lon.parse(), lat.parse(), z.parse()) {
                    (Ok(lon), Ok(lat), Ok(z)) => Some((lon, lat, z)),
                    _ => return Err("--center: whole numbers LON,LAT,ZOOM".into()),
                },
                _ => return Err("--center: whole numbers LON,LAT,ZOOM".into()),
            }
        }
    };
    let d = finish::Describe {
        name: need(&o, "name")?.into(),
        description: o.get("description").cloned(),
        data_date: need(&o, "data-date")?.into(),
        source: need(&o, "source")?.into(),
        coverage: need(&o, "coverage")?.into(),
        license: o.get("license").cloned().unwrap_or("ODbL-1.0".into()),
        attribution: o
            .get("attribution")
            .cloned()
            .unwrap_or("(c) OpenStreetMap contributors".into()),
        builder,
        center,
    };
    let inputs = finish::Inputs {
        tiles: Path::new(need(&o, "tiles")?),
        rows: Path::new(need(&o, "rows")?),
        poly: Path::new(need(&o, "poly")?),
    };
    let stats = finish::run(&inputs, &d, Path::new(need(&o, "output")?))
        .map_err(|e| format!("finish: {e}"))?;
    eprint!("{}", stats.report());
    Ok(())
}

fn run_natural_earth(args: &[String]) -> Result<(), String> {
    let mut keys = vec!["rows", "output"];
    keys.extend(overview::Kind::ALL.iter().map(|k| k.option()));
    let (o, rest) = parse(args, &keys)?;
    if !rest.is_empty() {
        return Err("natural-earth takes no positional arguments".into());
    }
    let mut rows =
        BufWriter::new(File::create_new(need(&o, "rows")?).map_err(|e| format!("--rows: {e}"))?);
    let mut features = BufWriter::new(
        File::create_new(need(&o, "output")?).map_err(|e| format!("--output: {e}"))?,
    );
    let mut stats = overview::Stats::default();
    let mut any = false;
    for kind in overview::Kind::ALL {
        let Some(path) = o.get(kind.option()) else {
            continue;
        };
        any = true;
        let len = std::fs::metadata(path)
            .map_err(|e| format!("{path}: {e}"))?
            .len();
        if len > overview::MAX_FILE_BYTES {
            return Err(format!("{path}: too large"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        overview::convert(kind, &text, &mut features, &mut rows, &mut stats)
            .map_err(|e| format!("{path}: {e}"))?;
    }
    if !any {
        return Err("no Natural Earth file given".into());
    }
    features.flush().map_err(|e| format!("--output: {e}"))?;
    rows.flush().map_err(|e| format!("--rows: {e}"))?;
    eprint!("{}", stats.report());
    Ok(())
}

fn run_make(args: &[String]) -> Result<(), String> {
    let mut req = make::Request {
        config: resilience_core::mapmake::CONFIG_PATH.into(),
        ..Default::default()
    };
    let mut rest: Vec<String> = Vec::new();
    for a in args {
        if a == "--yes" {
            req.yes = true;
        } else {
            rest.push(a.clone());
        }
    }
    let (o, pos) = parse(&rest, &["config", "region", "pbf", "poly"])?;
    if !pos.is_empty() {
        return Err("make takes no positional arguments".into());
    }
    if let Some(c) = o.get("config") {
        req.config = c.into();
    }
    req.region = o.get("region").cloned();
    req.pbf = o.get("pbf").map(Into::into);
    req.poly = o.get("poly").map(Into::into);
    make::make(&req).map(|_| ())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("classify") => run_classify(&args[1..]),
        Some("finish") => run_finish(&args[1..]),
        Some("natural-earth") => run_natural_earth(&args[1..]),
        Some("bounds") => run_bounds(&args[1..]),
        Some("sea") => run_sea(&args[1..]),
        Some("make") => run_make(&args[1..]),
        Some("--version") => {
            println!("resilience-mapmaker {}", resilience_core::VERSION);
            return ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
