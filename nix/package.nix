# Teskad, its webview helper (hangar-webview, for HTML kit tabs and PDFs) and
# the embedded Hangar Mobile, as the release workflow builds them for Linux.
{
  lib,
  rustPlatform,
  pkg-config,
  cmake,
  patchelf,
  wrapGAppsHook3,
  glib-networking,
  gst_all_1,
  hangar-mobile,
  runtimeLibs,
  buildLibs,
}:

let
  manifest = (lib.importTOML ../Cargo.toml).workspace.package;
in
rustPlatform.buildRustPackage {
  pname = "teskad";
  inherit (manifest) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.difference ../. (
      lib.fileset.unions [
        (lib.fileset.maybeMissing ../target)
        (lib.fileset.maybeMissing ../node_modules)
        (lib.fileset.maybeMissing ../dist-mobile)
        (lib.fileset.maybeMissing ../result)
        ../docs
        ../nix
        ../flake.nix
        (lib.fileset.maybeMissing ../flake.lock)
      ]
    );
  };

  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [
    "-p"
    "hangar-app"
    "-p"
    "hangar-webview"
    "--features"
    "hangar-webview/engine"
  ];

  nativeBuildInputs = [
    pkg-config
    cmake
    rustPlatform.bindgenHook # whisper.cpp's bindings, generated with libclang
    patchelf
    wrapGAppsHook3
  ];

  buildInputs =
    buildLibs
    ++ [
      glib-networking # TLS for the pages the webview helper loads
    ]
    ++ (with gst_all_1; [
      gst-plugins-base
      gst-plugins-good
    ]);

  # cmake is here for whisper.cpp, which its crate's build script drives:
  # the package itself is built by cargo.
  dontUseCmakeConfigure = true;

  # The phone app the binary embeds (crates/hangar-core/build.rs).
  preBuild = ''
    cp -r ${hangar-mobile} dist-mobile
    chmod -R u+w dist-mobile
  '';

  # The workspace's tests start shells, git and Claude Code mods; CI runs them.
  doCheck = false;

  postInstall = ''
    install -Dm644 crates/hangar-app/packaging/linux/teskad.desktop \
      $out/share/applications/teskad.desktop
    for size in 16 24 32 48 64 128 256 512; do
      install -Dm644 crates/hangar-app/packaging/icons/linux/''${size}x''${size}.png \
        $out/share/icons/hicolor/''${size}x''${size}/apps/teskad.png
    done
    install -Dm644 crates/hangar-app/packaging/icons/linux/teskad.svg \
      $out/share/icons/hicolor/scalable/apps/teskad.svg
  '';

  # Only the helper is a GTK program; the app is not wrapped (flake.nix says why).
  dontWrapGApps = true;

  # After fixupPhase's own patchelf --shrink-rpath, which would take out these
  # entries: nothing in the binary names them, winit and wgpu dlopen them.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibs} $out/bin/teskad
    wrapGApp $out/bin/hangar-webview
  '';

  meta = {
    description = "Run a team of Claude Code agents in one native window, and steer them from your phone";
    homepage = "https://github.com/BlasterWhite/hangar";
    license = lib.licenses.mit;
    mainProgram = "teskad";
    platforms = lib.platforms.linux;
  };
}
