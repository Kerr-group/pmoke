{
  description = "Reproducible development shell for pmoke";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/70ce234312134a463ba7728e94da2486a1d237ac";
  };

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          python = pkgs.python312.withPackages (pythonPackages: with pythonPackages; [ numpy ]);
        in
        {
          default = pkgs.mkShell {
            packages =
              (with pkgs; [
                cargo
                clippy
                lld
                nodejs_22
                (writeShellScriptBin "pnpm" ''
                  exec ${nodejs_22}/bin/node ${pnpm}/bin/pnpm "$@"
                '')
                python
                rustc
                rustfmt
                wasm-bindgen-cli_0_2_126
                wasm-pack
              ])
              ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.chromium ];

            shellHook = ''
              export PYO3_PYTHON="${python}/bin/python3.12"
              export PYTHONPATH="${python}/lib/python3.12/site-packages''${PYTHONPATH:+:$PYTHONPATH}"
              if [ -z "''${PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH:-}" ]; then
                if command -v chromium >/dev/null 2>&1; then
                  export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="$(command -v chromium)"
                elif command -v google-chrome >/dev/null 2>&1; then
                  export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="$(command -v google-chrome)"
                fi
              fi
            '';
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt-tree);
    };
}
