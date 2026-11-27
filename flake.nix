# Teskad on Nix and NixOS.
#
#   nix develop                 a shell with everything `scripts/native-dev.sh` and
#                               `cargo test --workspace` need (docs/guide/building.md)
#   nix build                   Teskad, its webview helper and Teskad Mobile, in ./result
#   nix run                     the same, started
#
# The window needs nothing at build time: winit and wgpu load Wayland, X11,
# xkbcommon, Vulkan and EGL with dlopen when they start. On NixOS those
# libraries are in no folder the loader searches, so the package puts them in
# the binary's RUNPATH (and the shell in LD_LIBRARY_PATH) rather than wrapping
# the binary: the app writes its own path into the agents' MCP configs and the
# login item, and a wrapper's renamed `.teskad-wrapped` would be the path
# they kept.
{
  description = "Teskad: run a team of Claude Code agents in one native window";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # What winit and wgpu open at run time.
      runtimeLibs =
        pkgs: with pkgs; [
          wayland
          libxkbcommon
          vulkan-loader
          libGL
          libx11
          libxcursor
          libxi
          libxrandr
        ];

      # What the build links: ALSA for dictation (cpal), OpenSSL, and GTK and
      # WebKitGTK for the webview helper's engine.
      buildLibs =
        pkgs: with pkgs; [
          alsa-lib
          openssl
          glib
          gtk3
          libsoup_3
          webkitgtk_4_1
        ];
    in
    {
      packages = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.teskad;
        teskad = pkgs.callPackage ./nix/package.nix {
          inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) hangar-mobile;
          runtimeLibs = runtimeLibs pkgs;
          buildLibs = buildLibs pkgs;
        };
        hangar-mobile = pkgs.callPackage ./nix/mobile.nix { };
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.teskad}/bin/teskad";
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            pkg-config
            cmake
            clang
            nodejs_22
            pnpm_10
          ];
          buildInputs = buildLibs pkgs;
          # whisper.cpp's bindings are generated with libclang (bindgen).
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
