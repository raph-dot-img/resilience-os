# Making a map on this computer (ROADMAP Phase 5b), driven through the real
# TUI on tty1 by an Ultra-Stable account. A second machine plays
# download.geofabrik.de over HTTPS (test certificate): the "latest" extract
# redirects to a dated file, with its .md5 and the region's .poly; the
# extract is the synthetic Testland (nix/maps/test-osm.nix).
#
# Covers: Maps & navigation -> Make or update a map, typing a region's
# name, the builder asking before it downloads, a download resumed from a
# partial file, the map built into ~/Maps and checked by the viewer's
# reader, then listed as "Made here" and offered to the viewer; a damaged
# download refused (MD5) and deleted; an extract from a file without its
# .poly (the extract's rectangle); the builder never runs as root and
# leaves no work files.
{ self }:
{ hostPkgs, ... }:
let
  pkgs = hostPkgs;
  host = "download.geofabrik.de";

  # Test-only TLS certificate (its key is no secret).
  testCert = pkgs.runCommand "test-geofabrik-cert" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir $out
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
      -keyout $out/key.pem -out $out/cert.pem -days 3650 -subj /CN=${host} \
      -addext subjectAltName=DNS:${host}
  '';

  test = import ../../nix/maps/test-osm.nix { inherit (pkgs) writeText; };

  site = pkgs.runCommand "test-geofabrik" { nativeBuildInputs = [ pkgs.osmium-tool ]; } ''
    d=$out/europe/testland
    mkdir -p $d $out/broken
    osmium cat --no-progress \
      --output-header=osmosis_replication_timestamp=2026-01-01T00:00:00Z \
      -o $d/testland-260101.osm.pbf -f pbf ${test.osm}
    (cd $d && md5sum testland-260101.osm.pbf > testland-260101.osm.pbf.md5)
    cp ${test.poly} $d/testland.poly
    # Same bytes, a checksum that does not match.
    cp $d/testland-260101.osm.pbf $out/broken/broken-260101.osm.pbf
    echo "00000000000000000000000000000000  broken-260101.osm.pbf" > $out/broken/broken-260101.osm.pbf.md5
    cp ${test.poly} $out/broken/broken.poly
  '';

  regions = pkgs.writeText "regions.json" (
    builtins.toJSON [
      {
        id = "europe";
        name = "Europe";
        pbf = "https://${host}/europe-latest.osm.pbf";
        poly = "https://${host}/europe.poly";
      }
      {
        id = "testland";
        name = "Testland";
        parent = "europe";
        pbf = "https://${host}/europe/testland/testland-latest.osm.pbf";
        poly = "https://${host}/europe/testland/testland.poly";
      }
      {
        id = "broken";
        name = "Brokenland";
        parent = "europe";
        pbf = "https://${host}/broken/broken-latest.osm.pbf";
        poly = "https://${host}/broken/broken.poly";
      }
    ]
  );
in
{
  name = "resilience-mapmaker";

  nodes.geofabrik = {
    services.nginx = {
      enable = true;
      commonHttpConfig = ''
        log_format ranges '$request $status range=$http_range';
      '';
      virtualHosts.${host} = {
        onlySSL = true;
        sslCertificate = "${testCert}/cert.pem";
        sslCertificateKey = "${testCert}/key.pem";
        root = site;
        extraConfig = ''
          access_log /var/log/nginx/ranges.log ranges;
          port_in_redirect off;
        '';
        # As Geofabrik: "latest" is a redirect to the dated file.
        locations."= /europe/testland/testland-latest.osm.pbf".return =
          "307 https://${host}/europe/testland/testland-260101.osm.pbf";
        locations."= /broken/broken-latest.osm.pbf".return =
          "307 https://${host}/broken/broken-260101.osm.pbf";
      };
    };
    networking.firewall.allowedTCPPorts = [ 443 ];
  };

  nodes.machine =
    { lib, nodes, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1536;
      virtualisation.cores = 2;
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
      resilience.mapmaker = {
        enable = true;
        inherit regions;
      };
      security.pki.certificateFiles = [ "${testCert}/cert.pem" ];
      networking.hosts.${nodes.geofabrik.networking.primaryIPAddress} = [ host ];
      networking.defaultGateway = nodes.geofabrik.networking.primaryIPAddress;
    };

  testScript = ''
    import base64, re, time as _time

    def tty_text(tty):
        cols = int(machine.wait_until_succeeds(f"stty -F /dev/tty{tty} size", timeout=10).split()[1])
        raw = base64.b64decode(machine.succeed(f"base64 -w0 /dev/vcsu{tty}"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(tty, regexp, timeout=60):
        deadline = _time.monotonic() + timeout
        while True:
            text = tty_text(tty)
            if re.search(regexp, text):
                return text
            if _time.monotonic() > deadline:
                print(f"--- tty{tty} contents ---")
                print(text)
                print(machine.execute("cat /home/alice/.local/state/resilience/tui.log")[1][-3000:])
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    def as_alice(cmd):
        return f"runuser -u alice -- sh -c 'cd && {cmd}'"

    host = "${host}"
    MAP = "/home/alice/Maps/map-osm-testland_2026-01.mbtiles"
    CACHE = "/home/alice/.cache/resilience/maps"

    start_all()
    geofabrik.wait_for_unit("nginx.service")
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    wait_tty("1", "MAIN MENU")
    machine.succeed("test -f /etc/resilience/mapmaker.toml")
    def setting(f, key):
        return machine.succeed(f"grep -o '^{key} = \"[^\"]*' {f}").split('"')[1].strip()
    MAKE = setting("/etc/resilience/mapmaker.toml", "program") + " make"
    CURL = setting("/etc/resilience/mapmaker.toml", "curl")
    VIEWER = setting("/etc/resilience/system.toml", "maps")

    with subtest("a partial download is waiting (an earlier attempt stopped)"):
        machine.succeed(as_alice(f"mkdir -p {CACHE}"))
        machine.succeed(
            f"{CURL} -sS https://{host}/europe/testland/testland-260101.osm.pbf | head -c 500 "
            f"> {CACHE}/testland-260101.osm.pbf.part && chown alice: {CACHE}/testland-260101.osm.pbf.part"
        )

    with subtest("Maps & navigation > Make or update a map, type the region, Enter"):
        text = wait_tty("1", "MAPS & NAVIGATION")
        number = re.search(r"\[(\d)\] MAPS & NAVIGATION", text)
        assert number is not None, text
        machine.send_chars(number.group(1))
        text = wait_tty("1", "Make or update a map")
        number = re.search(r"\[(\d)\] Make or update a map", text)
        assert number is not None, text
        machine.send_chars(number.group(1))
        wait_tty("1", "MAKE OR UPDATE A MAP")
        wait_tty("1", "Europe > Testland")
        machine.send_chars("testl")
        wait_tty("1", r"REGIONS \(1\)")
        machine.send_key("ret")

    with subtest("the builder asks first, then downloads (resumed), checks and builds"):
        text = wait_tty("1", r"Start\? \[y/N\]", timeout=120)
        assert "Map of Europe > Testland" in text, text
        assert "download.geofabrik.de" in text, text
        # Nothing but the HEAD request so far.
        machine.fail(f"test -e {MAP}")
        machine.succeed("pgrep -u alice -f 'resilience-mapmake[r] make'")
        machine.fail("pgrep -u root -f 'resilience-mapmake[r] make'")
        machine.send_chars("y\n")
        text = wait_tty("1", "Press Enter to return", timeout=300)
        assert "Done:" in text, text
        assert "OpenStreetMap data of 2026-01-01" in text, text
        machine.send_key("ret")
        wait_tty("1", "MAKE OR UPDATE A MAP")
        wait_tty("1", r"made here: data of 2026-01")
        served = geofabrik.succeed("cat /var/log/nginx/ranges.log")
        print(served)
        assert re.search(r"GET /europe/testland/testland-260101.osm.pbf HTTP/[0-9.]+ 206 range=bytes=500-", served), served
        assert re.search(r"HEAD /europe/testland/testland-latest.osm.pbf HTTP/[0-9.]+ 307", served), served

    with subtest("the map is the person's, checked, and nothing is left behind"):
        machine.succeed(f"stat -c %U {MAP} | grep -x alice")
        info = machine.succeed(as_alice(f"{VIEWER} info --check {MAP}"))
        print(info)
        assert "integrity: ok" in info, info
        assert "profile: resilience-1" in info, info
        assert "data-date: 2026-01-01" in info, info
        assert "Geofabrik extract boundary" in info, info
        assert "search: available" in info, info
        machine.fail(f"ls {CACHE}/build")
        machine.fail(f"ls {CACHE} | grep -q 'osm.pbf'")
        machine.fail("ls -a /home/alice/Maps | grep -q partial")

    with subtest("listed as Made here and opened with the other maps"):
        machine.send_key("esc")
        text = wait_tty("1", "Maps on this computer")
        number = re.search(r"\[(\d)\] Maps on this computer", text)
        assert number is not None, text
        machine.send_chars(number.group(1))
        text = wait_tty("1", "Testland")
        assert "Made here" in text, text
        machine.send_key("esc")
        machine.send_key("esc")
        wait_tty("1", "MAIN MENU")

    with subtest("a damaged download is refused and deleted; the map is unchanged"):
        before = machine.succeed(f"sha256sum {MAP}")
        status, out = machine.execute(as_alice(f"{MAKE} --region broken --yes 2>&1"))
        print(out)
        assert status != 0, out
        assert "damaged" in out, out
        machine.fail(f"ls {CACHE} | grep -q 'broken-.*pbf'")
        machine.fail("ls /home/alice/Maps | grep -q broken")
        assert machine.succeed(f"sha256sum {MAP}") == before

    with subtest("an extract from a file without its .poly covers its rectangle"):
        machine.succeed(
            f"mkdir -p /srv/stick && {CURL} -sS -o /srv/stick/testland-260101.osm.pbf "
            f"https://{host}/europe/testland/testland-260101.osm.pbf && chmod -R a+rX /srv/stick"
        )
        out = machine.succeed(as_alice(f"{MAKE} --pbf /srv/stick/testland-260101.osm.pbf --yes 2>&1"))
        print(out)
        assert "rectangle" in out, out
        info = machine.succeed(as_alice(f"{VIEWER} info --check {MAP}"))
        assert "integrity: ok" in info and "rectangle" in info, info
        assert "from a file" in info, info
        # The file is read in place, never removed.
        machine.succeed("test -s /srv/stick/testland-260101.osm.pbf")

    with subtest("an unknown region and a foreign address are refused"):
        status, out = machine.execute(as_alice(f"{MAKE} --region atlantis --yes 2>&1"))
        assert status != 0 and "no region" in out, out
  '';
}
