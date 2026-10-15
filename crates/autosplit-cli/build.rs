fn main() {
    slint_build::compile("ui/main.slint").expect("the UI compiles");
    // The icon Explorer shows for AutoSplit.exe and AutoSplit.com. Does nothing outside Windows.
    embed_resource::compile("icon.rc", embed_resource::NONE).manifest_optional().expect("the icon compiles");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        embed_info_plist();
    }
}

/// The bundle id the Info.plist names on macOS, and the app id windows get on Linux, which the
/// .desktop file in TUTORIAL.md is named after.
const APP_ID: &str = "io.github.torjeamundsen.autosplit";

/// macOS ships a bare binary, not an .app, so its Info.plist goes into the binary itself. macOS
/// reads the camera reason from it, and stops a program without one that asks for the camera
/// itself, rather than through the app that started it, like Terminal.
fn embed_info_plist() {
    let version = env!("CARGO_PKG_VERSION");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{APP_ID}</string>
    <key>CFBundleName</key>
    <string>AutoSplit</string>
    <key>CFBundleExecutable</key>
    <string>autosplit</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSCameraUsageDescription</key>
    <string>AutoSplit compares frames from your capture device with your split images.</string>
</dict>
</plist>
"#
    );
    let path = std::path::Path::new(&std::env::var("OUT_DIR").expect("Cargo sets OUT_DIR")).join("Info.plist");
    std::fs::write(&path, plist).expect("Info.plist is written");
    println!("cargo:rustc-link-arg-bin=autosplit=-Wl,-sectcreate,__TEXT,__info_plist,{}", path.display());
}
