use dioxus::document;
use dioxus::prelude::*;

use metis_component_library::{
    Input, Select, SelectGroup, SelectGroupLabel, SelectItemIndicator, SelectList, SelectOption,
    SelectTrigger, SelectValue,
};

const APP_CSS: Asset = asset!("/assets/app.scss");
const INPUT_CSS: Asset = asset!("./components/input/style.scss");
const SELECT_CSS: Asset = asset!("./components/select.scss");

#[derive(Clone, Copy, PartialEq)]
enum Theme {
    Light,
    Dark,
}

impl Theme {
    const fn label(self) -> &'static str {
        match self {
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }

    const fn attribute(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    const fn toggle(self) -> Self {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Cluster {
    Atlas,
    Nova,
    Quasar,
    Nimbus,
}

impl Cluster {
    const fn label(self) -> &'static str {
        match self {
            Cluster::Atlas => "Atlas",
            Cluster::Nova => "Nova",
            Cluster::Quasar => "Quasar",
            Cluster::Nimbus => "Nimbus",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum StreamMode {
    Sync,
    Async,
}

impl StreamMode {
    const fn label(self) -> &'static str {
        match self {
            StreamMode::Sync => "Sync",
            StreamMode::Async => "Async",
        }
    }
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut theme = use_signal(|| Theme::Light);
    let mut cluster = use_signal(|| Cluster::Atlas);
    let mut alias = use_signal(|| "delta-shard".to_string());
    let mut inbox = use_signal(|| "ops@metis.ai".to_string());

    use_effect(move || {
        let attribute = theme.read().attribute();
        document::eval(&format!(
            "document.documentElement.setAttribute('color-theme', {attribute:?});"
        ));
    });

    let theme_value = *theme.read();
    let alias_value = alias.read().clone();
    let inbox_value = inbox.read().clone();
    let active_cluster = *cluster.read();

    rsx! {
        document::Title { "Metis Component Library" }
        document::Stylesheet { href: APP_CSS }
        document::Stylesheet { href: INPUT_CSS }
        document::Stylesheet { href: SELECT_CSS }
        div { class: "demo-shell",
            header { class: "hero",
                div { class: "hero-copy",
                    p { class: "eyebrow", "Metis Component Library" }
                    h1 { "Component Studio" }
                    p { class: "lede",
                        "Explore Select and Input variations with a live theme toggle and ambient styling."
                    }
                }
                div { class: "hero-controls",
                    div { class: "theme-card",
                        span { class: "meta-label", "Theme" }
                        strong { class: "theme-value", "{theme_value.label()}" }
                        button {
                            class: "theme-toggle",
                            onclick: move |_| {
                                let next = theme.read().toggle();
                                theme.set(next);
                            },
                            "Toggle theme"
                        }
                    }
                    div { class: "status-card",
                        span { class: "meta-label", "Active cluster" }
                        strong { class: "cluster-value", "{active_cluster.label()}" }
                        p { class: "meta-detail", "Mirrors the Select state below." }
                    }
                }
            }
            main { class: "demo-grid",
                section { class: "panel",
                    div { class: "panel-header",
                        h2 { "Inputs" }
                        p { "Flexible text fields with focus rings and disabled states." }
                    }
                    div { class: "field-stack",
                        div { class: "field",
                            label { "Cluster alias" }
                            Input {
                                value: "{alias_value}",
                                placeholder: "e.g. delta-shard",
                                oninput: move |event: FormEvent| {
                                    alias.set(event.value());
                                }
                            }
                            span { class: "field-note", "Live value: {alias_value}" }
                        }
                        div { class: "field",
                            label { "Alert inbox" }
                            Input {
                                r#type: "email",
                                value: "{inbox_value}",
                                placeholder: "ops@metis.ai",
                                oninput: move |event: FormEvent| {
                                    inbox.set(event.value());
                                }
                            }
                            span { class: "field-note", "Notifications route here." }
                        }
                        div { class: "field",
                            label { "Frozen placeholder" }
                            Input {
                                placeholder: "Readonly in dark sectors",
                                disabled: true
                            }
                            span { class: "field-note", "Disabled appearance respects theme." }
                        }
                    }
                }
                section { class: "panel",
                    div { class: "panel-header",
                        h2 { "Selects" }
                        p { "Grouped options, indicators, and disabled states." }
                    }
                    div { class: "field-stack",
                        div { class: "field",
                            label { "Cluster switch" }
                            Select::<Cluster> {
                                default_value: Some(Cluster::Atlas),
                                on_value_change: move |value: Option<Cluster>| {
                                    cluster.set(value.unwrap_or(Cluster::Atlas));
                                },
                                placeholder: "Select cluster",
                                SelectTrigger { aria_label: "Cluster", SelectValue {} }
                                SelectList { aria_label: "Cluster options",
                                    SelectGroup {
                                        SelectGroupLabel { "Primary" }
                                        SelectOption::<Cluster> {
                                            index: 0usize,
                                            value: Cluster::Atlas,
                                            text_value: Some(Cluster::Atlas.label().to_string()),
                                            "{Cluster::Atlas.label()}"
                                            SelectItemIndicator {}
                                        }
                                        SelectOption::<Cluster> {
                                            index: 1usize,
                                            value: Cluster::Nova,
                                            text_value: Some(Cluster::Nova.label().to_string()),
                                            "{Cluster::Nova.label()}"
                                            SelectItemIndicator {}
                                        }
                                    }
                                    SelectGroup {
                                        SelectGroupLabel { "Experimental" }
                                        SelectOption::<Cluster> {
                                            index: 2usize,
                                            value: Cluster::Quasar,
                                            text_value: Some(Cluster::Quasar.label().to_string()),
                                            "{Cluster::Quasar.label()}"
                                            SelectItemIndicator {}
                                        }
                                        SelectOption::<Cluster> {
                                            index: 3usize,
                                            value: Cluster::Nimbus,
                                            text_value: Some(Cluster::Nimbus.label().to_string()),
                                            disabled: true,
                                            "{Cluster::Nimbus.label()}"
                                            SelectItemIndicator {}
                                        }
                                    }
                                }
                            }
                            span { class: "field-note", "Nimbus is disabled for preview." }
                        }
                        div { class: "field",
                            label { "Streaming mode" }
                            Select::<StreamMode> {
                                default_value: Some(StreamMode::Sync),
                                disabled: true,
                                placeholder: "Select mode",
                                SelectTrigger { aria_label: "Streaming mode", SelectValue {} }
                                SelectList { aria_label: "Streaming mode options",
                                    SelectGroup {
                                        SelectGroupLabel { "Pipeline" }
                                        SelectOption::<StreamMode> {
                                            index: 0usize,
                                            value: StreamMode::Sync,
                                            text_value: Some(StreamMode::Sync.label().to_string()),
                                            "{StreamMode::Sync.label()}"
                                            SelectItemIndicator {}
                                        }
                                        SelectOption::<StreamMode> {
                                            index: 1usize,
                                            value: StreamMode::Async,
                                            text_value: Some(StreamMode::Async.label().to_string()),
                                            "{StreamMode::Async.label()}"
                                            SelectItemIndicator {}
                                        }
                                    }
                                }
                            }
                            span { class: "field-note", "Disabled select for contrast." }
                        }
                    }
                }
            }
        }
    }
}
