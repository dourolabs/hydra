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

macro_rules! script_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            compile_script(Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/",
                $file
            )));
        }
    };
}

script_test!(compile_fix_merge, "fix_merge.rhai");
script_test!(compile_fix_pr, "fix_pr.rhai");
script_test!(compile_patch_pr, "patch_pr.rhai");
script_test!(compile_patch_with_review, "patch_with_review.rhai");
