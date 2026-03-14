use std::path::Path;
use std::process::Command;

fn main() {
    // Only build frontend assets when the embedded-frontend feature is enabled.
    if std::env::var("CARGO_FEATURE_EMBEDDED_FRONTEND").is_err() {
        return;
    }

    let frontend_dir = Path::new("../metis-web");
    let dist_dir = frontend_dir.join("packages/web/dist");

    // If the frontend source doesn't exist, skip the build gracefully.
    if !frontend_dir.join("package.json").exists() {
        println!(
            "cargo:warning=metis-web directory not found at {}, skipping frontend build",
            frontend_dir.display()
        );
        // Create an empty dist directory so rust-embed doesn't fail.
        if !dist_dir.exists() {
            std::fs::create_dir_all(&dist_dir).ok();
            // Create a minimal index.html so the embed has at least one file.
            std::fs::write(
                dist_dir.join("index.html"),
                "<!DOCTYPE html><html><body>Frontend not built</body></html>",
            )
            .ok();
        }
        return;
    }

    // If dist already exists with content, skip the build (dev workflow).
    // In CI/Docker the dist won't exist yet, so we build it.
    if dist_dir.join("index.html").exists() {
        println!("cargo:warning=Frontend dist already exists, skipping build");
        return;
    }

    // Install dependencies if needed.
    if !frontend_dir.join("node_modules").exists() {
        println!("cargo:warning=Installing frontend dependencies...");
        let status = Command::new("pnpm")
            .arg("install")
            .arg("--frozen-lockfile")
            .current_dir(frontend_dir)
            .status();

        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                println!(
                    "cargo:warning=pnpm install failed with status {s}, skipping frontend build"
                );
                ensure_dist_exists(&dist_dir);
                return;
            }
            Err(e) => {
                println!("cargo:warning=pnpm not found ({e}), skipping frontend build");
                ensure_dist_exists(&dist_dir);
                return;
            }
        }
    }

    // Build the frontend.
    println!("cargo:warning=Building frontend assets...");
    let status = Command::new("pnpm")
        .arg("run")
        .arg("build")
        .current_dir(frontend_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:warning=Frontend build completed successfully");
        }
        Ok(s) => {
            println!("cargo:warning=Frontend build failed with status {s}, creating placeholder");
            ensure_dist_exists(&dist_dir);
        }
        Err(e) => {
            println!("cargo:warning=Failed to run frontend build ({e}), creating placeholder");
            ensure_dist_exists(&dist_dir);
        }
    }

    // Tell cargo to re-run if the frontend source changes.
    println!("cargo:rerun-if-changed=../metis-web/packages/web/src");
    println!("cargo:rerun-if-changed=../metis-web/packages/web/index.html");
    println!("cargo:rerun-if-changed=../metis-web/packages/web/vite.config.ts");
}

fn ensure_dist_exists(dist_dir: &Path) {
    if !dist_dir.exists() {
        std::fs::create_dir_all(dist_dir).ok();
        std::fs::write(
            dist_dir.join("index.html"),
            "<!DOCTYPE html><html><body>Frontend not built</body></html>",
        )
        .ok();
    }
}
