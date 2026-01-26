use dioxus::prelude::*;

const STAGING_URL: &str = "http://metis-staging.monster-vibes.ts.net";

#[derive(Clone, Copy, PartialEq)]
enum ServerChoice {
    Staging,
    Custom,
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut server_choice = use_signal(|| ServerChoice::Staging);
    let mut custom_url = use_signal(String::new);
    let is_custom = matches!(*server_choice.read(), ServerChoice::Custom);
    let custom_url_value = custom_url.read().clone();
    let display_url = if is_custom {
        custom_url_value.clone()
    } else {
        STAGING_URL.to_string()
    };

    rsx! {
        document::Stylesheet { href: asset!("/assets/app.css") }
        div { class: "app",
            header { class: "top-bar",
                div { class: "server-selector",
                    label { "Server" }
                    select {
                        value: if is_custom { "custom" } else { "staging" },
                        onchange: move |event| {
                            let value = event.value();
                            server_choice.set(if value == "custom" {
                                ServerChoice::Custom
                            } else {
                                ServerChoice::Staging
                            });
                        },
                        option { value: "staging", "Staging" }
                        option { value: "custom", "Custom" }
                    }
                    if is_custom {
                        input {
                            class: "custom-input",
                            r#type: "text",
                            value: "{custom_url_value}",
                            placeholder: "https://your-metis.example",
                            oninput: move |event| {
                                custom_url.set(event.value());
                            }
                        }
                    }
                }
            }
            main { class: "content",
                div { class: "selected-url", "{display_url}" }
            }
        }
    }
}
