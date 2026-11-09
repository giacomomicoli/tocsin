//! Tocsin — native desktop GUI (iced).

// Hide the console window on Windows release builds; keep it in debug for logs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use iced::{window, Size};

mod app;
mod theme;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tocsin=info,tocsin_core=info".into()),
        )
        .init();

    // iced parses every system font on the first text layout, after the
    // window and the GPU are set up: hundreds of milliseconds before the
    // first frame (much more on a cold disk cache). Start that parse now,
    // on its own thread, so it overlaps the window and adapter setup.
    // Safe to race: `font_system` is a `OnceLock`, so whichever caller
    // comes second waits for the first and gets the same font system.
    std::thread::spawn(|| {
        let _ = iced::advanced::graphics::text::font_system();
    });

    // Before the settings are read and before the window (and its event
    // loop) exist: a few renames at most, and only on the first run after
    // an upgrade from an early build.
    tocsin_core::paths::migrate_legacy();

    // `tocsin --settings` opens on the settings page (handy for support).
    let start_on_settings = std::env::args().any(|a| a == "--settings");
    let boot = move || app::App::boot(start_on_settings);

    let mut app = iced::application(boot, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .default_font(theme::UI)
        .window(window::Settings {
            size: Size::new(theme::space::WINDOW.0, theme::space::WINDOW.1),
            min_size: Some(Size::new(theme::space::WINDOW_MIN.0, theme::space::WINDOW_MIN.1)),
            icon: window_icon(),
            platform_specific: platform_specific(),
            ..Default::default()
        })
        .antialiasing(true);
    // The fonts are compiled in: no file is read at startup.
    for bytes in theme::FONT_BYTES {
        app = app.font(bytes);
    }
    app.run()
}

/// Wayland compositors ignore the icon a window sets: they look for a
/// `.desktop` file named after the window's application id and take the icon
/// and the name from there (KDE Plasma and GNOME both do; without it the
/// taskbar shows a generic icon). The id is the basename of
/// `assets/linux/tocsin.desktop`, which the release tarball ships; on X11
/// the same value becomes the window class.
#[cfg(target_os = "linux")]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        application_id: "tocsin".into(),
        ..Default::default()
    }
}

#[cfg(not(target_os = "linux"))]
fn platform_specific() -> window::settings::PlatformSpecific {
    Default::default()
}

/// The emblem as the window icon. Decoded here because iced's own helper
/// needs its every-codec `image` feature; PNG is the only one compiled in.
fn window_icon() -> Option<window::icon::Icon> {
    let rgba = image::load_from_memory_with_format(theme::EMBLEM_PNG, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    window::icon::from_rgba(rgba.into_raw(), width, height).ok()
}
