//! Making a map on this computer (ROADMAP Phase 5b, ADR 0016 §10).
//!
//! The person picks a region of Geofabrik's list (shipped offline in the
//! system, `regions.json`), or an `.osm.pbf` extract on a USB stick;
//! `resilience-mapmaker make` then downloads the extract on request (HTTPS,
//! Geofabrik's checksum, resumed), builds the map in the foreground as the
//! person with the same steps as the catalog packs (osmium, tippecanoe,
//! the builder's own `classify` and `finish`), and writes it into `~/Maps`.
//!
//! This module holds what both the menus and the builder need, all pure:
//! the configuration file, the region list, the room a build needs, the
//! output file names and the small text formats read on the way.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Written by the NixOS module when map making is on.
pub const CONFIG_PATH: &str = "/etc/resilience/mapmaker.toml";

/// The folder of the maps made here, in the person's home.
pub const MAPS_FOLDER: &str = "Maps";

/// Downloads and the work folder, in the person's home.
pub const CACHE_FOLDER: &str = ".cache/resilience/maps";

/// Only extracts from this site are downloaded.
pub const SOURCE_PREFIX: &str = "https://download.geofabrik.de/";

/// Extracts larger than this are refused (a whole country of the size of
/// Germany needs more memory than the computers Resilience is meant for).
pub const MAX_PBF_BYTES: u64 = 2 << 30;

/// `/etc/resilience/mapmaker.toml`: absolute store paths on Resilience.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MakerConfig {
    /// `resilience-mapmaker` itself.
    pub program: String,
    pub regions: PathBuf,
    /// osmium's export configuration (nix/maps).
    pub export_config: PathBuf,
    pub osmium: String,
    pub tippecanoe: String,
    pub curl: String,
    pub md5sum: String,
    /// Tool versions, written into the map's metadata.
    pub tools: String,
}

impl MakerConfig {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// One region of Geofabrik's list.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// `https://download.geofabrik.de/…/ID-latest.osm.pbf`.
    pub pbf: String,
    /// The region's boundary, `…/ID.poly`.
    pub poly: String,
}

/// A region id: what Geofabrik uses, and part of a file name here.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

fn valid_source_url(u: &str, suffix: &str) -> bool {
    u.len() <= 300
        && u.strip_prefix(SOURCE_PREFIX).is_some_and(|rest| {
            rest.ends_with(suffix)
                && !rest.contains("..")
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
        })
}

/// Reads the region list. Every entry is checked: an id and a name, the
/// two URLs on Geofabrik's site; entries that fail are an error (the list
/// is built by us, a bad one must not be half used).
pub fn parse_regions(text: &str) -> Result<Vec<Region>, String> {
    let list: Vec<Region> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut seen = std::collections::BTreeSet::new();
    for r in &list {
        if !valid_id(&r.id) {
            return Err(format!("bad region id {:?}", r.id));
        }
        if !seen.insert(r.id.as_str()) {
            return Err(format!("region {} listed twice", r.id));
        }
        if r.name.trim().is_empty() || r.name.len() > 100 || r.name.chars().any(char::is_control) {
            return Err(format!("bad name for region {}", r.id));
        }
        if !valid_source_url(&r.pbf, "-latest.osm.pbf") || !valid_source_url(&r.poly, ".poly") {
            return Err(format!("bad URL for region {}", r.id));
        }
    }
    for r in &list {
        if let Some(p) = &r.parent
            && !seen.contains(p.as_str())
        {
            return Err(format!("region {}: unknown parent {p}", r.id));
        }
    }
    Ok(list)
}

pub fn load_regions(path: &Path) -> Result<Vec<Region>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_regions(&text)
}

/// "Europe > France > Bretagne": the region with the regions it is in.
pub fn region_path(list: &[Region], id: &str) -> String {
    let mut names = Vec::new();
    let mut cur = list.iter().find(|r| r.id == id);
    while let Some(r) = cur {
        names.push(r.name.as_str());
        if names.len() > 8 {
            break;
        }
        cur = r
            .parent
            .as_deref()
            .and_then(|p| list.iter().find(|x| x.id == p));
    }
    names.reverse();
    names.join(" > ")
}

/// Regions whose path contains every word of `query` (case and accents
/// folded the simple way), in list order; all of them for an empty query.
pub fn search<'a>(list: &'a [Region], query: &str) -> Vec<&'a Region> {
    let words: Vec<String> = query.split_whitespace().map(fold).collect();
    list.iter()
        .filter(|r| {
            let hay = fold(&region_path(list, &r.id)) + " " + &r.id;
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            c => c,
        })
        .collect()
}

/// What a build of an extract of `pbf_bytes` needs, measured on the
/// catalog packs (docs/TESTING.md, "Make a map"): disk for the download,
/// osmium's and tippecanoe's intermediate files and the map; memory for
/// osmium (its node locations) and tippecanoe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    pub disk: u64,
    pub memory: u64,
}

pub fn needs(pbf_bytes: u64) -> Needs {
    const MIB: u64 = 1 << 20;
    Needs {
        disk: pbf_bytes.saturating_mul(12).max(200 * MIB),
        memory: pbf_bytes.saturating_mul(6).max(512 * MIB),
    }
}

/// Why a build cannot start, in plain words; `None` when it can.
pub fn refusal(pbf_bytes: u64, free_disk: u64, memory: u64) -> Option<String> {
    let n = needs(pbf_bytes);
    if pbf_bytes > MAX_PBF_BYTES {
        return Some(format!(
            "This extract is {}: too large to make into a map here (at most {}). Choose a \
             smaller region inside it.",
            human(pbf_bytes),
            human(MAX_PBF_BYTES)
        ));
    }
    if free_disk < n.disk {
        return Some(format!(
            "Making this map needs about {} of free disk space; this computer has {}.",
            human(n.disk),
            human(free_disk)
        ));
    }
    if memory < n.memory {
        return Some(format!(
            "Making this map needs about {} of memory; this computer has {}. Choose a \
             smaller region.",
            human(n.memory),
            human(memory)
        ));
    }
    None
}

/// "48 MB", "1.2 GB".
pub fn human(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    let mb = bytes as f64 / MB;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{} MB", mb.round().max(1.0) as u64)
    }
}

/// `map-osm-ID_YYYY-MM.mbtiles` for data of `date` (YYYY-MM-DD).
pub fn output_name(id: &str, date: &str) -> Option<String> {
    (valid_id(id) && valid_date(date)).then(|| format!("map-osm-{id}_{}.mbtiles", &date[..7]))
}

/// The region id of a map made here, from its file name.
pub fn made_id(file_name: &str) -> Option<&str> {
    let rest = file_name
        .strip_prefix("map-osm-")?
        .strip_suffix(".mbtiles")?;
    let (id, month) = rest.rsplit_once('_')?;
    (valid_id(id) && month.len() == 7 && valid_date(&format!("{month}-01"))).then_some(id)
}

pub fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && (1..=12).contains(&d[5..7].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&d[8..10].parse::<u8>().unwrap_or(0))
}

/// The date of an extract's data, from `osmium fileinfo -g
/// header.option.osmosis_replication_timestamp` (`2026-11-21T20:21:43Z`).
pub fn data_date(timestamp: &str) -> Option<String> {
    let d = timestamp.trim().get(..10)?;
    valid_date(d).then(|| d.to_string())
}

/// The checksum in Geofabrik's `.md5` file (`HEX  NAME`).
pub fn parse_md5(text: &str) -> Option<String> {
    let hex = text.split_whitespace().next()?;
    (hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_lowercase())
}

/// The size and final URL from `curl --head --location --write-out`
/// with the format `%{size_download} %{url_effective}`… curl prints the
/// headers of every hop: the last `content-length` wins.
pub fn parse_head(headers: &str) -> Option<u64> {
    headers
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<u64>().ok())?
        })
        .next_back()
}

/// The coverage of an extract that came without its `.poly`: the
/// rectangle of `osmium fileinfo -g header.boxes` (`(W,S,E,N)`), as a
/// `.poly` file.
pub fn box_poly(name: &str, boxes: &str) -> Option<String> {
    let inner = boxes.trim().strip_prefix('(')?.split(')').next()?;
    let v: Vec<f64> = inner
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    let [w, s, e, n] = v.as_slice() else {
        return None;
    };
    let ok = (-180.0..=180.0).contains(w)
        && (-180.0..=180.0).contains(e)
        && (-90.0..=90.0).contains(s)
        && (-90.0..=90.0).contains(n)
        && w < e
        && s < n;
    ok.then(|| {
        format!(
            "{name}\n1\n   {w:.7}   {s:.7}\n   {e:.7}   {s:.7}\n   {e:.7}   {n:.7}\n   \
             {w:.7}   {n:.7}\n   {w:.7}   {s:.7}\nEND\nEND\n"
        )
    })
}

/// Arguments of every download: HTTPS only (redirects too), bounded
/// times, failures as errors, never a config file of the person's.
pub fn curl_base(curl: &str) -> Vec<String> {
    [
        curl,
        "--disable",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--tlsv1.2",
        "--fail",
        "--location",
        "--max-redirs",
        "3",
        "--connect-timeout",
        "30",
        "--speed-limit",
        "1000",
        "--speed-time",
        "60",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const LIST: &str = r#"[
      {"id":"europe","name":"Europe","pbf":"https://download.geofabrik.de/europe-latest.osm.pbf","poly":"https://download.geofabrik.de/europe.poly"},
      {"id":"france","name":"France","parent":"europe","pbf":"https://download.geofabrik.de/europe/france-latest.osm.pbf","poly":"https://download.geofabrik.de/europe/france.poly"},
      {"id":"bretagne","name":"Bretagne","parent":"france","pbf":"https://download.geofabrik.de/europe/france/bretagne-latest.osm.pbf","poly":"https://download.geofabrik.de/europe/france/bretagne.poly"},
      {"id":"ile-de-france","name":"Île-de-France","parent":"france","pbf":"https://download.geofabrik.de/europe/france/ile-de-france-latest.osm.pbf","poly":"https://download.geofabrik.de/europe/france/ile-de-france.poly"}
    ]"#;

    #[test]
    fn region_list_paths_and_search() {
        let l = parse_regions(LIST).unwrap();
        assert_eq!(region_path(&l, "bretagne"), "Europe > France > Bretagne");
        let ids = |q| {
            search(&l, q)
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("bret"), ["bretagne"]);
        assert_eq!(ids("france ile"), ["ile-de-france"]);
        assert_eq!(ids("ÎLE"), ["ile-de-france"]);
        assert_eq!(ids("").len(), 4);
        assert!(ids("mars").is_empty());
    }

    #[test]
    fn region_list_rejects_foreign_urls_and_bad_ids() {
        let bad_url = LIST.replace(
            "https://download.geofabrik.de/europe/france-latest",
            "https://evil.example/france-latest",
        );
        assert!(parse_regions(&bad_url).is_err());
        let http = LIST.replace(
            "https://download.geofabrik.de/europe.poly",
            "http://download.geofabrik.de/europe.poly",
        );
        assert!(parse_regions(&http).is_err());
        let dots = LIST.replace("europe/france/bretagne.poly", "europe/../bretagne.poly");
        assert!(parse_regions(&dots).is_err());
        let id = LIST.replace("\"id\":\"bretagne\"", "\"id\":\"Bre tagne\"");
        assert!(parse_regions(&id).is_err());
        let parent = LIST.replace(
            "\"parent\":\"france\",\"pbf\":\"https://download.geofabrik.de/europe/france/bretagne",
            "\"parent\":\"nowhere\",\"pbf\":\"https://download.geofabrik.de/europe/france/bretagne",
        );
        assert!(parse_regions(&parent).is_err());
        assert!(!valid_id("-x") && !valid_id("") && valid_id("us-west"));
    }

    #[test]
    fn needs_and_refusals() {
        let mb = 1u64 << 20;
        assert_eq!(needs(48 * mb).disk, 576 * mb);
        assert_eq!(needs(1).memory, 512 * mb);
        assert!(refusal(48 * mb, 10 << 30, 4 << 30).is_none());
        assert!(
            refusal(48 * mb, 100 * mb, 4 << 30)
                .unwrap()
                .contains("disk space")
        );
        assert!(
            refusal(400 * mb, 10 << 30, 1 << 30)
                .unwrap()
                .contains("memory")
        );
        assert!(
            refusal(3 << 30, 1 << 40, 1 << 40)
                .unwrap()
                .contains("too large")
        );
        assert_eq!(human(48_004_963), "48 MB");
        assert_eq!(human(1_200_000_000), "1.2 GB");
    }

    #[test]
    fn names_dates_and_small_formats() {
        assert_eq!(
            output_name("luxembourg", "2026-11-21").as_deref(),
            Some("map-osm-luxembourg_2026-11.mbtiles")
        );
        assert_eq!(output_name("../x", "2026-11-21"), None);
        assert_eq!(
            made_id("map-osm-ile-de-france_2026-11.mbtiles"),
            Some("ile-de-france")
        );
        assert_eq!(made_id("map-osm-x_2026-13.mbtiles"), None);
        assert_eq!(made_id("wikipedia.zim"), None);
        assert_eq!(
            data_date("2026-11-21T20:21:43Z\n").as_deref(),
            Some("2026-11-21")
        );
        assert_eq!(data_date(""), None);
        assert_eq!(
            parse_md5("8583AB07ba6c99064bd3f585f2fe3338  luxembourg-latest.osm.pbf\n").as_deref(),
            Some("8583ab07ba6c99064bd3f585f2fe3338")
        );
        assert_eq!(parse_md5("<html>"), None);
        let h = "HTTP/1.1 307 Temporary Redirect\r\nContent-Length: 0\r\n\r\nHTTP/1.1 200 OK\r\ncontent-length: 48004963\r\n";
        assert_eq!(parse_head(h), Some(48_004_963));
        let p = box_poly("x", "(5.7,49.4,6.6,50.2)\n").unwrap();
        assert!(p.starts_with("x\n1\n") && p.ends_with("END\nEND\n"));
        assert!(box_poly("x", "(6.6,49.4,5.7,50.2)").is_none());
        assert!(box_poly("x", "nothing").is_none());
    }

    #[test]
    fn curl_is_https_only() {
        let a = curl_base("curl");
        assert!(a.windows(2).any(|w| w == ["--proto", "=https"]));
        assert!(a.windows(2).any(|w| w == ["--proto-redir", "=https"]));
        assert_eq!(a[1], "--disable");
    }

    #[test]
    fn config_parses() {
        let c = MakerConfig::parse(
            "program = \"/p\"\nregions = \"/r\"\nexport_config = \"/e\"\n\
             osmium = \"/o\"\ntippecanoe = \"/t\"\ncurl = \"/c\"\nmd5sum = \"/m\"\ntools = \"x\"\n",
        )
        .unwrap();
        assert_eq!(c.regions, PathBuf::from("/r"));
        assert!(MakerConfig::parse("program = 1").is_err());
    }
}
