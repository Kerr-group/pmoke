use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

fn main() {
    // Re-run the build script if these env vars change
    println!("cargo:rerun-if-env-changed=GPIB_LIB_DIR");
    println!("cargo:rerun-if-env-changed=NI_GPIB_LIB_DIR");
    println!("cargo:rerun-if-env-changed=VISA_LIB_DIR");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "windows" {
        link_for_windows();
        return;
    }

    // --- Non-Windows path (Linux, macOS, *BSD, etc.) ---

    if let Some(dir) = env::var_os("GPIB_LIB_DIR") {
        link_unix(&PathBuf::from(dir), &target_os);
        return;
    }

    if try_pkg_config() {
        return;
    }

    if let Some(dir) = find_unix_lib_dir(&target_os) {
        link_unix(&dir, &target_os);
        return;
    }

    println!(
        "cargo:warning=Could not locate linux-gpib (libgpib). \
              Set GPIB_LIB_DIR=/path/to/lib or ensure pkg-config provides \"gpib\"."
    );
}

// ---------- Windows (VISA版) ----------

fn link_for_windows() {
    // VISA_LIB_DIR が指定されていればそれを優先
    // なければ 64-bit VISA のデフォルトパスを使う
    let visa_dir = env::var_os("VISA_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // 一般的な IVI VISA パス (環境によって異なる場合あり)
            PathBuf::from(r"C:\Program Files\IVI Foundation\VISA\Win64\Lib_x64\msc")
        });

    println!("cargo:rustc-link-search=native={}", visa_dir.display());
    // 64-bit の VISA import ライブラリ
    println!("cargo:rustc-link-lib=dylib=visa64");

    println!("cargo:warning=Linking against visa64 (VISA). Using VISA API wrapper.");
}

// ---------- Unix-like (Linux/macOS/*BSD) ----------
fn try_pkg_config() -> bool {
    pkg_config::Config::new().probe("gpib").is_ok()
}

fn link_unix(dir: &Path, target_os: &str) {
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=dylib=gpib");
    if target_os != "macos" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
    }
}

fn find_unix_lib_dir(target_os: &str) -> Option<PathBuf> {
    let mut candidates = default_unix_candidates(target_os);
    add_nix_candidates(&mut candidates);

    let mut seen = HashSet::new();
    candidates.retain(|p| seen.insert(p.clone()));

    let names: &[&str] = match target_os {
        "macos" => &["libgpib.dylib"],
        _ => &["libgpib.so", "libgpib.so.0", "libgpib.so.1"],
    };

    candidates.into_iter().find(|dir| contains_any(dir, names))
}

fn default_unix_candidates(target_os: &str) -> Vec<PathBuf> {
    // Common Unix library locations
    let mut v = vec![
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/lib64"),
        PathBuf::from("/usr/local/lib"),
    ];

    // Linux multiarch & popular prefixes
    if target_os != "macos" {
        v.push(PathBuf::from("/lib"));
        v.push(PathBuf::from("/lib/x86_64-linux-gnu"));
        v.push(PathBuf::from("/opt/local/lib"));
    }

    // Homebrew (Apple Silicon / Intel) and MacPorts
    if target_os == "macos" {
        v.push(PathBuf::from("/opt/homebrew/lib")); // Apple Silicon default
        v.push(PathBuf::from("/usr/local/lib")); // Intel Homebrew
        v.push(PathBuf::from("/opt/local/lib")); // MacPorts
    }

    v
}

fn add_nix_candidates(candidates: &mut Vec<PathBuf>) {
    // NixOS: lib may live under /nix/store/*linux-gpib*/lib
    if let Ok(paths) = glob::glob("/nix/store/*linux-gpib*/lib") {
        for p in paths.flatten() {
            candidates.push(p);
        }
    }
}

fn contains_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| dir.join(name).exists())
}