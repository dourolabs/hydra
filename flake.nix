{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    rust-overlay,
    ...
  }: let
    rust-stable = {rust-bin}: rust-bin.stable.latest.default.override {
      extensions = ["rust-src" "rustfmt" "clippy" "rust-analyzer"];
      targets = ["wasm32-unknown-unknown"];
    };

    dev-shell = {
      mkShell,
      rust-stable,
      pkg-config,
      openssl,
      git,
      kubectl,
      kind,
      coreutils,
      gnugrep,
      dioxus-cli,
      wasm-bindgen-cli,
      k9s
    }:
      mkShell {
        FORCE_COLOR = 1;
        name = "dev-shell";
        buildInputs = [
          rust-stable
          pkg-config
          openssl
          git
          kubectl
          kind
          coreutils
          gnugrep
          k9s
          dioxus-cli
          wasm-bindgen-cli
        ];
      };

    overlays = let
      mkOverlay = pkg-name: pkg: composedOverlays:
        nixpkgs.lib.composeManyExtensions (composedOverlays
          ++ [
            (final: _: {"${pkg-name}" = final.callPackage pkg {};})
          ]);
    in {
      rust-stable = mkOverlay "rust-stable" rust-stable [rust-overlay.overlays.default];
      dev-shell = mkOverlay "dev-shell" dev-shell [
        overlays.rust-stable
      ];
    };
  in
    (flake-utils.lib.eachDefaultSystem
      (
        system: let
          pkg-from-overlay = overlay-name:
            (import nixpkgs {
              inherit system;
              overlays = [overlays."${overlay-name}"];
              config = {};
            })."${overlay-name}";
        in {
          packages = nixpkgs.lib.mapAttrs (name: _: pkg-from-overlay name) overlays;
          devShells.default = pkg-from-overlay "dev-shell";
        }
      ))
    // {
      inherit overlays;
    };
}
