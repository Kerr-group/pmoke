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
          python = pkgs.python312.withPackages (
            pythonPackages:
            let
              scipy = pythonPackages.scipy.overridePythonAttrs (old: {
                # This precision property test fails with the pinned
                # NumPy/SciPy pair on the supported hosts. Keep all other
                # SciPy checks enabled.
                disabledTests = old.disabledTests ++ [ "test_support_moments_sample" ];
              });
              uncertainties = pythonPackages.uncertainties.overridePythonAttrs (old: {
                nativeCheckInputs = map (
                  dependency: if pkgs.lib.getName dependency == "scipy" then scipy else dependency
                ) old.nativeCheckInputs;
              });
              lmfit = pythonPackages.lmfit.overridePythonAttrs (old: {
                dependencies = map (
                  dependency:
                  if pkgs.lib.getName dependency == "scipy" then
                    scipy
                  else if pkgs.lib.getName dependency == "uncertainties" then
                    uncertainties
                  else
                    dependency
                ) old.dependencies;
              });
              gsplot = pythonPackages.buildPythonPackage rec {
                pname = "gsplot";
                version = "0.2.0";
                pyproject = true;

                src = pythonPackages.fetchPypi {
                  inherit pname version;
                  hash = "sha256-pZQmnAA4DRAAWQAD4mT3R8XSZYcRygsTVZoyT8XWBgM=";
                };

                build-system = [ pythonPackages.poetry-core ];
                dependencies = with pythonPackages; [
                  matplotlib
                  numpy
                  pyyaml
                  rich
                  types-pyyaml
                ];

                postInstall = ''
                  export HOME="$TMPDIR"
                '';

                pythonImportsCheck = [ "gsplot" ];

                meta = {
                  description = "General-scientific plot based on matplotlib";
                  homepage = "https://soichiroyamane.github.io/gsplot/";
                  license = pkgs.lib.licenses.mit;
                };
              };
            in
            [
              gsplot
              lmfit
              pythonPackages.matplotlib
              pythonPackages.numpy
              scipy
            ]
          );
        in
        {
          default = pkgs.mkShell {
            packages =
              (with pkgs; [
                cargo
                clippy
                git
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
              ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
                pkgs.chromium
                pkgs.linux-gpib
                pkgs.pkg-config
              ];

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
