{
  description = "bb — Bitbucket Cloud CLI";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs, ... }:
    let
      supportedSystems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          bbcloud = pkgs.rustPlatform.buildRustPackage {
            pname = manifest.package.name;
            inherit (manifest.package) version;

            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # The Linux Secret Service backend builds its vendored OpenSSL with Perl.
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.perl ];
            doCheck = false;

            meta = {
              inherit (manifest.package) description homepage;
              license = pkgs.lib.licenses.mit;
              mainProgram = "bb";
            };
          };
        in
        {
          inherit bbcloud;
          default = bbcloud;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${nixpkgs.lib.getExe self.packages.${system}.default}";
          meta.description = "Run the bb Bitbucket Cloud CLI";
        };
      });

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt-tree);
    };
}
