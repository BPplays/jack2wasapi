use dioxus::prelude::*;
use dioxus_desktop::{self, Config};
use dioxus_desktop::use_window;

pub fn run(minimized: bool) {
    // Keep the app alive when the window is hidden.
    // Tray behavior is enabled through the desktop config + tray hooks.
    dioxus::LaunchBuilder::desktop().launch(app);
}

const NORMAL_ITEMS: &[&str] = &["Red", "Green", "Blue"];
const SEARCH_ITEMS: &[&str] = &[
    "Apple",
    "Apricot",
    "Banana",
    "Blackberry",
    "Blueberry",
    "Cherry",
    "Grape",
    "Orange",
    "Peach",
    "Pear",
    "Pineapple",
];

#[component]
fn app() -> Element {
    let mut buffer_size_selected = use_signal(|| NORMAL_ITEMS[0].to_string());
    let mut search_text = use_signal(String::new);
    let mut fuzzy_selected = use_signal(|| SEARCH_ITEMS[0].to_string());

    let window = use_window();

    let needle = search_text.read().to_lowercase();
    let filtered: Vec<&str> = SEARCH_ITEMS
        .iter()
        .copied()
        .filter(|item| item.to_lowercase().contains(&needle))
        .collect();

    rsx! {
        div { style: "font-family: inter,Inter,sans-serif; padding: 20px; max-width: 420px;",
            h1 { "Basic Dioxus GUI" }

            div { style: "margin-bottom: 16px;",
                div { "Normal dropdown" }
                select {
                    value: "{buffer_size_selected}",
                    oninput: move |event| {
                        let value = event.value();
                        buffer_size_selected.set(value.clone());

                        // run event here
                        info!("buffer size changed -> {value}");
                    },
                    for item in NORMAL_ITEMS {
                        option {
                            value: "{item}",
                            "{item}"
                        }
                    }
                }
            }

            div { style: "margin-bottom: 16px;",
                div { "Searchable dropdown" }
                input {
                    r#type: "text",
                    placeholder: "Type to filter...",
                    value: "{search_text}",
                    oninput: move |event| {
                        let value = event.value();
                        search_text.set(value.clone());
                    },
                }

                div { style: "border: 1px solid #ccc; margin-top: 8px; max-height: 180px; overflow: auto;",
                    for item in filtered.iter().copied() {
                        button {
                            style: "display: block; width: 100%; text-align: left; padding: 8px; border: 0; background: transparent;",
                            onmousedown: move |_| {
                                fuzzy_selected.set(item.to_string());
                                search_text.set(item.to_string());

                                let value = item.to_string();
                                info!("audio device changed -> {value}");
                            },
                            "{item}"
                        }
                    }
                    if filtered.is_empty() {
                        div { style: "padding: 8px; color: #888;", "No matches" }
                    }
                }

                div { style: "margin-top: 8px;",
                    "Selected: {fuzzy_selected}"
                }
            }

            button {
                onmousedown: move |_| {
                    // Hide the window instead of exiting.
                    window.set_visible(false);
                },
                "Minimize to tray"
            }
        }
    }
}
