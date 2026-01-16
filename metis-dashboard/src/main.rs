#[cfg(target_arch = "wasm32")]
mod web_app {
    use dioxus::prelude::*;

    pub fn launch() {
        dioxus_web::launch(App);
    }

    fn App(cx: Scope) -> Element {
        cx.render(rsx!(
            div { class: "placeholder", "Metis dashboard placeholder" }
        ))
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    web_app::launch();

    #[cfg(not(target_arch = "wasm32"))]
    println!("metis-dashboard targets wasm32; build with wasm32-unknown-unknown");
}
