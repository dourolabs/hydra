use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let web_dir = Path::new(&manifest_dir).join("../metis-web");
    let dist_dir = web_dir.join("packages/web/dist");

    // Only run pnpm build if the hydra-web directory exists (it may not
    // during cross-compilation or in stripped source trees).
    if !web_dir.exists() {
        println!(
            "cargo:warning=metis-web directory not found at {}; skipping frontend build",
            web_dir.display()
        );
        ensure_dist_dir(&dist_dir);
        return;
    }

    // Re-run this build script when frontend source files change.
    println!("cargo:rerun-if-changed=../metis-web/packages/web/src");
    println!("cargo:rerun-if-changed=../metis-web/packages/web/index.html");
    println!("cargo:rerun-if-changed=../metis-web/packages/web/vite.config.ts");
    println!("cargo:rerun-if-changed=../metis-web/packages/ui/src");
    println!("cargo:rerun-if-changed=../metis-web/packages/api/src");

    // If dist already exists with content, skip the build.
    if dist_dir.join("index.html").exists() {
        return;
    }

    // Install dependencies if needed, then build.
    let status = Command::new("pnpm")
        .args(["install", "--frozen-lockfile"])
        .current_dir(&web_dir)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            println!("cargo:warning=pnpm install exited with status {s}; skipping frontend build");
            ensure_dist_dir(&dist_dir);
            return;
        }
        Err(e) => {
            println!("cargo:warning=pnpm not found ({e}); skipping frontend build");
            ensure_dist_dir(&dist_dir);
            return;
        }
    }

    let status = Command::new("pnpm")
        .args(["--filter", "@metis/web...", "build"])
        .current_dir(&web_dir)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            println!(
                "cargo:warning=pnpm build failed with status {s}; frontend will not be embedded"
            );
            ensure_dist_dir(&dist_dir);
        }
        Err(e) => {
            println!("cargo:warning=failed to run pnpm build: {e}; frontend will not be embedded");
            ensure_dist_dir(&dist_dir);
        }
    }
}

/// Ensure the dist directory exists (even if empty) so that rust-embed
/// does not fail at compile time.
fn ensure_dist_dir(dist_dir: &Path) {
    if !dist_dir.exists() {
        std::fs::create_dir_all(dist_dir).ok();
    }
}
