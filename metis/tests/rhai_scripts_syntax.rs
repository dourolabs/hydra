use metis::constants;
use std::path::Path;

fn compile_script(script: &Path) {
    let mut engine = rhai::Engine::new();
    engine.set_max_expr_depths(
        constants::RHAI_MAX_EXPR_DEPTHS.0,
        constants::RHAI_MAX_EXPR_DEPTHS.1,
    );
    engine.set_max_call_levels(constants::RHAI_MAX_CALL_LEVELS);
    engine.set_max_operations(constants::RHAI_MAX_OPERATIONS);

    engine
        .compile_file(script.to_path_buf())
        .unwrap_or_else(|err| panic!("failed to compile {}: {err}", script.display()));
}

include!(concat!(env!("OUT_DIR"), "/rhai_script_tests.rs"));
