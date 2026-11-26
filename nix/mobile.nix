# Hangar Mobile, the phone app: `pnpm mobile:build` into dist-mobile/, which
# the desktop binary embeds (crates/hangar-core/src/remote/assets.rs).
{
  lib,
  stdenvNoCC,
  nodejs_22,
  pnpm_10,
  fetchPnpmDeps,
  pnpmConfigHook,
}:

let
  version = (lib.importJSON ../package.json).version;
in
stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "hangar-mobile";
  inherit version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../package.json
      ../pnpm-lock.yaml
      ../pnpm-workspace.yaml
      ../mobile
    ];
  };

  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs) pname version src;
    pnpm = pnpm_10;
    fetcherVersion = 5;
    hash = "sha256-72NtgMopPchcP5j79XzN+f4gEQrcjcj60gTJqjIuUqE=";
  };

  nativeBuildInputs = [
    nodejs_22
    pnpm_10
    pnpmConfigHook
  ];

  buildPhase = ''
    runHook preBuild
    pnpm mobile:build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    cp -r dist-mobile $out
    runHook postInstall
  '';
})
