use std::path::{Path, PathBuf};
use std::process::Command;

/// Try to find the `pnpm` binary, checking PATH first, then well-known
/// node version manager locations (NVM, Volta, fnm).
fn find_pnpm() -> String {
    // 1. Check if pnpm is available on PATH.
    if Command::new("pnpm")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
    {
        return "pnpm".to_string();
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
            // Collect version dirs and sort descending to pick the highest version.
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
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let web_dir = Path::new(&manifest_dir).join("../hydra-web");
    let dist_dir = web_dir.join("packages/web/dist");

    // Only run pnpm build if the hydra-web directory exists (it may not
    // during cross-compilation or in stripped source trees).
    if !web_dir.exists() {
        println!(
            "cargo:warning=hydra-web directory not found at {}; skipping frontend build",
            web_dir.display()
        );
        ensure_dist_dir(&dist_dir);
        return;
    }

    // Re-run this build script when frontend source files change.
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/src");
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/index.html");
    println!("cargo:rerun-if-changed=../hydra-web/packages/web/vite.config.ts");
    println!("cargo:rerun-if-changed=../hydra-web/packages/ui/src");
    println!("cargo:rerun-if-changed=../hydra-web/packages/api/src");

    // If dist already exists with content, skip the build.
    if dist_dir.join("index.html").exists() {
        return;
    }

    // Install dependencies if needed, then build.
    let pnpm = find_pnpm();

    let status = Command::new(&pnpm)
        .args(["install", "--frozen-lockfile"])
        .current_dir(&web_dir)
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

    let status = Command::new(&pnpm)
        .args(["--filter", "@hydra/web...", "build"])
        .current_dir(&web_dir)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            panic!("pnpm build failed with status {s}");
        }
        Err(e) => {
            panic!("Failed to run pnpm build: {e}");
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
