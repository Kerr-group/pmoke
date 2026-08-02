use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    track_git_path("HEAD");
    track_git_path("packed-refs");
    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        track_git_path(&reference);
    }
    let commit = std::env::var("GITHUB_SHA")
        .ok()
        .or_else(|| git_output(&["rev-parse", "HEAD"]));
    let commit = commit
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| "development".to_string());
    println!("cargo:rustc-env=PMOKE_SOURCE_COMMIT={commit}");
}

fn track_git_path(name: &str) {
    if let Some(path) = git_output(&["rev-parse", "--git-path", name]) {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
