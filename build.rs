use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SDKROOT");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        configure_macos_link_search();
    }
}

fn configure_macos_link_search() {
    if let Ok(output) = Command::new("xcrun").args(["--show-sdk-path"]).output()
        && output.status.success()
        && let Ok(sdk_path) = String::from_utf8(output.stdout)
    {
        let sdk_path = sdk_path.trim();
        if !sdk_path.is_empty() {
            let sdk_usr_lib = std::path::Path::new(sdk_path).join("usr/lib");
            if sdk_usr_lib.exists() {
                println!("cargo:rustc-link-search=native={}", sdk_usr_lib.display());
            }
        }
    }
}
