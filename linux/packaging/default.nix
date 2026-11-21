{ lib, rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "campus-bjtu";
  version = "1.0.0";
  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type: !(builtins.elem (baseNameOf path) [ "target" "artifacts" "build" ".gradle" ]);
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [ "-p" "campus-cli" ];
  cargoTestFlags = [ "--workspace" ];
  postInstall = ''
    mkdir -p $out/share/bash-completion/completions $out/share/zsh/site-functions $out/share/fish/vendor_completions.d
    $out/bin/campus completions bash > $out/share/bash-completion/completions/campus
    $out/bin/campus completions zsh > $out/share/zsh/site-functions/_campus
    $out/bin/campus completions fish > $out/share/fish/vendor_completions.d/campus.fish
  '';
  meta = {
    description = "交大自由行 CLI (campus) - Offline-first BJTU campus CLI";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "campus";
  };
}
