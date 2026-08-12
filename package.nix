{
  lib,
  perl,
  rustPlatform,
  stdenv,
}:

let
  manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name;
  inherit (manifest.package) version;

  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;

  # The Linux Secret Service backend builds its vendored OpenSSL with Perl.
  nativeBuildInputs = lib.optionals stdenv.isLinux [ perl ];
  doCheck = false;

  meta = {
    inherit (manifest.package) description homepage;
    license = lib.licenses.mit;
    mainProgram = "bb";
  };
}
