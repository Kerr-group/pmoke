use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

fn main() {
    // Re-run the build script if these env vars change
    println!("cargo:rerun-if-env-changed=GPIB_LIB_DIR");
    println!("cargo:rerun-if-env-changed=NI_GPIB_LIB_DIR");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "windows" {
        link_for_windows();
        return;
    }

    // --- Non-Windows path (Linux, macOS, *BSD, etc.) ---

    // 1) Explicit override wins
    if let Some(dir) = env::var_os("GPIB_LIB_DIR") {
        link_unix(&PathBuf::from(dir), &target_os);
        return;
    }

    // 2) pkg-config (best when available)
    //    If found, it will emit all the right cargo metadata and we can exit.
    if try_pkg_config() {
        return;
    }

    // 3) Heuristic search in common locations
    if let Some(dir) = find_unix_lib_dir(&target_os) {
        link_unix(&dir, &target_os);
        return;
    }

    println!(
        "cargo:warning=Could not locate linux-gpib (libgpib). \
              Set GPIB_LIB_DIR=/path/to/lib or ensure pkg-config provides \"gpib\"."
    );
}

// ---------- Windows ----------

fn link_for_windows() {
    // Prefer env-provided lib directory (either key).
    // This is where gpib-32.lib usually resides (build-time).
    if let Some(dir) = env::var_os("GPIB_LIB_DIR").or_else(|| env::var_os("NI_GPIB_LIB_DIR")) {
        let dir = PathBuf::from(dir);
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    // Link to NI-488.2 import lib. At runtime, gpib-32.dll must be in PATH or next to the exe.
    println!("cargo:rustc-link-lib=dylib=gpib-32");
}

// ---------- Unix-like (Linux/macOS/*BSD) ----------

fn try_pkg_config() -> bool {
    // Use pkg-config only on non-Windows targets.
    // If "gpib" is present, pkg-config will print link search paths and link-libs itself.
    // Return true if probe succeeded.
    match pkg_config::Config::new().probe("gpib") {
        Ok(_) => true,
        Err(_) => false,
    }
}

fn link_unix(dir: &Path, target_os: &str) {
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=dylib=gpib");

    // Embed RPATH on ELF platforms so the loader can find libgpib.so at runtime.
    // Skip on macOS: prefer using install_name / rpath via other means if necessary.
    if target_os != "macos" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
    }
}

fn find_unix_lib_dir(target_os: &str) -> Option<PathBuf> {
    let mut candidates = default_unix_candidates(target_os);
    add_nix_candidates(&mut candidates);

    // Deduplicate to avoid redundant filesystem checks
    let mut seen = HashSet::new();
    candidates.retain(|p| seen.insert(p.clone()));

    let names: &[&str] = match target_os {
        "macos" => &["libgpib.dylib"],
        _ => &["libgpib.so", "libgpib.so.0", "libgpib.so.1"],
    };

    for dir in candidates {
        if contains_any(&dir, names) {
            return Some(dir);
        }
    }
    None
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
