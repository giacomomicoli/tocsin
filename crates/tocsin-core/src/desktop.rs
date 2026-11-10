//! What the desktop around the app offers.
//!
//! On Windows and macOS the folder dialog is part of the system. On Linux it
//! is a separate service, the file chooser of `xdg-desktop-portal`, with
//! `zenity` as the dialog library's fallback. A bare window manager (sway,
//! i3, Hyprland without a portal backend) or a minimal install may have
//! neither, and then the dialog comes back empty at once, exactly as if the
//! user had closed it. This module tells the two apart, so *Add game folder*
//! can say why nothing opened.

/// Shown when no folder dialog can open on this desktop.
pub const NO_FOLDER_DIALOG: &str = "The folder window could not open: this desktop has no file chooser. \
     Install xdg-desktop-portal with the backend for your desktop (or zenity), then try again.";

/// Why the folder dialog gave nothing back: `None` when it could open (the
/// user closed it), a message for the user when the desktop has no dialog.
///
/// Call it only after the dialog returned nothing, from a Tokio runtime: it
/// asks the session bus, which can take a moment on a broken setup.
pub async fn folder_dialog_problem() -> Option<&'static str> {
    // Only Linux (and the BSDs, untested) lack a dialog of their own.
    #[cfg(target_os = "linux")]
    return missing_dialog(linux::portal_has_file_chooser().await, linux::on_path("zenity"));
    #[cfg(not(target_os = "linux"))]
    None
}

/// The decision, apart from the probes: one way to open a dialog is enough.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn missing_dialog(portal: bool, zenity: bool) -> Option<&'static str> {
    (!portal && !zenity).then_some(NO_FOLDER_DIALOG)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::time::Duration;

    use zbus::connection::Builder;

    /// A portal whose backend hangs must not hold the answer for the bus's
    /// default 25 seconds.
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Whether `xdg-desktop-portal` runs (or can be started) and one of its
    /// backends implements the file chooser. The portal exports the
    /// `FileChooser` interface only then, so reading its `version` property
    /// answers both. Nothing about the user is sent: the call stays on the
    /// session bus of this machine.
    pub(super) async fn portal_has_file_chooser() -> bool {
        let Ok(bus) = Builder::session().method_timeout(TIMEOUT).build().await else {
            return false;
        };
        bus.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.portal.FileChooser", "version"),
        )
        .await
        .is_ok()
    }

    /// Whether a program called `name` is in one of the `PATH` folders. Only
    /// its presence is checked; nothing is read or run.
    pub(super) fn on_path(name: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sway with no portal backend and no zenity: the click on *Add game
    /// folder* must be answered. Either one is enough to open a dialog, and
    /// then an empty answer is the user closing it.
    #[test]
    fn only_a_desktop_with_no_dialog_at_all_is_a_problem() {
        assert_eq!(missing_dialog(false, false), Some(NO_FOLDER_DIALOG));
        assert_eq!(missing_dialog(true, false), None);
        assert_eq!(missing_dialog(false, true), None);
        assert_eq!(missing_dialog(true, true), None);
    }
}
