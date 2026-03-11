use crate::test_utils::test_state;
use axum::Router;

#[test]
fn build_router_returns_composable_router() {
    let state = test_state();
    let base_router: Router<crate::app::AppState> = crate::build_router(&state);

    // Callers can merge additional routes before applying state.
    let extended = base_router.route("/custom", axum::routing::get(|| async { "custom" }));

    // Applying state produces a ready-to-serve Router<()>.
    let _app: Router<()> = extended.with_state(state);
}
