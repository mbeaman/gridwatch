# UNVERIFIED — no `nix` on the machine this was written on, so this flake has
# never been evaluated. It is written from nixpkgs' `rustPlatform` conventions
# and the same facts as `packaging/nfpm.yaml` (which CI does build and
# inspect). Treat the first `nix build` as the test, and please report what it
# says.
#
# The interesting part is `outputHashes`: gridwatch depends on astral-watch by
# git revision, so Nix needs the hash of that fetch. It changes whenever the
# pinned revision does.
{
  description = "gridwatch — a modular, themeable ops dashboard for the terminal";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "gridwatch";
          version = "0.9.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            # Replace with the hash `nix build` prints on the first attempt.
            outputHashes = {
              "astral-watch-0.7.0" = pkgs.lib.fakeHash;
            };
          };

          buildFeatures = [ "cpu" "gpu" "pins" "audio" "sensors" "mpris" "net" "net-probe" ];

          # Nothing is linked that is not pure Rust: NVML is dlopened,
          # pw-record is spawned, D-Bus is spoken over a socket. So there are
          # no buildInputs beyond the toolchain — which is also why the binary
          # runs on a machine with none of them and says so.
          nativeBuildInputs = [ pkgs.pkg-config ];

          # The pty suite drives the binary under util-linux `script`; it skips
          # visibly without it, but the sandbox has it.
          nativeCheckInputs = [ pkgs.util-linux ];

          postInstall = ''
            mkdir -p $out/share/gridwatch
            cp -r themes $out/share/gridwatch/
            install -Dm0644 packaging/udev/90-gridwatch-rapl.rules \
              $out/share/gridwatch/udev/90-gridwatch-rapl.rules
          '';

          meta = with pkgs.lib; {
            description = "A modular, themeable ops dashboard for the terminal";
            homepage = "https://github.com/mbeaman/gridwatch";
            license = licenses.mit;
            mainProgram = "gridwatch";
            platforms = platforms.linux;
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt clippy cargo-deny util-linux python3 ];
        };
      });
}
