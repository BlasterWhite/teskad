//! Which theme the window shows: the mode (Dark, Light, System) and the theme
//! each of the two lists has picked.
//!
//! `settings.theme_id` stays what every reader of the settings takes: the
//! theme the window shows. [`sync`] keeps it in line with the mode and the
//! picks whenever either changes, or the desktop does under System. A file
//! from before the mode has a theme and nothing else: it is read as the pick
//! of its own list, in the mode it belongs to, so the window stays as it was.

use std::sync::atomic::{AtomicBool, Ordering};

use hangar_theme::DEFAULT_ID;

use super::types::{Settings, ThemeMode};

/// The default theme of the light list.
pub const DEFAULT_LIGHT_ID: &str = "teskad-light";

/// What the desktop said last: light, or not. Dark until it has answered
/// (the default theme is dark, and most desktops that cannot say are).
static DESKTOP_IS_LIGHT: AtomicBool = AtomicBool::new(false);

/// Records the desktop's appearance; `true` when it changed.
pub fn set_desktop_light(light: bool) -> bool {
    DESKTOP_IS_LIGHT.swap(light, Ordering::Relaxed) != light
}

pub fn desktop_is_light() -> bool {
    DESKTOP_IS_LIGHT.load(Ordering::Relaxed)
}

fn is_light(id: &str) -> bool {
    hangar_theme::by_id(id).is_some_and(|theme| !theme.is_dark())
}

/// The mode: the one chosen, else the one of the theme the file has.
pub fn mode(settings: &Settings) -> ThemeMode {
    settings
        .theme_mode
        .unwrap_or(if is_light(&settings.theme_id) {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        })
}

/// The theme the dark list has picked.
pub fn dark_id(settings: &Settings) -> &str {
    match settings.dark_theme_id.as_deref() {
        Some(id) if !is_light(id) => id,
        _ if !is_light(&settings.theme_id) => &settings.theme_id,
        _ => DEFAULT_ID,
    }
}

/// The theme the light list has picked.
pub fn light_id(settings: &Settings) -> &str {
    match settings.light_theme_id.as_deref() {
        Some(id) if is_light(id) => id,
        _ if is_light(&settings.theme_id) => &settings.theme_id,
        _ => DEFAULT_LIGHT_ID,
    }
}

/// The theme the window should show, for a desktop that is light or not.
pub fn wanted_id(settings: &Settings, desktop_light: bool) -> &str {
    let light = match mode(settings) {
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
        ThemeMode::System => desktop_light,
    };
    if light {
        light_id(settings)
    } else {
        dark_id(settings)
    }
}

/// Writes the picks down, then brings `theme_id` to the theme the mode wants.
/// Only for a settings that has been given a mode or a pick: a file from
/// before is left as it is until then.
pub fn sync(settings: &mut Settings, desktop_light: bool) {
    let (dark, light) = (
        dark_id(settings).to_string(),
        light_id(settings).to_string(),
    );
    let wanted = wanted_id(settings, desktop_light).to_string();
    settings.dark_theme_id = Some(dark);
    settings.light_theme_id = Some(light);
    settings.theme_id = wanted;
}

/// Brings the window to the desktop's appearance, once the mode is System.
/// Changes nothing under another mode, nor in a file from before.
pub fn follow_desktop(settings: &mut Settings, desktop_light: bool) {
    if settings.theme_mode == Some(ThemeMode::System) {
        sync(settings, desktop_light);
    }
}

/// A theme picked in the grid: it goes to the list it belongs to, and the
/// mode follows it unless it is System (which uses both).
pub fn pick(settings: &mut Settings, id: &str) {
    let Some(theme) = hangar_theme::by_id(id) else {
        settings.theme_id = id.to_string();
        return;
    };
    let current = mode(settings);
    // Written down before `theme_id` moves: it may be the other list's pick.
    sync(settings, desktop_is_light());
    if theme.is_dark() {
        settings.dark_theme_id = Some(id.to_string());
    } else {
        settings.light_theme_id = Some(id.to_string());
    }
    settings.theme_mode = Some(match current {
        ThemeMode::System => ThemeMode::System,
        _ if theme.is_dark() => ThemeMode::Dark,
        _ => ThemeMode::Light,
    });
    sync(settings, desktop_is_light());
}

/// Another mode; the picks stay.
pub fn set_mode(settings: &mut Settings, chosen: ThemeMode) {
    sync(settings, desktop_is_light());
    settings.theme_mode = Some(chosen);
    sync(settings, desktop_is_light());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(theme: &str) -> Settings {
        Settings {
            theme_id: theme.into(),
            ..Settings::default()
        }
    }

    #[test]
    fn a_file_from_before_keeps_its_theme_and_the_mode_it_belongs_to() {
        let dark = settings("dracula");
        assert_eq!(mode(&dark), ThemeMode::Dark);
        assert_eq!(wanted_id(&dark, true), "dracula");
        let light = settings("teskad-light");
        assert_eq!(mode(&light), ThemeMode::Light);
        assert_eq!(wanted_id(&light, false), "teskad-light");
        // Its dark list has the default, its light list the theme it had.
        assert_eq!(dark_id(&light), DEFAULT_ID);
        assert_eq!(light_id(&light), "teskad-light");
        assert_eq!(light_id(&dark), DEFAULT_LIGHT_ID);
        // Nothing is written until the person touches the mode.
        let mut untouched = settings("dracula");
        follow_desktop(&mut untouched, true);
        assert_eq!(untouched, settings("dracula"));
    }

    #[test]
    fn system_follows_the_desktop_into_each_lists_own_pick() {
        let mut s = settings("dracula");
        pick(&mut s, "teskad-light");
        set_mode(&mut s, ThemeMode::System);
        assert_eq!(wanted_id(&s, false), "dracula");
        assert_eq!(wanted_id(&s, true), "teskad-light");
        follow_desktop(&mut s, true);
        assert_eq!(s.theme_id, "teskad-light");
        follow_desktop(&mut s, false);
        assert_eq!(s.theme_id, "dracula");
    }

    #[test]
    fn a_pick_goes_to_its_list_and_moves_the_mode_unless_it_is_system() {
        let mut s = settings("dracula");
        pick(&mut s, "teskad-light");
        assert_eq!(mode(&s), ThemeMode::Light);
        assert_eq!(s.theme_id, "teskad-light");
        assert_eq!(dark_id(&s), "dracula");
        set_mode(&mut s, ThemeMode::System);
        pick(&mut s, "nord");
        assert_eq!(mode(&s), ThemeMode::System);
        assert_eq!(dark_id(&s), "nord");
        assert_eq!(light_id(&s), "teskad-light");
    }

    #[test]
    fn a_dark_pick_does_not_lose_the_light_theme_of_a_file_from_before() {
        let mut s = settings("teskad-light");
        pick(&mut s, "nord");
        assert_eq!(mode(&s), ThemeMode::Dark);
        assert_eq!(s.theme_id, "nord");
        set_mode(&mut s, ThemeMode::Light);
        assert_eq!(s.theme_id, "teskad-light");
    }
}
