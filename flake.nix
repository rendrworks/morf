{
  description = "morf Rust development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?rev=4c1018dae018162ec878d42fec712642d214fdfa";
    flake-utils.url = "github:numtide/flake-utils";
    nixgl.url = "github:nix-community/nixGL";
    # A rust toolchain that can be asked for another target's standard library.
    # The one in nixpkgs carries the host's and only the host's, which is enough
    # until the day something has to run on a phone.
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    { nixpkgs, flake-utils, nixgl, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [
          rust-overlay.overlays.default
          (final: prev: {
            xorg = prev.xorg // {
              libX11 = final.libx11;
              libxcb = final.libxcb;
              libxshmfence = final.libxshmfence;
            };
          })
        ];

        pkgs = import nixpkgs {
          inherit system overlays;
          config = {
            allowUnfree = true;
            nvidia.acceptLicense = true;
          };
        };

        nvidiaVersion = builtins.getEnv "NVIDIA_VERSION";
        hasNvidia = nvidiaVersion != "";

        nixglPkgs = import "${nixgl}/default.nix" ({
          inherit pkgs;
        } // pkgs.lib.optionalAttrs hasNvidia {
          inherit nvidiaVersion;
          nvidiaHash = null;
        });

        nixGLTarget =
          if hasNvidia
          then "${nixglPkgs.nixGLNvidia}/bin/nixGLNvidia-${nvidiaVersion}"
          else "${nixglPkgs.nixGLIntel}/bin/nixGLIntel";
        nixVulkanTarget =
          if hasNvidia
          then "${nixglPkgs.nixVulkanNvidia}/bin/nixVulkanNvidia-${nvidiaVersion}"
          else "${nixglPkgs.nixVulkanIntel}/bin/nixVulkanIntel";

        nixGLAlias = pkgs.runCommand "nixGL" { } ''
          mkdir -p $out/bin
          ln -s ${nixGLTarget} $out/bin/nixGL
        '';
        nixVulkanAlias = pkgs.runCommand "nixVulkan" { } ''
          mkdir -p $out/bin
          ln -s ${nixVulkanTarget} $out/bin/nixVulkan
        '';

        # postmarketOS is Alpine underneath, so the target is musl and not glibc.
        # Getting that wrong produces a binary that links, ships, and then dies
        # on the device looking for an interpreter that was never there.
        crossTriple = "aarch64-unknown-linux-musl";
        crossPkgs = import nixpkgs {
          inherit system overlays;
          crossSystem.config = crossTriple;
        };
        crossCc = "${crossPkgs.stdenv.cc}/bin/${crossPkgs.stdenv.cc.targetPrefix}cc";
        # The two libraries the engine actually links against. Vulkan is not one
        # of them — wgpu opens the loader at run time, so the device's own
        # driver is found on the device and nothing is needed here.
        crossLibs = [ crossPkgs.wayland crossPkgs.libxkbcommon ];

        guiLibs = with pkgs; [
          alsa-lib
          udev
          vulkan-loader
          libxkbcommon
          wayland
          libx11
          libxcursor
          libxi
          libxrandr
        ];
      in
      {
        # `nix develop .#cross-aarch64` — then `cargo build --release
        # --target aarch64-unknown-linux-musl`, and the binary runs on the
        # phone. Cross-compiling means building the target's libraries from
        # source the first time, because there is no binary cache for them;
        # after that it is as quick as any other build here, and quicker than
        # the device managing it itself.
        devShells.cross-aarch64 = pkgs.mkShell {
          packages = [
            (pkgs.rust-bin.stable.latest.default.override {
              targets = [ crossTriple ];
            })
            pkgs.pkg-config
            crossPkgs.stdenv.cc
          ];

          CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER = crossCc;
          CC_aarch64_unknown_linux_musl = crossCc;
          # Dynamic, not static. A static musl binary cannot load the device's
          # own `libwayland-client`, and that is the whole point of the exercise.
          #
          # And pointed at the device's loader by absolute path. Without this the
          # binary asks for the musl that built it — a `/nix/store` path that
          # exists on this machine and nowhere else — and the device answers
          # "no such file or directory" about a file that is plainly there,
          # because the file it cannot find is the interpreter and not the
          # binary.
          CARGO_BUILD_RUSTFLAGS =
            "-C target-feature=-crt-static "
            + "-C link-arg=-Wl,--dynamic-linker=/lib/ld-musl-aarch64.so.1";
          PKG_CONFIG_ALLOW_CROSS = "1";
          # Only the cross libraries, so pkg-config cannot find this machine's
          # own and hand back an x86 path that links and then does not run.
          PKG_CONFIG_LIBDIR =
            pkgs.lib.concatStringsSep ":"
              (map (lib: "${lib.dev}/lib/pkgconfig") crossLibs);
          # And emptied, because `PKG_CONFIG_PATH` is searched *as well as*
          # `PKG_CONFIG_LIBDIR`, not instead of it. Whatever loaded the ordinary
          # shell — direnv, usually — leaves this machine's own `.pc` files on
          # it, and the linker then finds an x86 library, says it is skipping
          # something incompatible, and fails having never looked anywhere else.
          PKG_CONFIG_PATH = "";
          # Host libraries have no business on a cross build's search path.
          LD_LIBRARY_PATH = "";
        };

        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.rustc
            pkgs.cargo
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rust-analyzer
            pkgs.git-cliff
            pkgs.clang
            pkgs.morf
            pkgs.pkg-config

            nixGLAlias
            nixVulkanAlias
            nixglPkgs.nixGLIntel
            nixglPkgs.nixVulkanIntel
          ] ++ pkgs.lib.optionals hasNvidia [
            nixglPkgs.nixGLNvidia
            nixglPkgs.nixVulkanNvidia
          ] ++ guiLibs;

          # A musl toolchain for the static build, handed over as a path rather than a package.
          # As a package its headers land on the default search path, and an ordinary build then
          # compiles against musl while linking against glibc -- which succeeds without a word and
          # crashes at startup. Only the static build is given it: .make.lua reads MUSL_CC.
          # gcc targeting musl, which is the only one of the two that has a C++ standard library.
          MUSL_CC = pkgs.pkgsMusl.stdenv.cc;
          # musl-clang: the host clang, pointed at musl's headers and libs. C only -- it has no
          # libstdc++, so a C++ build against it fails on the first #include <string>.
          MUSL_CLANG = pkgs.musl.dev;

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibs;
          WGPU_VALIDATION = "0";
          WGPU_DEBUG = "0";
        };
      }
    );
}
