//! State of the Settings tab: the Daemon's last copy of the config and the
//! rules for turning it into a theme. No I/O; changes go out as `config_set`
//! ops and come back as `config_reloaded`, so the file stays the one truth.

use std::collections::BTreeMap;

use serde_json::Value;

use super::theme::Theme;

/// What kind of control a setting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One of the built-in or custom themes.
    Theme,
    /// On or off.
    Switch,
}

/// One row of the Settings list.
#[derive(Debug, Clone, Copy)]
pub struct Item {
    pub label: &'static str,
    /// The dotted config key it edits.
    pub key: &'static str,
    pub kind: Kind,
    pub help: &'static str,
    /// What the key means when absent from the file.
    pub default: bool,
}

const fn switch(label: &'static str, key: &'static str, default: bool, help: &'static str) -> Item {
    Item {
        label,
        key,
        kind: Kind::Switch,
        help,
        default,
    }
}

/// Every setting, in the order the list shows them.
pub const ITEMS: [Item; 11] = [
    Item {
        label: "Theme",
        key: "theme",
        kind: Kind::Theme,
        help: "The colours of the TUI. Enter picks the next one.",
        default: false,
    },
    switch(
        "Nerd Font icons",
        "ui.nerd_font",
        false,
        "Icons on the tabs. Needs a Nerd Font in your terminal.",
    ),
    switch(
        "Reduced motion",
        "ui.reduce_motion",
        false,
        "No gliding, sliding, shaking or pulsing. Colour changes stay.",
    ),
    switch(
        "Notify: joined",
        "notifications.joined",
        true,
        "A desktop notification when you join a Network.",
    ),
    switch(
        "Notify: lost",
        "notifications.lost",
        true,
        "A desktop notification when a Network drops.",
    ),
    switch(
        "Notify: bad password",
        "notifications.wrong_password",
        true,
        "A desktop notification when a join fails on the password.",
    ),
    switch(
        "Notify: prompt",
        "notifications.secret_prompt",
        true,
        "A desktop notification when a password is asked for and no Nexus window is open.",
    ),
    switch(
        "Notify: login",
        "notifications.portal",
        true,
        "A desktop notification when a Portal wants a login, and again once you are online.",
    ),
    switch(
        "Notify: VPN drop",
        "notifications.vpn_dropped",
        true,
        "A desktop notification when a VPN goes down without being asked to.",
    ),
    switch(
        "Notify: untrusted",
        "notifications.policy",
        true,
        "A desktop notification when joining an Untrusted Network brought up a VPN or marked it metered.",
    ),
    switch(
        "Untrusted: metered",
        "trust.metered",
        false,
        "Mark an Untrusted Network metered when you join it. (The VPN to bring up is `trust.auto_vpn` in config.toml.)",
    ),
];

/// The Daemon's config as the Settings tab last heard it.
#[derive(Debug, Default)]
pub struct SettingsTab {
    config: Option<Value>,
    /// Why the Daemon is ignoring `config.toml`, if it is.
    warning: Option<String>,
}

impl SettingsTab {
    /// Takes the Daemon's config. Returns whether anything changed.
    pub fn refresh(&mut self, config: Value, warning: Option<String>) -> bool {
        if self.config.as_ref() == Some(&config) && self.warning == warning {
            return false;
        }
        self.config = Some(config);
        self.warning = warning;
        true
    }

    /// Whether the Daemon has said anything yet.
    pub fn has_data(&self) -> bool {
        self.config.is_some()
    }

    pub fn config(&self) -> Option<&Value> {
        self.config.as_ref()
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    fn at(&self, key: &str) -> Option<&Value> {
        let mut v = self.config.as_ref()?;
        for part in key.split('.') {
            v = v.get(part)?;
        }
        Some(v)
    }

    /// A switch's value now.
    pub fn is_on(&self, item: &Item) -> bool {
        self.at(item.key)
            .and_then(Value::as_bool)
            .unwrap_or(item.default)
    }

    /// The `theme` setting as written.
    pub fn theme_name(&self) -> &str {
        self.at("theme")
            .and_then(Value::as_str)
            .unwrap_or(Theme::MIDNIGHT_BLOOM.name)
    }

    /// Names of the custom themes, sorted.
    pub fn custom_names(&self) -> Vec<String> {
        self.at("themes")
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Every name `theme` may take: the built-ins, then the custom ones.
    pub fn theme_choices(&self) -> Vec<String> {
        let mut v: Vec<String> = Theme::BUILTIN.iter().map(|t| t.name.to_owned()).collect();
        v.extend(
            self.custom_names()
                .into_iter()
                .filter(|n| Theme::builtin(n).is_none() && self.custom_problem(n).is_none()),
        );
        v
    }

    /// What is wrong with the custom theme `name`, if anything.
    pub fn custom_problem(&self, name: &str) -> Option<String> {
        Theme::custom(name, &self.custom_tokens(name)?).err()
    }

    /// The theme after the one in force (the first when it is unknown).
    pub fn next_theme(&self) -> String {
        let choices = self.theme_choices();
        let now = self.theme_name();
        let i = choices.iter().position(|n| n == now).map_or(0, |i| i + 1);
        choices[i % choices.len()].clone()
    }

    /// The tokens of the custom theme `name`, if the config defines it.
    pub fn custom_tokens(&self, name: &str) -> Option<BTreeMap<String, String>> {
        let table = self.at("themes")?.get(name)?.as_object()?;
        Some(
            table
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map_or_else(|| v.to_string(), String::from),
                    )
                })
                .collect(),
        )
    }

    /// The theme to draw with, and why it is not the one asked for (an
    /// unknown name, or a custom theme with a mistake in it). A custom theme
    /// of the same name as a built-in one wins.
    pub fn resolve_theme(&self) -> (Theme, Option<String>) {
        let name = self.theme_name();
        if let Some(tokens) = self.custom_tokens(name) {
            return match Theme::custom(name, &tokens) {
                Ok(theme) => (theme, None),
                Err(problem) => (
                    Theme::MIDNIGHT_BLOOM,
                    Some(format!("{problem}. Using Midnight Bloom.")),
                ),
            };
        }
        match Theme::builtin(name) {
            Some(theme) => (theme, None),
            None => (
                Theme::MIDNIGHT_BLOOM,
                Some(format!(
                    "There is no theme called `{name}`. Using Midnight Bloom."
                )),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tab(config: Value) -> SettingsTab {
        let mut t = SettingsTab::default();
        t.refresh(config, None);
        t
    }

    #[test]
    fn a_missing_key_is_the_default() {
        let t = tab(json!({}));
        assert!(t.is_on(&ITEMS[3]));
        assert!(!t.is_on(&ITEMS[1]));
        assert_eq!(t.theme_name(), "midnight-bloom");
    }

    #[test]
    fn cycling_walks_built_ins_then_custom_and_wraps() {
        let t = tab(json!({"theme": "high-contrast", "themes": {"mine": {"accent": "#ff8800"}}}));
        assert_eq!(t.next_theme(), "mine");
        let t = tab(json!({"theme": "mine", "themes": {"mine": {}}}));
        assert_eq!(t.next_theme(), "midnight-bloom");
        let t = tab(json!({"theme": "gone"}));
        assert_eq!(t.next_theme(), "midnight-bloom");
    }

    #[test]
    fn cycling_skips_custom_themes_with_mistakes() {
        let t = tab(json!({"theme": "high-contrast", "themes": {
            "a-broken": {"accent": "teal"}, "b-fine": {}}}));
        assert_eq!(t.next_theme(), "b-fine");
        assert!(t.custom_problem("a-broken").is_some());
    }

    #[test]
    fn an_unknown_theme_falls_back_with_a_warning() {
        let (theme, note) = tab(json!({"theme": "gone"})).resolve_theme();
        assert_eq!(theme.name, "midnight-bloom");
        assert!(note.unwrap().contains("`gone`"));
    }

    #[test]
    fn an_invalid_custom_theme_falls_back_with_a_warning() {
        let t = tab(json!({"theme": "mine", "themes": {"mine": {"accent": "teal"}}}));
        let (theme, note) = t.resolve_theme();
        assert_eq!(theme.name, "midnight-bloom");
        assert!(note.unwrap().contains("accent = teal"));
    }

    #[test]
    fn a_custom_theme_resolves() {
        let t =
            tab(json!({"theme": "mine", "themes": {"mine": {"base": "nord", "ok": "#00ff00"}}}));
        let (theme, note) = t.resolve_theme();
        assert!(note.is_none());
        assert_eq!(theme.ok.rgb, (0, 255, 0));
        assert_eq!(theme.accent.rgb, Theme::NORD.accent.rgb);
    }
}
