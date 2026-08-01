declare module '*/pmoke_web_wasm.js' {
  export default function init(): Promise<unknown>;
  export function validate_config_toml(toml_str: string): string;
  export function build_info(): string;
}
