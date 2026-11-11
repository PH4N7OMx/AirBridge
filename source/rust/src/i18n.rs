//! UTF-8 catalog shared by the receiver, tray and settings; no runtime files required.
use crate::settings::Settings;
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
pub const LANGUAGES: [(&str, &str); 10] = [
    ("en", "English"),
    ("ru", "Русский"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("pt", "Português"),
    ("it", "Italiano"),
    ("pl", "Polski"),
    ("uk", "Українська"),
    ("tr", "Türkçe"),
];
static LANGUAGE: AtomicUsize = AtomicUsize::new(0);
fn catalog() -> &'static HashMap<&'static str, Vec<&'static str>> {
    static CATALOG: OnceLock<HashMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        include_str!("../locales/ui.tsv")
            .lines()
            .skip(1)
            .map(|line| {
                let mut fields = line.split('\t');
                (fields.next().unwrap(), fields.collect())
            })
            .collect()
    })
}
pub fn tr(key: &str) -> &'static str {
    catalog()
        .get(key)
        .and_then(|values| values.get(index()).or_else(|| values.first()))
        .copied()
        .unwrap_or("…")
}
pub fn index() -> usize {
    LANGUAGE.load(Ordering::Relaxed)
}
pub fn set(index: usize) {
    LANGUAGE.store(
        if index < LANGUAGES.len() { index } else { 0 },
        Ordering::Relaxed,
    );
}
pub fn choose(saved: Option<&str>, windows: &str) -> usize {
    let tag = saved
        .unwrap_or(windows)
        .split(['-', '_'])
        .next()
        .unwrap_or("en");
    LANGUAGES
        .iter()
        .position(|(code, _)| code.eq_ignore_ascii_case(tag))
        .unwrap_or(0)
}
pub fn windows_language() -> String {
    use windows_sys::Win32::Globalization::*;
    let mut name = [0u16; 85];
    let n =
        unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, name.as_mut_ptr(), 85, 0) };
    if n > 1 {
        String::from_utf16_lossy(&name[..n as usize - 1])
    } else {
        "en".into()
    }
}
pub fn initialize(settings: &mut Settings) {
    let selected = choose(settings.text("language"), &windows_language());
    set(selected);
    let code = LANGUAGES[selected].0;
    if settings.text("language") != Some(code) {
        settings.set("language", code);
        if let Err(error) = settings.save() {
            log::warn!("cannot persist selected language: {error}");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_selection_respects_saved_language_and_falls_back_to_english() {
        assert_eq!(choose(None, "ru-RU"), 1);
        assert_eq!(choose(None, "pt-BR"), 5);
        assert_eq!(choose(None, "zh-CN"), 0);
        assert_eq!(choose(None, "xx"), 0);
        assert_eq!(choose(Some("de"), "ru-RU"), 2);
        assert_eq!(choose(Some("unknown"), "ru-RU"), 0);
    }
    #[test]
    fn every_message_has_all_ten_translations() {
        for (key, values) in catalog() {
            assert_eq!(values.len(), LANGUAGES.len(), "{key}");
            assert!(values.iter().all(|s| !s.trim().is_empty()), "{key}");
        }
        assert_eq!(
            catalog().len(),
            include_str!("../locales/ui.tsv").lines().count() - 1,
            "duplicate message keys"
        );
    }
    #[test]
    fn all_ui_message_keys_exist() {
        for source in [include_str!("ui.rs"), include_str!("ui_style.rs")] {
            for fragment in source.split("tr(\"").skip(1) {
                let key = fragment.split('"').next().unwrap();
                assert!(catalog().contains_key(key), "missing message {key}");
            }
        }
    }
}
