//! The settings the manager exposes, with the same normalization as the tool.

use serde::Serialize;

use crate::config::Config;

#[derive(Clone, Copy, Serialize)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Copy, Serialize)]
pub struct Spec {
    pub key: &'static str,
    pub title: &'static str,
    pub kind: &'static str, // "choice" | "int"
    pub default: &'static str,
    pub choices: &'static [Choice],
    pub minimum: i64,
    pub maximum: i64,
    pub special_zero: &'static str,
}

#[derive(Clone, Copy, Serialize)]
pub struct Group {
    pub name: &'static str,
    pub title: &'static str,
    pub keys: &'static [&'static str],
}

const LAUNCHER_CHOICES: &[Choice] = &[
    Choice {
        value: "ask",
        label: "Ask every time",
    },
    Choice {
        value: "saved",
        label: "Use the remembered choice",
    },
];
const CANCEL_CHOICES: &[Choice] = &[
    Choice {
        value: "abort",
        label: "Stop the launch",
    },
    Choice {
        value: "game",
        label: "Run the plain game",
    },
];
const FALLBACK_CHOICES: &[Choice] = &[
    Choice {
        value: "1",
        label: "Run the plain game",
    },
    Choice {
        value: "0",
        label: "Do nothing",
    },
];
const WAIT_CHOICES: &[Choice] = &[
    Choice {
        value: "1",
        label: "Yes - wait for the game (recommended)",
    },
    Choice {
        value: "0",
        label: "No - start right away",
    },
];

pub const SPECS: &[Spec] = &[
    Spec {
        key: "LAUNCHER_GUI",
        title: "Ask at launch",
        kind: "choice",
        default: "ask",
        choices: LAUNCHER_CHOICES,
        minimum: 0,
        maximum: 0,
        special_zero: "",
    },
    Spec {
        key: "GUI_CONFIRM_DELAY",
        title: "Countdown before launch",
        kind: "int",
        default: "3",
        choices: &[],
        minimum: 0,
        maximum: 60,
        special_zero: "no card",
    },
    Spec {
        key: "GUI_CANCEL",
        title: "When cancelled",
        kind: "choice",
        default: "abort",
        choices: CANCEL_CHOICES,
        minimum: 0,
        maximum: 0,
        special_zero: "",
    },
    Spec {
        key: "FALLBACK_TO_GAME",
        title: "When nothing is chosen",
        kind: "choice",
        default: "1",
        choices: FALLBACK_CHOICES,
        minimum: 0,
        maximum: 0,
        special_zero: "",
    },
    Spec {
        key: "EXTRA_TIMEOUT",
        title: "Wait for a program to start",
        kind: "int",
        default: "120",
        choices: &[],
        minimum: 0,
        maximum: 3600,
        special_zero: "",
    },
    Spec {
        key: "EXTRA_DELAY",
        title: "Gap between starts",
        kind: "int",
        default: "5",
        choices: &[],
        minimum: 0,
        maximum: 600,
        special_zero: "",
    },
    Spec {
        key: "WAIT_FOR_GAME",
        title: "Wait for the game",
        kind: "choice",
        default: "1",
        choices: WAIT_CHOICES,
        minimum: 0,
        maximum: 0,
        special_zero: "",
    },
    Spec {
        key: "WAIT_FOR_GAME_TIMEOUT",
        title: "Game wait limit",
        kind: "int",
        default: "600",
        choices: &[],
        minimum: 0,
        maximum: 86400,
        special_zero: "no limit",
    },
    Spec {
        key: "UI_LANGUAGE",
        title: "Language",
        kind: "choice",
        default: "",
        choices: &[],
        minimum: 0,
        maximum: 0,
        special_zero: "",
    },
];

pub const GROUPS: &[Group] = &[
    Group {
        name: "start",
        title: "Game start",
        keys: &[
            "LAUNCHER_GUI",
            "GUI_CONFIRM_DELAY",
            "GUI_CANCEL",
            "FALLBACK_TO_GAME",
        ],
    },
    Group {
        name: "sidecars",
        title: "Sidecars",
        keys: &[
            "EXTRA_TIMEOUT",
            "EXTRA_DELAY",
            "WAIT_FOR_GAME",
            "WAIT_FOR_GAME_TIMEOUT",
        ],
    },
    Group {
        name: "advanced",
        title: "Advanced",
        keys: &["UI_LANGUAGE"],
    },
];

pub fn spec(key: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.key == key)
}

pub fn normalize(key: &str, raw: &str) -> String {
    let Some(spec) = spec(key) else {
        return raw.to_owned();
    };
    if spec.kind == "int" {
        return if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
            raw.to_owned()
        } else {
            spec.default.to_owned()
        };
    }
    match key {
        "LAUNCHER_GUI" => {
            if matches!(raw, "never" | "saved" | "no" | "off") {
                "saved".to_owned()
            } else {
                "ask".to_owned()
            }
        }
        "GUI_CANCEL" => {
            if raw == "game" {
                "game".to_owned()
            } else {
                "abort".to_owned()
            }
        }
        "UI_LANGUAGE" => {
            let valid = raw.is_empty()
                || raw == "auto"
                || (raw.len() <= 8 && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
            if valid {
                raw.to_owned()
            } else {
                String::new()
            }
        }
        _ => {
            if raw == "0" {
                "0".to_owned()
            } else {
                "1".to_owned()
            }
        }
    }
}

pub fn current(config: &Config) -> Vec<(&'static str, String)> {
    SPECS
        .iter()
        .map(|spec| (spec.key, normalize(spec.key, &config.get(spec.key, ""))))
        .collect()
}

pub fn set(config: &mut Config, key: &str, value: &str) -> Result<String, String> {
    if spec(key).is_none() {
        return Err(format!("unknown setting: {key}"));
    }
    let normalized = normalize(key, value);
    config.set(key, &normalized)?;
    Ok(normalized)
}

pub fn restore(config: &mut Config, group: &str) -> Result<(), String> {
    let Some(group) = GROUPS.iter().find(|g| g.name == group) else {
        return Err(format!("unknown group: {group}"));
    };
    for key in group.keys {
        if let Some(spec) = spec(key) {
            config.set(key, spec.default)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_special_keys() {
        assert_eq!(normalize("LAUNCHER_GUI", "never"), "saved");
        assert_eq!(normalize("GUI_CANCEL", "game"), "game");
        assert_eq!(normalize("GUI_CANCEL", "wat"), "abort");
        assert_eq!(normalize("WAIT_FOR_GAME", "0"), "0");
        assert_eq!(normalize("WAIT_FOR_GAME", "yes"), "1");
        assert_eq!(normalize("EXTRA_DELAY", "abc"), "5");
        assert_eq!(normalize("EXTRA_DELAY", "42"), "42");
    }

    #[test]
    fn the_dialog_toolkit_key_is_not_offered_anymore() {
        // `GUI` only picked an external dialog toolkit; this app is the UI, so
        // the manager must not offer a setting that cannot do anything.
        assert!(spec("GUI").is_none());
        assert!(GROUPS.iter().all(|group| !group.keys.contains(&"GUI")));
        // The settings that do something stay.
        for key in [
            "LAUNCHER_GUI",
            "GUI_CONFIRM_DELAY",
            "GUI_CANCEL",
            "FALLBACK_TO_GAME",
            "EXTRA_TIMEOUT",
            "EXTRA_DELAY",
            "WAIT_FOR_GAME",
            "WAIT_FOR_GAME_TIMEOUT",
        ] {
            assert!(spec(key).is_some(), "{key} disappeared");
        }
    }
}
