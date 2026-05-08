use std::path::{Path, PathBuf};
use std::process::Command;

/// Try to find the `pnpm` binary, checking PATH first, then well-known
/// node version manager locations (NVM, Volta, fnm).
fn find_pnpm() -> String {
    // 1. Check if pnpm is available on PATH using a which-style lookup.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("pnpm");
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }

    // 2. Check well-known node manager locations.
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return "pnpm".to_string(),
    };

    // NVM: $NVM_DIR/versions/node/*/bin/pnpm or ~/.nvm/versions/node/*/bin/pnpm
    let nvm_dir = std::env::var("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".nvm"));
    let nvm_versions = nvm_dir.join("versions/node");
    if nvm_versions.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&nvm_versions) {
            let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            dirs.sort_by(|a, b| {
                let va = a
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace('v', "");
                let vb = b
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace('v', "");
                let parse =
                    |s: &str| -> Vec<u64> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
                parse(&vb).cmp(&parse(&va))
            });
            for dir in dirs {
                let candidate = dir.join("bin/pnpm");
                if candidate.is_file() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
    }

    // Volta: $VOLTA_HOME/bin/pnpm or ~/.volta/bin/pnpm
    let volta_home = std::env::var("VOLTA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".volta"));
    let volta_pnpm = volta_home.join("bin/pnpm");
    if volta_pnpm.is_file() {
        return volta_pnpm.to_string_lossy().into_owned();
    }

    // fnm: $FNM_MULTISHELL_PATH/bin/pnpm or ~/.local/share/fnm/aliases/default/bin/pnpm
    if let Ok(fnm_path) = std::env::var("FNM_MULTISHELL_PATH") {
        let candidate = PathBuf::from(&fnm_path).join("bin/pnpm");
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    let fnm_default = home.join(".local/share/fnm/aliases/default/bin/pnpm");
    if fnm_default.is_file() {
        return fnm_default.to_string_lossy().into_owned();
    }

    // Fallback to bare name (will produce a clear error if not found).
    "pnpm".to_string()
}

fn main() {
    // Only build frontend assets when the embedded-frontend feature is enabled.
    if std::env::var("CARGO_FEATURE_EMBEDDED_FRONTEND").is_err() {
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
    // Install dependencies if needed.
    let pnpm = find_pnpm();

    if !frontend_dir.join("node_modules").exists() {
        println!("cargo:warning=Installing frontend dependencies...");
        let status = Command::new(&pnpm)
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
    let status = Command::new(&pnpm)
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

    // Tell cargo to re-run if the frontend source changes.
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/src");
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/index.html");
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/vite.config.ts");
}
