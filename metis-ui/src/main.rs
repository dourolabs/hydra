use dioxus::prelude::*;

use metis_component_library::{
    Select, SelectGroup, SelectGroupLabel, SelectItemIndicator, SelectList, SelectOption,
    SelectTrigger, SelectValue,
};

const STAGING_URL: &str = "http://metis-staging.monster-vibes.ts.net";
const APP_CSS: Asset = asset!("/assets/app.css");

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
        document::Stylesheet { href: APP_CSS }
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
