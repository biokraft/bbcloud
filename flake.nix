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

            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./tests
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;

            # The Linux Secret Service backend builds its vendored OpenSSL with Perl.
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.perl ];
            nativeCheckInputs = [ pkgs.gitMinimal ];

            # Some table tests inspect whether stdout is a terminal. Redirect the test
            # harness so the Nix builder provides the same non-TTY environment as CI.
            checkPhase = ''
              runHook preCheck
              if ! cargo test --release --target ${pkgs.stdenv.hostPlatform.rust.rustcTarget} --offline > cargo-test.log; then
                cat cargo-test.log
                exit 1
              fi
              cat cargo-test.log
              runHook postCheck
            '';

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
