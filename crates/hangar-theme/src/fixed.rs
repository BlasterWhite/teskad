//! The few colours that are not the theme's to choose, so that no crate but
//! this one writes a colour of its own.

use iced::Color;

/// The ground a QR code is drawn on: white whatever the theme, since a code on
/// a dark ground does not scan.
pub const QR_GROUND: Color = Color::WHITE;

/// Windows' close button under the pointer: the system's red, which every
/// Windows window shows, and which a theme does not recolour.
pub const WINDOWS_CLOSE: Color = Color::from_rgb(196.0 / 255.0, 43.0 / 255.0, 28.0 / 255.0);

/// Windows' close button while pressed.
pub const WINDOWS_CLOSE_PRESSED: Color = Color {
    a: 0.8,
    ..WINDOWS_CLOSE
};

/// The glyph on Windows' close button, white on the red.
pub const ON_WINDOWS_CLOSE: Color = Color::WHITE;
