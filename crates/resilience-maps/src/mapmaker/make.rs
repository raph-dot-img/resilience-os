//! `resilience-mapmaker make`: a map made on this computer, in the
//! foreground, as the person (ROADMAP Phase 5b, ADR 0016 §10).
//!
//! ```text
//! (--region ID: HTTPS download from Geofabrik, resumed, MD5 checked)
//!  | --pbf FILE (a USB stick; its .poly beside it, else the extract's box)
//!   -> osmium export | resilience-mapmaker classify
//!   -> tippecanoe -> resilience-mapmaker finish -> ~/Maps/map-osm-ID_YYYY-MM.mbtiles
//! ```
//!
//! The same steps and options as the catalog packs (`nix/maps/default.nix`),
//! without the sea and without `osmium tags-filter`: its ID sets take
//! about 2.2 GB whatever the region (Luxembourg, 2026-11-22), while export
//! straight from the extract gives the same features and rows (in another
//! order) in 180 MB. Every program runs from a fixed argv with absolute
//! paths from the root-owned configuration, a clean environment, and
//! `oom_score_adj` 1000 so that, if memory runs out, the kernel stops the
//! build and nothing else. Work files live in `~/.cache/resilience/maps`
//! and are removed at the end, or at the next start after an interruption.

use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use resilience_core::mapmake::{self, MakerConfig, Region};

/// What `make` was asked.
#[derive(Debug, Default)]
pub struct Request {
    pub config: PathBuf,
    pub region: Option<String>,
    pub pbf: Option<PathBuf>,
    pub poly: Option<PathBuf>,
    /// Do not ask before starting (tests).
    pub yes: bool,
}

struct Places {
    maps: PathBuf,
    cache: PathBuf,
    work: PathBuf,
}

fn places() -> Result<Places, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or("HOME is not set")?;
    let maps = home.join(mapmake::MAPS_FOLDER);
    let cache = home.join(mapmake::CACHE_FOLDER);
    for d in [&maps, &cache] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(d)
            .map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let work = cache.join("build");
    Ok(Places { maps, cache, work })
}

/// The kernel picks this process and its children first if memory runs out.
fn be_first_to_go() {
    if let Err(e) = fs::write("/proc/self/oom_score_adj", "1000") {
        eprintln!("(could not lower this build's priority for memory: {e})");
    }
}

fn free_disk(p: &Path) -> u64 {
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()) else {
        return 0;
    };
    // SAFETY: `c` is a valid NUL-terminated path; `st` is a plain struct
    // the call fills in; nothing is kept after the call.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return 0;
        }
        (st.f_bavail as u64).saturating_mul(st.f_frsize as u64)
    }
}

fn memory_total() -> u64 {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("MemTotal:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        })
        .map_or(0, |kib| kib * 1024)
}

fn command(program: &str, work: &Path) -> Command {
    let mut c = Command::new(program);
    c.env_clear()
        .env("PATH", "/run/current-system/sw/bin")
        .env("LC_ALL", "C")
        .env("TMPDIR", work)
        .current_dir(work);
    if let Some(h) = std::env::var_os("HOME") {
        c.env("HOME", h);
    }
    c
}

fn run(step: &str, c: &mut Command) -> Result<(), String> {
    let st = c.status().map_err(|e| format!("{step}: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{step} failed ({st})"))
    }
}

fn output(step: &str, c: &mut Command) -> Result<String, String> {
    let o = c
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("{step}: {e}"))?;
    if !o.status.success() {
        return Err(format!("{step} failed ({})", o.status));
    }
    String::from_utf8(o.stdout).map_err(|_| format!("{step}: not text"))
}

fn ask(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).is_ok()
        && matches!(line.trim(), "y" | "Y" | "yes" | "Yes" | "o" | "O" | "oui")
}

/// Where the latest extract really is, and its size (a HEAD request).
fn head(cfg: &MakerConfig, url: &str, work: &Path) -> Result<(String, u64), String> {
    let mut c = command(&cfg.curl, work);
    c.args(&mapmake::curl_base(&cfg.curl)[1..])
        .args(["--silent", "--show-error", "--head", "--max-time", "60"])
        .args([
            "--write-out",
            "\nRESILIENCE-URL %{url_effective}\n",
            "--",
            url,
        ]);
    let text = output("Asking Geofabrik for the extract", &mut c)?;
    let size = mapmake::parse_head(&text).ok_or("Geofabrik did not say the extract's size")?;
    let fin = text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("RESILIENCE-URL "))
        .unwrap_or(url)
        .trim()
        .to_string();
    let ok = fin.starts_with(mapmake::SOURCE_PREFIX)
        && fin.ends_with(".osm.pbf")
        && file_name(&fin).is_some();
    if !ok {
        return Err(format!("unexpected download address {fin:?}"));
    }
    Ok((fin, size))
}

fn file_name(url: &str) -> Option<&str> {
    let n = url.rsplit('/').next()?;
    let ok = n.len() <= 120
        && !n.starts_with('.')
        && n.ends_with(".osm.pbf")
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    ok.then_some(n)
}

fn download(
    cfg: &MakerConfig,
    url: &str,
    dest: &Path,
    work: &Path,
    resume: bool,
) -> Result<(), String> {
    let mut c = command(&cfg.curl, work);
    c.args(&mapmake::curl_base(&cfg.curl)[1..]);
    if resume {
        c.args(["--continue-at", "-", "--progress-bar"]);
    } else {
        c.args([
            "--silent",
            "--show-error",
            "--max-time",
            "120",
            "--max-filesize",
            "50000000",
        ]);
    }
    c.arg("--output").arg(dest).args(["--", url]);
    run("Downloading", &mut c)
}

fn md5(cfg: &MakerConfig, file: &Path, work: &Path) -> Result<String, String> {
    let mut c = command(&cfg.md5sum, work);
    c.arg("--").arg(file);
    let out = output("Checking the download", &mut c)?;
    mapmake::parse_md5(&out).ok_or_else(|| "md5sum gave no checksum".into())
}

/// The source extract on disk, its boundary, and what to call the map.
struct Source {
    pbf: PathBuf,
    poly: PathBuf,
    id: String,
    name: String,
    origin: String,
    coverage: String,
    /// Downloaded here: removed once the map is made.
    downloaded: bool,
}

fn from_download(
    cfg: &MakerConfig,
    r: &Region,
    list: &[Region],
    p: &Places,
    yes: bool,
) -> Result<Option<Source>, String> {
    println!(
        "Asking Geofabrik about {} ...",
        mapmake::region_path(list, &r.id)
    );
    let (url, size) = head(cfg, &r.pbf, &p.work)?;
    if !confirm(
        size,
        &mapmake::region_path(list, &r.id),
        &r.id,
        p,
        yes,
        true,
    )? {
        return Ok(None);
    }
    let base = file_name(&url).ok_or("bad file name")?.to_string();
    // Partial downloads of an older extract are of no use.
    if let Ok(rd) = fs::read_dir(&p.cache) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(&format!("{}-", r.id))
                && n.ends_with(".part")
                && n != format!("{base}.part")
            {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    println!("\nStep 1 of 4: downloading the boundary and the checksum");
    let poly = p.cache.join(format!("{}.poly", r.id));
    download(cfg, &r.poly, &p.work.join("region.poly"), &p.work, false)?;
    fs::rename(p.work.join("region.poly"), &poly).map_err(|e| e.to_string())?;
    download(
        cfg,
        &format!("{url}.md5"),
        &p.work.join("md5"),
        &p.work,
        false,
    )?;
    let want = mapmake::parse_md5(&fs::read_to_string(p.work.join("md5")).unwrap_or_default())
        .ok_or("Geofabrik's checksum file is not readable")?;
    println!(
        "\nDownloading the extract {base} ({}); if it stops, start again: it goes on where it stopped.",
        mapmake::human(size)
    );
    let part = p.cache.join(format!("{base}.part"));
    download(cfg, &url, &part, &p.work, true)?;
    println!("Checking the download ...");
    if md5(cfg, &part, &p.work)? != want {
        let _ = fs::remove_file(&part);
        return Err("the download is damaged (its checksum differs from Geofabrik's): it was deleted; start again".into());
    }
    let pbf = p.cache.join(&base);
    fs::rename(&part, &pbf).map_err(|e| e.to_string())?;
    Ok(Some(Source {
        pbf,
        poly,
        id: r.id.clone(),
        name: r.name.clone(),
        origin: format!("Geofabrik {base} md5:{want}"),
        coverage: format!("{} (Geofabrik extract boundary)", r.name),
        downloaded: true,
    }))
}

fn from_file(
    cfg: &MakerConfig,
    req: &Request,
    file: &Path,
    list: &[Region],
    p: &Places,
) -> Result<Option<Source>, String> {
    // The programs run in the work folder: absolute paths only.
    let file = &fs::canonicalize(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let req_poly = match &req.poly {
        Some(pp) => Some(fs::canonicalize(pp).map_err(|e| format!("{}: {e}", pp.display()))?),
        None => None,
    };
    let meta = fs::metadata(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", file.display()));
    }
    let fname = file
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.ends_with(".osm.pbf"))
        .ok_or("the extract must be an .osm.pbf file")?
        .to_string();
    let stem = fname.trim_end_matches(".osm.pbf");
    // "luxembourg-261121" or "luxembourg-latest": the region before the date.
    let guess = stem.rsplit_once('-').map_or(stem, |(a, b)| {
        if b.bytes().all(|c| c.is_ascii_digit()) || b == "latest" {
            a
        } else {
            stem
        }
    });
    let region = req
        .region
        .as_deref()
        .or_else(|| list.iter().any(|r| r.id == guess).then_some(guess))
        .and_then(|id| list.iter().find(|r| r.id == id));
    let id = region.map_or_else(|| guess.to_string(), |r| r.id.clone());
    if !mapmake::valid_id(&id) {
        return Err(format!(
            "cannot make a map name out of {fname:?}: rename the file (letters, digits and -)"
        ));
    }
    let name = region.map_or_else(|| id.clone(), |r| r.name.clone());
    let shown = region.map_or_else(|| name.clone(), |r| mapmake::region_path(list, &r.id));
    if !confirm(meta.len(), &shown, &id, p, req.yes, false)? {
        return Ok(None);
    }
    let beside = file.with_file_name(format!("{stem}.poly"));
    let beside_region = file.with_file_name(format!("{id}.poly"));
    let mut coverage = format!("{name} (the extract's boundary file)");
    let poly = match req_poly {
        Some(pp) => pp,
        None if beside.is_file() => beside,
        None if beside_region.is_file() => beside_region,
        None => {
            let mut c = command(&cfg.osmium, &p.work);
            c.args(["fileinfo", "-g", "header.boxes", "--"]).arg(file);
            let boxes = output("Reading the extract", &mut c)?;
            // Not every extract has a box in its header: then the box of
            // its data (osmium reads the whole file once).
            let text = match mapmake::box_poly(&id, &boxes) {
                Some(t) => t,
                None => {
                    let mut c = command(&cfg.osmium, &p.work);
                    c.args(["fileinfo", "-e", "-g", "data.bbox", "--"])
                        .arg(file);
                    let data = output("Reading the extract", &mut c)?;
                    mapmake::box_poly(&id, &data)
                        .ok_or("the extract has no boundary file beside it and no place in it")?
                }
            };
            println!(
                "(No {stem}.poly beside the extract: the map covers the extract's rectangle.)"
            );
            coverage = format!("{name} (the extract's rectangle)");
            let f = p.work.join("box.poly");
            fs::write(&f, text).map_err(|e| e.to_string())?;
            f
        }
    };
    Ok(Some(Source {
        pbf: file.to_path_buf(),
        poly,
        id,
        name,
        origin: format!("{fname} (from a file)"),
        coverage,
        downloaded: false,
    }))
}

fn older_maps(maps: &Path, id: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(maps)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().and_then(mapmake::made_id) == Some(id))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn confirm(
    size: u64,
    shown: &str,
    id: &str,
    p: &Places,
    yes: bool,
    download: bool,
) -> Result<bool, String> {
    let need = mapmake::needs(size);
    if let Some(why) = mapmake::refusal(size, free_disk(&p.cache), memory_total()) {
        return Err(why);
    }
    println!("\nMap of {shown}");
    if download {
        println!(
            "  Download: {} from download.geofabrik.de (OpenStreetMap data, ODbL)",
            mapmake::human(size)
        );
    } else {
        println!("  Extract: {}", mapmake::human(size));
    }
    println!(
        "  Needs about {} of free disk space while it works and {} of memory.",
        mapmake::human(need.disk),
        mapmake::human(need.memory)
    );
    println!(
        "  The map goes into {} (folder Maps of your home).",
        p.maps.display()
    );
    for o in older_maps(&p.maps, id) {
        println!(
            "  It replaces {} once it is made.",
            o.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        );
    }
    println!("  This takes from a few minutes to an hour or more; the computer stays usable.");
    Ok(yes || ask("\nStart?"))
}

/// Runs the whole thing; `Ok(None)` when the person said no.
pub fn make(req: &Request) -> Result<Option<PathBuf>, String> {
    let cfg = MakerConfig::load(&req.config)?;
    let list = mapmake::load_regions(&cfg.regions)?;
    let p = places()?;
    be_first_to_go();
    // A build interrupted earlier left its work folder.
    let _ = fs::remove_dir_all(&p.work);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&p.work)
        .map_err(|e| format!("{}: {e}", p.work.display()))?;
    let result = make_in(req, &cfg, &list, &p);
    let _ = fs::remove_dir_all(&p.work);
    result
}

fn make_in(
    req: &Request,
    cfg: &MakerConfig,
    list: &[Region],
    p: &Places,
) -> Result<Option<PathBuf>, String> {
    let src = match (&req.pbf, &req.region) {
        (Some(f), _) => from_file(cfg, req, f, list, p)?,
        (None, Some(id)) => {
            let r = list
                .iter()
                .find(|r| &r.id == id)
                .ok_or_else(|| format!("no region {id:?} in the list"))?;
            from_download(cfg, r, list, p, req.yes)?
        }
        (None, None) => return Err("give --region ID or --pbf FILE".into()),
    };
    let Some(src) = src else {
        println!("Nothing was done.");
        return Ok(None);
    };
    let w = &p.work;

    let mut c = command(&cfg.osmium, w);
    c.args([
        "fileinfo",
        "-g",
        "header.option.osmosis_replication_timestamp",
        "--",
    ])
    .arg(&src.pbf);
    let date = mapmake::data_date(&output("Reading the extract", &mut c)?)
        .ok_or("the extract does not say the date of its data")?;
    let out_name = mapmake::output_name(&src.id, &date).ok_or("bad map name")?;
    println!("\nOpenStreetMap data of {date}.");

    println!("\nStep 2 of 4: turning it into map features (osmium, classify)");
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut export = command(&cfg.osmium, w)
        .args(["export", "--no-progress", "-c"])
        .arg(&cfg.export_config)
        .args(["-f", "geojsonseq", "-o", "-", "--"])
        .arg(&src.pbf)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("osmium export: {e}"))?;
    let pipe = export.stdout.take().ok_or("osmium export: no output")?;
    let classify = command(&me.to_string_lossy(), w)
        .arg("classify")
        .arg("--poly")
        .arg(&src.poly)
        .args([
            "--rows",
            "rows.jsonl",
            "--output",
            "features.geojsonseq",
            "-",
        ])
        .stdin(pipe)
        .status();
    let exported = export.wait().map_err(|e| e.to_string())?;
    let classified = classify.map_err(|e| format!("classify: {e}"))?;
    if !exported.success() || !classified.success() {
        return Err(format!(
            "making the features failed (osmium {exported}, classify {classified})"
        ));
    }

    println!("Step 3 of 4: drawing the tiles (tippecanoe; the longest step)");
    run(
        "tippecanoe",
        command(&cfg.tippecanoe, w).args([
            "--quiet",
            "--output=tiles.mbtiles",
            "--read-parallel",
            "--minimum-zoom=0",
            "--maximum-zoom=14",
            "--no-tile-size-limit",
            "--no-feature-limit",
            "--drop-rate=1",
            "--no-tile-stats",
            &format!("--temporary-directory={}", w.display()),
            "features.geojsonseq",
        ]),
    )?;
    let _ = fs::remove_file(w.join("features.geojsonseq"));

    println!("Step 4 of 4: writing the map");
    let partial = p.maps.join(format!(".{out_name}.partial"));
    let _ = fs::remove_file(&partial);
    run(
        "finish",
        command(&me.to_string_lossy(), w)
            .args([
                "finish",
                "--tiles",
                "tiles.mbtiles",
                "--rows",
                "rows.jsonl",
                "--poly",
            ])
            .arg(&src.poly)
            .arg("--output")
            .arg(&partial)
            .args(["--name", &src.name])
            .args([
                "--description",
                &format!("OpenStreetMap data for {}, made on this computer", src.name),
            ])
            .args(["--data-date", &date])
            .args(["--source", &src.origin])
            .args(["--coverage", &src.coverage])
            .args(["--tools", &cfg.tools]),
    )?;
    let _ = fs::set_permissions(&partial, fs::Permissions::from_mode(0o644));
    let dest = p.maps.join(&out_name);
    fs::rename(&partial, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    for o in older_maps(&p.maps, &src.id) {
        if o != dest {
            let _ = fs::remove_file(&o);
            println!("Removed the older {}", o.display());
        }
    }
    if src.downloaded {
        let _ = fs::remove_file(&src.pbf);
    }
    let size = fs::metadata(&dest).map_or(0, |m| m.len());
    println!(
        "\nDone: {} ({}). Open it from Maps & navigation, then Map.",
        dest.display(),
        mapmake::human(size)
    );
    Ok(Some(dest))
}
