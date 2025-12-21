use std::{fs, path::Path};

#[test]
fn all_rhai_scripts_compile() {
    let scripts_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    assert!(
        scripts_dir.is_dir(),
        "scripts directory missing: {}",
        scripts_dir.display()
    );

    let mut scripts = Vec::new();
    for entry in fs::read_dir(&scripts_dir).expect("failed to read scripts directory") {
        let entry = entry.expect("failed to read scripts directory entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rhai") {
            scripts.push(path);
        }
    }

    scripts.sort();
    assert!(
        !scripts.is_empty(),
        "no Rhai scripts found in {}",
        scripts_dir.display()
    );

    let mut engine = rhai::Engine::new();
    engine.set_max_expr_depths(256, 256);
    engine.set_max_call_levels(128);
    engine.set_max_operations(50_000);

    for script in scripts {
        engine
            .compile_file(script.clone())
            .unwrap_or_else(|err| panic!("failed to compile {}: {err}", script.display()));
    }
}
