# Adds the Resilience packages to nixpkgs.
final: _prev: {
  resilience = final.callPackage ../packages/resilience { };
  resilience-gui = final.callPackage ../packages/resilience-gui { };
  resilience-session = final.callPackage ../packages/resilience-session { };
  resilience-handbook = final.callPackage ../packages/resilience-handbook { };
  resilience-world-map = final.callPackage ../packages/resilience-world-map { };
}
