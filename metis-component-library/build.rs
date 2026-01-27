use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() -> io::Result<()> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
    let mut scss_files = Vec::new();
    collect_scss_files(&manifest_dir, &mut scss_files)?;

    for path in &scss_files {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    for path in scss_files {
        compile_scss(&path, &manifest_dir)?;
    }

    Ok(())
}

fn collect_scss_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_scss_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("scss") {
            let is_partial = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.starts_with('_'));
            if !is_partial {
                files.push(path);
            }
        }
    }

    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | "target"))
}

fn compile_scss(path: &Path, manifest_dir: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or(manifest_dir);
    let options = grass::Options::default()
        .load_path(parent)
        .load_path(manifest_dir);

    let css = grass::from_path(path, &options).map_err(|err| io::Error::other(err.to_string()))?;
    let css_path = path.with_extension("css");

    if let Ok(existing) = fs::read_to_string(&css_path) {
        if existing == css {
            return Ok(());
        }
    }

    fs::write(css_path, css)?;
    Ok(())
}
