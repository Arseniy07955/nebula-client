{
  description = "Nebula Client: join Nebula overlay networks managed by nebula-panel";

  inputs = { 
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      in
        f pkgs
      );
    in
    {
      packages = forAll (pkgs: {
        default = pkgs.callPackage ./nix/package.nix { 
          rust-bin = pkgs.rust-bin;
        };
      });

      nixosModules.default = import ./nix/module.nix self;

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = [ pkgs.nebula ];
        };
      });
    };
}
