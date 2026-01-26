use dioxus::prelude::*;

use crate::components::select::{
    Select, SelectGroup, SelectGroupLabel, SelectItemIndicator, SelectList, SelectOption,
    SelectTrigger, SelectValue,
};

mod components;

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

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, ElementId, Mutation};
    use dioxus::html::{PlatformEventData, SerializedHtmlEventConverter, SerializedMouseData};
    use std::{any::Any, rc::Rc};

    fn aria_expanded_value(value: &AttributeValue) -> Option<bool> {
        match value {
            AttributeValue::Bool(value) => Some(*value),
            AttributeValue::Text(value) => match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn find_aria_expanded(edits: &[Mutation]) -> Option<(ElementId, bool)> {
        edits.iter().find_map(|edit| {
            if let Mutation::SetAttribute {
                name, id, value, ..
            } = edit
            {
                if *name == "aria-expanded" {
                    return aria_expanded_value(value).map(|value| (*id, value));
                }
            }
            None
        })
    }

    fn find_aria_expanded_for_id(edits: &[Mutation], target_id: ElementId) -> Option<bool> {
        edits.iter().find_map(|edit| {
            if let Mutation::SetAttribute {
                name, id, value, ..
            } = edit
            {
                if *name == "aria-expanded" && *id == target_id {
                    return aria_expanded_value(value);
                }
            }
            None
        })
    }

    #[test]
    fn server_select_opens_on_click() {
        set_event_converter(Box::new(SerializedHtmlEventConverter));

        let mut dom = VirtualDom::new(App);
        let edits = dom.rebuild_to_vec();
        let (trigger_id, is_open) =
            find_aria_expanded(&edits.edits).expect("select trigger aria-expanded not found");
        assert!(!is_open, "expected select to start closed");

        let event = Event::new(
            Rc::new(PlatformEventData::new(Box::<SerializedMouseData>::default())) as Rc<dyn Any>,
            true,
        );
        dom.runtime().handle_event("click", event, trigger_id);

        let edits = dom.render_immediate_to_vec();
        let is_open = find_aria_expanded_for_id(&edits.edits, trigger_id)
            .expect("select trigger aria-expanded not updated");
        assert!(is_open, "expected select to open after click");
    }
}
