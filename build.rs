//! Build script: stamp a per-build cache id (the git short rev) into the
//! environment as `BUILD_REV`, so the `GET /sw.js` handler can replace the
//! `__BUILD_REV__` placeholder in `public/sw.js`. That makes the service
//! worker's `CACHE_VERSION` unique on every release without a manual bump,
//! which drives the in-app "new version available" refresh prompt.

use std::process::Command;

fn main() {
    let rev = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dev".to_string());

    println!("cargo:rustc-env=BUILD_REV={rev}");
    // Rebuild when HEAD moves: `.git/HEAD` catches branch switches, and
    // `.git/logs/HEAD` is appended on every commit/checkout/reset/merge.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/logs/HEAD");
}
