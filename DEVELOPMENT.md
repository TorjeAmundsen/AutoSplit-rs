# Development

## Building

Needs Rust 1.92 or newer. Linux builds also need CMake, plus NASM on x64, to build libjpeg-turbo. The window loads its Linux libraries (X11 or Wayland, xkbcommon, fontconfig) when it opens, so the build doesn't need them.

`cargo run -p autosplit --example screenshots -- <folder>` draws every window to PNG files without a display, to check the layout.

```sh
cargo build --release
cargo test --workspace
```

The tests need no internet, Python or capture devices. The LiveSplit One tests listen on a local port, and the session tests register global hotkeys for a moment.

## Release builds

`build.ps1` (PowerShell 7), or `build.sh` on Linux and macOS without PowerShell, builds release zips into `build/`, named `AutoSplit-v{version}-{target}.zip`. It strips symbols unless you pass `--debug`. Windows zips have `AutoSplit.exe` and `AutoSplit.com`, renamed from Cargo's `autosplit.exe` and `autosplit-console.exe` (see [Command line](MANUAL.md#command-line)). Linux zips also have `icon.png` and `install-desktop-entry.sh`, which adds the program to the desktop's app menu. Every zip has the `presets` folder, apart from presets marked as a work in progress.

The macOS binary is bare, not an `.app`, so `crates/autosplit-cli/build.rs` links an Info.plist into it, with the bundle id and the camera usage text, and `.cargo/config.toml` sets `MACOSX_DEPLOYMENT_TARGET` to 11.0. On Windows, `build.rs` embeds the icon instead. Shell scripts keep LF line endings through `.gitattributes`, since a CR after `#!` breaks them.

```sh
./build.ps1                  # pick a target from a menu
./build.ps1 linux-arm64      # one or more targets
./build.ps1 --all            # every target this machine can build
./build.ps1 --debug ...      # keep debug symbols
./build.sh ...               # the same, with bash
```

Targets are `windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` and `macos-universal`, one binary for Apple Silicon and Intel Macs. A target for this machine's OS builds natively, and the others cross-compile:

| Building on | Linux targets | Windows targets | macOS |
| --- | --- | --- | --- |
| Windows | [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), needs [Zig](https://ziglang.org/download) or `pip install ziglang` | native | not possible |
| Linux | native, or cargo-zigbuild for the other architecture | [cargo-xwin](https://github.com/rust-cross/cargo-xwin), needs clang and lld | not possible |
| macOS | cargo-zigbuild | cargo-xwin | native |

The script installs the Rust targets and the cargo tools it needs. macOS builds need a Mac, because Apple's SDK may only be used there. Cross-compiled Linux binaries need glibc 2.35 or newer (Ubuntu 22.04). Windows ARM64 builds on Windows need the MSVC ARM64 build tools from the Visual Studio installer.

## CI

- `.github/workflows/validate.yml` runs on pull requests, and by hand from the Actions tab. Pushes to `main` don't run it while the repo is private. It checks formatting and clippy and draws every window without a display. Then it runs the tests and a release build for every target, natively on Windows, Linux and macOS, x64 and ARM64. The x64 half of the macOS build is tested under Rosetta 2.
- `.github/workflows/release.yml` runs on `v*` tags. It checks that the tag matches the version in `Cargo.toml`, runs the validate workflow, and creates a draft GitHub release with the zips. Tags with a `-` (like `v0.2.0-beta`) become prereleases. Linux release builds run on Ubuntu 22.04, so they work on 22.04 and newer.

## Layout

- `crates/autosplit-core`: everything that doesn't depend on the OS. `runner.rs` is the split state machine. It never blocks: the host calls `tick` with the latest frame when `next_wakeup` is due, and acts on the effects it returns. `tester.rs` is the Live Tester, which decides like the runner for one image, outside of a run. `preset.rs` reads and saves the split image presets and `mask.rs` applies their masks, like AutoMask.
- `crates/autosplit-capture`: video capture devices, one backend per OS, and the pixel format conversions.
- `crates/autosplit-input`: global hotkeys, sending key presses and AutoSplit's hotkey names. Hotkeys go through livesplit-hotkey on Windows and Linux. On macOS, `macos.rs` has its own listen-only event tap, since livesplit-hotkey's asks for Accessibility, which only pressing keys needs.
- `crates/autosplit-cli`: the `autosplit` program. `session.rs` drives the runner from capture (`device.rs`), hotkeys (`hotkeys.rs`), LiveSplit (`livesplit.rs`), LiveSplit One (`livesplit_one.rs`) and Molok (`molok.rs`). `run.rs` runs a session in the terminal and `gui/` in the window, laid out in `ui/*.slint`. `gui/tester.rs` is the Live Tester, and `gui/maker.rs` is Make Split Images, with its preset editing in `gui/maker/editor.rs` and `gui/preset_form.rs`. `platform.rs` has what differs between systems, like consoles on Windows and App Nap and the menu bar on macOS, and `file_clipboard.rs` puts savestates on the Linux clipboard in every format file managers paste. It uses the presets and masks in core (`preset.rs`, `mask.rs`, `filename.rs`). `tests/gui/` tests the window logic that doesn't need a window (`document.rs`, `messages.rs`, `settings_form.rs`, `placement.rs`, `maker_plan.rs`, `recorder.rs`), with the preset editing in `tests/gui/preset_form.rs`, and `tests/view.rs` what the session hands the window. `src/bin/autosplit-console.rs` becomes `AutoSplit.com` on Windows.
- `tests/fixtures`: split images, captures and profiles. `real/` holds split images made with AutoMask.
- `tests/golden`: expected values from AutoSplit and OpenCV.
- `tools/golden/generate.py`: regenerates the fixtures and golden values from AutoSplit 2.3.4. Instructions are at the top of the file. Only rerun it when you mean to change the baseline.
- `presets/` and `splits/`: the split image presets and the splits library. `tools/convert_presets.py` converted them from AutoMask. Release builds put the presets next to the binary, apart from those with a `WIP.txt` in their folder, which are still being made. The splits library stays out of them until it can be imported.
