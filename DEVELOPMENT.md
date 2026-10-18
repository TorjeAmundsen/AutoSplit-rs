# Development

How to build AutoSplit-rs, where things are in the code, and how releases are made.

## Getting started

You need Rust 1.92 or newer. On Linux you also need CMake, and on x64 Linux NASM too, because libjpeg-turbo is built from source.

```sh
cargo build --release
cargo test --workspace
```

The tests don't need internet, Python or a capture device. Two kinds of test do touch the system for a moment: the LiveSplit One tests listen on a local port, and the session tests register global hotkeys.

To check a window's layout without opening it, draw every window to PNG files:

```sh
cargo run -p autosplit --example screenshots -- <folder>
```

On Linux, the window loads its system libraries (X11 or Wayland, xkbcommon, fontconfig) when it opens, so building doesn't need them installed.

## Project layout

The workspace has four crates:

| Crate | What's in it |
| --- | --- |
| `crates/autosplit-core` | Everything that doesn't depend on the OS: comparisons, split image file names, profiles, presets, masks and the split library |
| `crates/autosplit-capture` | Video capture devices, one backend per OS, and the pixel format conversions |
| `crates/autosplit-input` | Global hotkeys, sending key presses, and AutoSplit's hotkey names |
| `crates/autosplit-cli` | The `autosplit` program: the window, the terminal mode and the timer connections |

### Where to find things

| Part | Files |
| --- | --- |
| The split state machine | `autosplit-core/src/runner.rs` |
| The Live Tester's decisions | `autosplit-core/src/tester.rs` |
| Fade images: the similarity to a color and telling a fade from a cut | `autosplit-core/src/fade.rs` |
| Presets and their masks | `autosplit-core/src/preset.rs`, `mask.rs`, `filename.rs` |
| Writing the split library | `autosplit-core/src/library.rs` |
| A running session | `autosplit-cli/src/session.rs`, fed by `device.rs` (capture), `hotkeys.rs`, `livesplit.rs`, `livesplit_one.rs` and `molok.rs` |
| Terminal mode | `autosplit-cli/src/run.rs` |
| The window | `autosplit-cli/src/gui/`, laid out in `autosplit-cli/ui/*.slint` |
| Live Tester window | `gui/tester.rs` |
| Make Split Images | `gui/maker.rs`, with preset editing in `gui/maker/editor.rs` and `gui/preset_form.rs` |
| Split Library window | `gui/maker/library.rs`, and adding to the library in `gui/maker/library_entry.rs` |
| Differences between systems | `autosplit-cli/src/platform.rs` (consoles on Windows, App Nap and the menu bar on macOS) |
| Savestates on the Linux clipboard | `autosplit-cli/src/file_clipboard.rs` |
| `AutoSplit.com` on Windows | `autosplit-cli/src/bin/autosplit-console.rs` |

A few design notes:

- The runner never blocks. The host calls `tick` with the latest frame when `next_wakeup` is due, then acts on the effects it returns.
- The Live Tester decides the same way the runner does, for one image, outside of a run.
- Hotkeys go through livesplit-hotkey on Windows and Linux. On macOS, `autosplit-input/src/macos.rs` has its own listen-only event tap, because livesplit-hotkey's asks for Accessibility permission, which only sending key presses needs.

## Tests

- `autosplit-cli/tests/gui/` tests the window's logic without a window: documents, messages, settings, placement, Make Split Images' plans, the hotkey recorder, and preset editing (`preset_form.rs`).
- `autosplit-cli/tests/view.rs` tests what a session hands the window.
- `tests/fixtures` has split images, captures and profiles. `tests/fixtures/real/` has split images made with AutoMask.
- `tests/golden` has expected values from Toufool's AutoSplit and OpenCV.

`tools/golden/generate.py` regenerates the fixtures and golden values from AutoSplit 2.3.4. Its instructions are at the top of the file. Only run it when you mean to change the baseline.

## Presets and the split library

`presets/` holds the split image presets, and `splits/` the split library, one folder per game. `tools/convert_presets.py` converted both from AutoMask. Release builds put both folders next to the binary. A preset with a `WIP.txt` in its folder is still being made, and is left out of releases.

A library's `splits.toml` has a `game`, and a `[[split]]` table for each image. The tables take the same keys as a preset's splits, plus three of their own:

- `description`: what it splits on, shown in the Split Library.
- `base_image`: a screenshot the mask fits, shown beside it.
- `then`: `true` makes it the next image of the split above it. The Split Library shows them as one ready-made split and inserts them together, in order.

For example, a full fade scene entry that has the entrance text for the scene before it as a dummy:

```toml
game = "The Legend of Zelda: Ocarina of Time"

[[split]]
name = "HyruleFieldDummy"
mask = "HyruleFieldText.png"
dummy = true
description = "Enter Kakariko after Hyrule Field scene text dummy"
base_image = "base_HyruleFieldText.png"

[[split]]
name = "EnterKakariko"
mask = "FullFade.png"
threshold = 0.97
base_image = "base_FullFade.png"
then = true
```

### Editing the library

Debug builds (`cargo run`) can write the library, so you don't have to write `splits.toml` by hand. Release builds don't have these buttons.

1. Edit a preset of the game in Make Split Images, and make the splits there.
2. Select the first split and click **Add to Library...**. For a ready-made split of several images, raise **Images** to take the splits after it too.
3. Write a description and save.

This copies the masks and savestates into the game's library folder, which is made if the game has none yet. It writes each base image at 64x48 from the split's grab or screenshot, or else from the live capture. To use another screenshot instead, click **Picture...** beside that image. Each image's name can be changed in the field above its file name, like a generic `FastBlackFade` for a split made from the preset's `ExitKokiriShop`. The preset keeps its own names.

In the Split Library window, **Edit...** changes an entry's description and its images' names. A renamed image's mask and base image are renamed with it, unless another entry uses the same mask. With **Replace** ticked, it also takes the entry's images and settings from the selected splits again, so you can insert an entry, tune it in the editor, and save it back. **Picture...** in the Edit window replaces just that image's base image, leaving the rest as it is. **Delete...** removes an entry.

How the writes protect the library:

- Every file is checked before anything is written, and a write that fails partway deletes what it wrote, so the library stays as it was.
- Files that no entry uses anymore are deleted.
- Saving refuses if the entry changed since its window opened.
- Edit and Delete won't write over or delete a file that a split in the preset being edited still uses. Save the preset first, which copies those files into it.

## Release builds

`build.ps1` (PowerShell 7) builds release zips into `build/`, named `AutoSplit-v{version}-{target}.zip`. On Linux and macOS without PowerShell, `build.sh` does the same with bash.

```sh
./build.ps1                  # pick a target from a menu
./build.ps1 linux-arm64      # one or more targets
./build.ps1 --all            # every target this machine can build
./build.ps1 --debug ...      # keep debug symbols
./build.sh ...               # the same, with bash
```

The targets are `windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` and `macos-universal`, which is one binary for both Apple Silicon and Intel Macs. The script installs the Rust targets and cargo tools it needs. A target for this machine's OS builds natively, and the others cross-compile:

| Building on | Linux targets | Windows targets | macOS |
| --- | --- | --- | --- |
| Windows | [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), which needs [Zig](https://ziglang.org/download) or `pip install ziglang` | native | not possible |
| Linux | native, or cargo-zigbuild for the other architecture | [cargo-xwin](https://github.com/rust-cross/cargo-xwin), which needs clang and lld | not possible |
| macOS | cargo-zigbuild | cargo-xwin | native |

- macOS builds need a Mac, because Apple's SDK may only be used there.
- Cross-compiled Linux binaries need glibc 2.35 or newer (Ubuntu 22.04).
- Windows ARM64 builds on Windows need the MSVC ARM64 build tools from the Visual Studio installer.

### What's in the zips

- Every zip has the `presets` and `splits` folders.
- Windows zips have `AutoSplit.exe` and `AutoSplit.com`, renamed from Cargo's `autosplit.exe` and `autosplit-console.exe`. See [Command line](MANUAL.md#command-line) for what each one is for.
- Linux zips also have `icon.png` and `install-desktop-entry.sh`, which adds the program to the desktop's app menu.
- Symbols are stripped unless you pass `--debug`.

### Platform details

- The macOS binary is bare, not an `.app`. So `crates/autosplit-cli/build.rs` links an Info.plist into it, with the bundle id and the camera usage text, and `.cargo/config.toml` sets `MACOSX_DEPLOYMENT_TARGET` to 11.0.
- On Windows, `build.rs` embeds the icon instead.
- Shell scripts keep LF line endings through `.gitattributes`, since a CR after `#!` breaks them.

## CI

`.github/workflows/validate.yml` runs on pull requests, on pushes to `main`, and by hand from the Actions tab. It:

1. checks formatting and clippy,
2. draws every window without a display,
3. runs the tests and a release build for every target, natively on Windows, Linux and macOS, x64 and ARM64. The x64 half of the macOS build is tested under Rosetta 2.

The tests run in the `ci` profile, which is release without link-time optimization, because that's much quicker to build. Only runs on `main` save their build caches, which every branch can read.

`.github/workflows/release.yml` runs on `v*` tags. It checks that the tag matches the version in `Cargo.toml`, runs the validate workflow, and creates a draft GitHub release with the zips. A tag with a `-`, like `v0.2.0-beta`, becomes a prerelease. Linux release builds run on Ubuntu 22.04, so they work on 22.04 and newer.
