use dioxus::prelude::*;

fn build_toggle_class(disabled: bool) -> String {
    let mut class_name = String::from("metis-toggle");

    if disabled {
        class_name.push_str(" metis-toggle--disabled");
    }

    class_name
}

#[component]
pub fn ToggleSwitch(
    #[props(optional)] checked: Option<bool>,
    #[props(optional)] disabled: Option<bool>,
    onchange: Option<EventHandler<FormEvent>>,
    #[props(extends = GlobalAttributes)]
    #[props(extends = input)]
    attributes: Vec<Attribute>,
) -> Element {
    let checked = checked.unwrap_or(false);
    let disabled = disabled.unwrap_or(false);
    let class_name = build_toggle_class(disabled);

    rsx! {
        document::Link { rel: "stylesheet", href: asset!("./style.scss") }
        label {
            class: "{class_name}",
            input {
                class: "metis-toggle__input",
                r#type: "checkbox",
                role: "switch",
                aria_checked: checked,
                checked: checked,
                disabled: disabled,
                onchange: move |e| _ = onchange.map(|callback| callback(e)),
                ..attributes,
            }
            span {
                class: "metis-toggle__track",
                span { class: "metis-toggle__thumb" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_toggle_class_includes_disabled_state() {
        let class_name = build_toggle_class(true);

        assert!(class_name.contains("metis-toggle"));
        assert!(class_name.contains("metis-toggle--disabled"));
    }

    #[test]
    fn build_toggle_class_skips_disabled_when_false() {
        let class_name = build_toggle_class(false);

        assert!(class_name.contains("metis-toggle"));
        assert!(!class_name.contains("metis-toggle--disabled"));
    }
}
