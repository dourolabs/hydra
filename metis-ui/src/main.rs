use dioxus::prelude::*;

use crate::components::select::{
    Select, SelectGroup, SelectGroupLabel, SelectItemIndicator, SelectList, SelectOption,
    SelectTrigger, SelectValue,
};

mod components;

const STAGING_URL: &str = "http://metis-staging.monster-vibes.ts.net";
const APP_STYLES: &str = r#"
@import url("https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;600&display=swap");

:root {
    --ink: #1f2a2e;
    --panel: rgba(255, 255, 255, 0.7);
    --panel-border: rgba(31, 42, 46, 0.15);
    --focus: #2b7a78;
}

* {
    box-sizing: border-box;
}

body {
    margin: 0;
    color: var(--ink);
    font-family: "Space Grotesk", "Avenir Next", "Segoe UI", sans-serif;
    background: linear-gradient(135deg, #f8f2e4 0%, #dceeff 100%);
}

.app {
    min-height: 100vh;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 24px;
}

.top-bar {
    display: flex;
    justify-content: flex-end;
}

.server-selector {
    display: inline-flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    border-radius: 999px;
    background: var(--panel);
    border: 1px solid var(--panel-border);
    backdrop-filter: blur(8px);
}

.server-selector label {
    font-weight: 600;
    font-size: 0.95rem;
}

.server-selector input {
    font-family: inherit;
    font-size: 0.95rem;
    padding: 8px 12px;
    border-radius: 999px;
    border: 1px solid var(--panel-border);
    background: white;
    color: var(--ink);
}

.server-selector input:focus {
    outline: 2px solid color-mix(in srgb, var(--focus) 70%, white 30%);
    outline-offset: 2px;
}

.custom-input {
    min-width: min(50vw, 320px);
}

.content {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 12px;
}

.selected-url {
    font-size: clamp(1.25rem, 3vw, 2.25rem);
    font-weight: 600;
    text-align: center;
    word-break: break-word;
}

@media (max-width: 700px) {
    .server-selector {
        flex-wrap: wrap;
        justify-content: flex-end;
    }

    .custom-input {
        width: 100%;
    }
}
"#;

#[derive(Clone, Copy, PartialEq)]
enum ServerChoice {
    Staging,
    Custom,
}

impl ServerChoice {
    const fn label(self) -> &'static str {
        match self {
            ServerChoice::Staging => "Staging",
            ServerChoice::Custom => "Custom",
        }
    }
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
        style { "{APP_STYLES}" }
        div { class: "app",
            header { class: "top-bar",
                div { class: "server-selector",
                    label { "Server" }
                    Select::<ServerChoice> {
                        default_value: Some(ServerChoice::Staging),
                        on_value_change: move |value: Option<ServerChoice>| {
                            server_choice.set(value.unwrap_or(ServerChoice::Staging));
                        },
                        placeholder: "Select server",
                        SelectTrigger { aria_label: "Server", SelectValue {} }
                        SelectList { aria_label: "Server options",
                            SelectGroup {
                                SelectGroupLabel { "Environment" }
                                SelectOption::<ServerChoice> {
                                    index: 0usize,
                                    value: ServerChoice::Staging,
                                    text_value: Some(ServerChoice::Staging.label().to_string()),
                                    "{ServerChoice::Staging.label()}"
                                    SelectItemIndicator {}
                                }
                                SelectOption::<ServerChoice> {
                                    index: 1usize,
                                    value: ServerChoice::Custom,
                                    text_value: Some(ServerChoice::Custom.label().to_string()),
                                    "{ServerChoice::Custom.label()}"
                                    SelectItemIndicator {}
                                }
                            }
                        }
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
