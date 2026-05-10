use std::fs::File;
use std::path::Path;
use std::process::Command;

use fs2::FileExt;

fn main() {
    // Only build frontend assets when the embedded-frontend feature is enabled.
    if std::env::var("CARGO_FEATURE_EMBEDDED_FRONTEND").is_err() {
        return;
    }

    // Allow skipping the frontend build in Docker where assets are built separately.
    if std::env::var("SKIP_FRONTEND_BUILD").is_ok() {
        return;
    }

    let frontend_dir = Path::new("../hydra-web");

    // The embedded-frontend feature is enabled, so the frontend source must be present.
    if !frontend_dir.join("package.json").exists() {
        panic!(
            "hydra-web directory not found at {} — required for embedded-frontend feature",
            frontend_dir.display()
        );
    }

    // Acquire an exclusive file lock to prevent concurrent pnpm builds from
    // racing on shared output directories (e.g., ui/dist, web/dist).
    let lock_path = frontend_dir.join(".build-lock");
    let lock_file = File::create(&lock_path).expect("failed to create build lock file");
    lock_file
        .lock_exclusive()
        .expect("failed to acquire build lock");

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
                panic!("pnpm install failed with status {s}");
            }
            Err(e) => {
                panic!("pnpm not found ({e}), cannot build frontend");
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
            panic!("Frontend build failed with status {s}");
        }
        Err(e) => {
            panic!("Failed to run frontend build: {e}");
        }
    }

    // Lock is released when lock_file is dropped.
    drop(lock_file);

    // Tell cargo to re-run if the frontend source changes.
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/src");
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/index.html");
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/vite.config.ts");
}
