{ lib
, rustPlatform
, rust-bin
, makeRustPlatform
, makeWrapper
, libGL
, libxkbcommon
, wayland
, xorg
, nebulaServer ? "https://svyaz.msk1.catpaws.ru"
}:


let
  cargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);
  # egui открывает эти библиотеки через dlopen, на NixOS их надо показать явно
  runtimeLibs = [ libGL libxkbcommon wayland xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr ];

  toolchain = rust-bin.stable."1.88.0".minimal;
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };

in
rustPlatform.buildRustPackage {
  pname = "nebula-client";
  version = cargoToml.workspace.package.version;

  src = lib.cleanSourceWith {
    src = ../.;
    # dist/linux подключается ниже отдельными путями
    filter = path: type: !(lib.elem (baseNameOf path) [ "target" "dist" ".git" "result" ]);
  };

  cargoLock.lockFile = ../Cargo.lock;
  nativeBuildInputs = [ makeWrapper ];
  env.NEBULA_CLIENT_SERVER = nebulaServer;

  # тесты трогают только разбор имён адаптеров, сеть им не нужна
  doCheck = true;

  postInstall = ''
    wrapProgram $out/bin/nebula-client --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
    install -Dm644 ${../dist/linux/nebula-client.desktop} $out/share/applications/nebula-client.desktop
    install -Dm644 ${../dist/linux/nebula-client.png} $out/share/icons/hicolor/256x256/apps/nebula-client.png
  '';

  meta = {
    description = "Join Nebula overlay networks managed by nebula-panel";
    homepage = "https://github.com/Arseniy07955/nebula-client";
    license = lib.licenses.gpl3Only;
    mainProgram = "nebula-client";
    platforms = lib.platforms.linux;
  };
}
