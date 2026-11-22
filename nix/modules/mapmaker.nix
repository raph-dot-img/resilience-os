# Making a map on this computer (ROADMAP Phase 5b, ADR 0016 §10):
# Maps & navigation -> "Make or update a map" runs `resilience-mapmaker
# make` in the foreground as the person. It downloads a Geofabrik extract
# only when asked (HTTPS, MD5 checked, resumed) or reads one from a USB
# stick, builds it with osmium and tippecanoe like the catalog packs (no
# sea), and writes ~/Maps/map-osm-ID_YYYY-MM.mbtiles. Nothing privileged.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.resilience.mapmaker;
  maps = pkgs.callPackage ../maps { resilience = config.resilience.package; };
  settings = (pkgs.formats.toml { }).generate "resilience-mapmaker.toml" {
    program = lib.getExe maps.mapmaker;
    regions = "${cfg.regions}";
    export_config = "${../maps/osmium-export.json}";
    osmium = lib.getExe' pkgs.osmium-tool "osmium";
    tippecanoe = lib.getExe' pkgs.tippecanoe "tippecanoe";
    curl = lib.getExe pkgs.curl;
    md5sum = "${pkgs.coreutils}/bin/md5sum";
    tools = "osmium-tool ${pkgs.osmium-tool.version}, tippecanoe ${pkgs.tippecanoe.version}";
  };
in
{
  options.resilience.mapmaker.regions = lib.mkOption {
    type = lib.types.path;
    default = ../maps/regions.json;
    description = ''
      The regions offered (JSON list: id, name, parent, pbf and poly URLs on
      download.geofabrik.de), from Geofabrik's index by scripts/map-regions.py.
    '';
  };

  options.resilience.mapmaker.enable = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Offer "Make or update a map" in Maps & navigation: the person builds
      a map of a region (downloaded from Geofabrik on request, or an
      .osm.pbf extract on a USB stick) into ~/Maps. Adds osmium and
      tippecanoe to the system (standard edition).
    '';
  };

  config = lib.mkIf cfg.enable {
    environment.etc."resilience/mapmaker.toml".source = settings;
  };
}
