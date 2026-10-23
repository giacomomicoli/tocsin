fn main() {
    // On Windows, embed the exe icon and version info. winresource fills
    // FileVersion / ProductVersion from CARGO_PKG_VERSION; the rest is set here.
    #[cfg(windows)]
    {
        const ICON: &str = "assets/icon.ico";
        println!("cargo:rerun-if-changed={ICON}");
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "Tocsin");
        res.set("FileDescription", "Tocsin — WoW addon update checker");
        if std::path::Path::new(ICON).exists() {
            res.set_icon(ICON);
        } else if std::env::var("PROFILE").is_ok_and(|p| p == "release") {
            // A shipped exe without its icon is a packaging bug: stop the build.
            panic!("{ICON} is missing: a release build must embed the application icon");
        } else {
            println!("cargo:warning={ICON} is missing: the exe will have no icon");
        }
        if let Err(e) = res.compile() {
            println!("cargo:warning=winresource failed: {e}");
        }
    }
}
