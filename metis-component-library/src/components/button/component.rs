use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Ghost,
}

impl ButtonVariant {
    const fn class_name(self) -> &'static str {
        match self {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Ghost => "ghost",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
}

impl ButtonSize {
    const fn class_name(self) -> &'static str {
        match self {
            ButtonSize::Sm => "sm",
            ButtonSize::Md => "md",
            ButtonSize::Lg => "lg",
        }
    }
}

fn build_button_class(variant: ButtonVariant, size: ButtonSize, full_width: bool) -> String {
    let mut class_name = format!(
        "metis-button metis-button--{} metis-button--{}",
        variant.class_name(),
        size.class_name()
    );

    if full_width {
        class_name.push_str(" metis-button--full");
    }

    class_name
}

#[component]
pub fn Button(
    #[props(optional)] variant: Option<ButtonVariant>,
    #[props(optional)] size: Option<ButtonSize>,
    #[props(optional)] full_width: Option<bool>,
    #[props(optional)] disabled: Option<bool>,
    onclick: Option<EventHandler<MouseEvent>>,
    onfocus: Option<EventHandler<FocusEvent>>,
    onblur: Option<EventHandler<FocusEvent>>,
    onkeydown: Option<EventHandler<KeyboardEvent>>,
    onkeyup: Option<EventHandler<KeyboardEvent>>,
    #[props(extends = GlobalAttributes)]
    #[props(extends = button)]
    attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let variant = variant.unwrap_or_default();
    let size = size.unwrap_or_default();
    let full_width = full_width.unwrap_or(false);
    let disabled = disabled.unwrap_or(false);
    let class_name = build_button_class(variant, size, full_width);

    rsx! {
        document::Link { rel: "stylesheet", href: asset!("./style.scss") }
        button {
            class: "{class_name}",
            r#type: "button",
            disabled: disabled,
            onclick: move |e| _ = onclick.map(|callback| callback(e)),
            onfocus: move |e| _ = onfocus.map(|callback| callback(e)),
            onblur: move |e| _ = onblur.map(|callback| callback(e)),
            onkeydown: move |e| _ = onkeydown.map(|callback| callback(e)),
            onkeyup: move |e| _ = onkeyup.map(|callback| callback(e)),
            ..attributes,
            {children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_button_class_includes_variant_size_and_width() {
        let class_name = build_button_class(ButtonVariant::Secondary, ButtonSize::Lg, true);

        assert!(class_name.contains("metis-button"));
        assert!(class_name.contains("metis-button--secondary"));
        assert!(class_name.contains("metis-button--lg"));
        assert!(class_name.contains("metis-button--full"));
    }

    #[test]
    fn build_button_class_skips_full_width_when_false() {
        let class_name = build_button_class(ButtonVariant::Primary, ButtonSize::Sm, false);

        assert!(class_name.contains("metis-button--primary"));
        assert!(class_name.contains("metis-button--sm"));
        assert!(!class_name.contains("metis-button--full"));
    }
}
