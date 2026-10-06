# AutoSplit rewrite

A from-scratch Rust rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit), the image comparison based auto splitter for speedrunning. Goals:

- Same behavior as AutoSplit, including the similarity values it computes.
- Existing split images, settings profiles and the LiveSplit integration keep working.
- Windows, Linux and macOS.
- Low overhead, so it can run next to OBS on a weak machine.

In this README, "AutoSplit" is the original Python program, and `autosplit` is this one.

## Getting started

1. Download the zip for your system from the releases (`windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` or `macos-universal`) and unzip it.
2. `autosplit` is a command-line program for now: double-clicking it only shows its help. Open PowerShell (Windows: in the folder, right-click > Open in Terminal) or a terminal (macOS, Linux) in the unzipped folder. In the commands below, type `.\autosplit` on Windows and `./autosplit` on macOS and Linux instead of `autosplit`. Put paths with spaces in "quotes"; dragging a file into the window pastes its path.
3. macOS blocks the first start because the program isn't notarized. Run `xattr -d com.apple.quarantine autosplit`, or allow it in System Settings > Privacy & Security > Open Anyway.
4. On Linux and macOS, set up the [permissions](#permissions).
5. Run `autosplit devices` and note your capture device's name.
6. Run `autosplit grab --device "<name>" frame.png` and check that `frame.png` shows the game.
7. Run `autosplit check settings.toml` to check the split images. `settings.toml` is the profile you saved in AutoSplit (File > Save Profile As).
8. Run `autosplit run settings.toml`. It prints what happens, with the current split image and its similarity on the last line. Press Ctrl+C to stop.

Without a capture device, `run` keeps going, looks for the device every 2 seconds and shows `NO DEVICE`.

### Which timer setup to use

- Windows with LiveSplit: add LiveSplit's AutoSplit Integration component, and in its settings choose `autosplit.exe` and your profile. LiveSplit then starts `autosplit` with `--auto-controlled`, like it does AutoSplit. Nothing is shown; if it doesn't split, read `autosplit.log` next to `autosplit.exe` (or in the temp folder when that folder is read-only).
- LiveSplit One, on any system: the [LiveSplit One](#livesplit-one) connection. Browsers only see key presses while focused, so hotkeys don't reach LiveSplit One in a browser.
- Otherwise: hotkeys, like AutoSplit. `autosplit` presses the profile's hotkeys, which the timer listens for.

### LiveSplit One

LiveSplit One (the web version or the desktop app) can be controlled without hotkeys. Add these lines near the top of the profile, above the `[capture_region]` line (lines after it belong to the capture region and are ignored):

```toml
timer_connection = "livesplit_one"
livesplit_one_port = 16835  # optional, this is the default
```

Then start `autosplit run`, and in LiveSplit One's menu choose "Connect to Server" and enter `ws://localhost:16835`.

- `autosplit` sends starts, splits, resets and pauses (images with the `{p}` flag). Resets save the attempt without asking, like LiveSplit does for AutoSplit.
- Starting, splitting, skipping, undoing and resetting in LiveSplit One moves `autosplit` along too, like the LiveSplit integration does.
- Start `autosplit` first. LiveSplit One doesn't reconnect by itself: after restarting `autosplit`, choose Connect to Server again.
- LiveSplit One must run on the same computer. Only LiveSplit One can connect, not other web pages.
- Only one LiveSplit One can be connected. A new connection replaces the older one.
- If the browser refuses to connect (Safari has blocked this in the past), use Chrome, Edge or Firefox, or the LiveSplit One desktop app. Chrome and Edge may ask whether one.livesplit.org may access devices on your network: allow it.

AutoSplit ignores these keys, so the profile keeps working with it.

### Making a profile without AutoSplit

AutoSplit has no macOS version, so Mac users can write a profile by hand. Keys that are left out get the defaults of AutoSplit's settings window.

```toml
split_hotkey = "num 1"
reset_hotkey = "num 3"
undo_split_hotkey = "num 8"
skip_split_hotkey = "num 2"
capture_method = "VIDEO_CAPTURE_DEVICE"
capture_device_name = "OBS Virtual Camera"  # as `autosplit devices` shows it
split_image_directory = "/Users/me/Splits/OoT Any%"  # on Windows, use / or 'single quotes'
default_similarity_threshold = 0.95
timer_connection = "livesplit_one"  # optional, see LiveSplit One

[capture_region]  # in pixels of the device's frames; `autosplit grab` shows their size
x = 0
y = 0
width = 1920
height = 1080
```

The four hotkeys aren't needed with `timer_connection = "livesplit_one"`. Relative folders are read from the profile's folder.

### Using a profile from another computer

- Change `split_image_directory` to where the split images are now, or use `autosplit run --split-dir <folder>`.
- Device names can differ between systems. Check `capture_device_name` against `autosplit devices`.
- If the device's frames have another size there, change `[capture_region]` to match. `autosplit grab --profile settings.toml --device "<name>" region.png` saves exactly what gets compared.

### Permissions

- Linux:
  - Global hotkeys need to read the keyboards: `sudo usermod -a -G input $USER`, then log out and back in. Any program you run can then read your keystrokes.
  - Sending key presses needs `/dev/uinput`: `echo 'KERNEL=="uinput", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/50-uinput.rules`, then restart. If `/dev/uinput` doesn't exist, load it with `sudo modprobe uinput` (and `echo uinput | sudo tee /etc/modules-load.d/uinput.conf` to keep it after a restart). Under WSL, key presses can't be sent.
  - Capture devices need the `video` group, which desktop users usually have.
  - Without these, AutoSplit still runs and says what's missing. The LiveSplit One connection needs none of them.
- macOS: allow your terminal app (Terminal, iTerm, ...) in System Settings > Privacy & Security, under Camera for capture devices, Accessibility for pressing hotkeys and Input Monitoring for listening to them. Then quit and reopen the terminal.

## Status

Milestone 2 is done: `autosplit run` splits from a video capture device (capture card, OBS Virtual Camera, webcam) with global hotkeys, LiveSplit's `--auto-controlled` integration works, and LiveSplit One can connect over WebSocket. There's no GUI and no screen or window capture yet.

| Area | State |
| --- | --- |
| Split image filename conventions (`(0.95)`, `[10]`, `#500#`, `@3@`, `^1^`, `{dbp}`, `start_auto_splitter`, `reset`) | Done |
| Image loading, masks and the adaptive resize for transparent images | Done, PNG only |
| L2 norm, histograms and pHash comparisons | Done, bit-exact with AutoSplit |
| OCR split files (tesseract) | Not supported for now. Folders containing one can't start |
| Split image folder validation, with AutoSplit's error messages | Done |
| Settings profiles (`.toml`) | Done, reads and writes AutoSplit's files |
| Split state machine (delays, pauses, below flag, dummies, loops, reset image, start image, skip and undo) | Done |
| `--auto-controlled` protocol (LiveSplit integration) | Done |
| Video capture devices | Done: DirectShow on Windows (pixel-identical setup to AutoSplit's OpenCV), V4L2 on Linux, AVFoundation on macOS |
| Global hotkeys and sending key presses | Done, with AutoSplit's hotkey names |
| LiveSplit One (WebSocket) | Done, new |
| Screenshots | Done |
| Screen and window capture | Later |
| GUI (Slint), region and window selection | Milestone 3 |
| LiveSplit integration component (Windows only) | Later |

## Compatibility

The test suite compares this code against values produced by AutoSplit's own Python modules running on OpenCV 4.11.0.86, the version AutoSplit's lockfile pins. Covered:

- Every comparison method on real masked split images (made with AutoMask) and synthetic ones, against several capture sizes, including captures with alpha 0 like BitBlt produces. These go through the same code path the runner uses. Results are equal to the last bit.
- Intermediate steps: nearest, area and linear-exact resizes, grayscale conversion and 400 pHash hashes.
- Filename parsing: thresholds, pauses, delays, loops, methods and flags, including edge cases like `inf`, `1e2`, `.5`, `+1`, unclosed delimiters and conflicting flags.
- Profiles written with tomli_w, the writer AutoSplit uses. Profiles saved here load back in Python with the same values and key order.
- Capture: OpenCV's YUV to BGR conversions (NV12, NV21, I420, YV12, YUY2, UYVY) and MJPEG decoding through libjpeg-turbo, the decoder OpenCV uses. The OBS Virtual Camera placeholder frame is detected by the color it really converts to.

Capture devices themselves can't be tested in CI. To check a device on your machine, `tools/capture_parity/capture_parity.py` grabs a frame the way AutoSplit does and compares it with `autosplit grab`. Instructions are at the top of the file.

### Deliberate differences from AutoSplit

Parity is strict where users depend on it: similarity values, the file formats, the LiveSplit protocol and how a run plays out. Implementation details of the Python version aren't copied.

Split images and filenames:

- Only PNG split images are supported. Other formats get a load error saying so.
- OCR split files (`.txt`) aren't supported yet. A folder with one gets a load error and can't start.
- Grayscale and 16-bit PNGs load. AutoSplit crashes on them.
- A fully transparent split image is a load error, since there's nothing to compare. AutoSplit compares it as an opaque image.
- A start or reset image that can't be used makes the folder invalid, like a broken split image. AutoSplit lets the folder run, and that image never matches.
- `reset` and `start_auto_splitter` only count as whole words, not next to other letters: `preset_menu.png` is a normal split image. AutoSplit matches them anywhere in the name.
- Values need their closing delimiter: `x_(0.9` has no threshold. AutoSplit reads up to the end of the name.
- Numbers in filenames are read with Rust's number parsing, which is stricter than Python's: spaces around the value (`( 0.95 )`), underscores (`#1_000#`) and digits from other scripts (`٣`) aren't accepted, so the default is used.
- Filename values that are ignored get a warning when the folder loads: values that aren't numbers, values out of range (like `(1.5)` or `@0@`), unknown comparison methods (`^5^`, which never matches) and invalid or conflicting flags (`{x}`, `{dp}`). The values are still ignored like in AutoSplit.

Runs:

- Skip split and undo split go by what's possible, not by button state: undo needs a previous split and skip a later one. AutoSplit uses its buttons' enabled state, which is sometimes stale (undo is blocked during the pause right after the first split).
- Next image (or the skip hotkey) does nothing on the last split. In AutoSplit it ends the run, while LiveSplit stays on the last split.
- Skipping a split during the start image's pause keeps the skipped-to split. AutoSplit sends the skip to the timer, then jumps back to the first split anyway.
- Starting with the split hotkey during the start image's delay drops the pending start. AutoSplit sends it once the delay ends, which the timer reads as a split.
- When LiveSplit or LiveSplit One controls the timer, its skips and undos aren't sent back to it. AutoSplit prints them, and LiveSplit ignores them. Screenshots for them ("screenshot on" skip or undo) are still taken.
- During pauses and delays the reset image is checked at its FPS limit. AutoSplit checks it every millisecond, which costs a lot of CPU and catches no extra frames.
- The reset image's highest similarity is cleared for every run. Without a start image, AutoSplit keeps the highest value across runs.
- The start image is checked every 1/FPS seconds exactly. AutoSplit uses a timer of whole milliseconds, so it checks slightly more often (62.5 times a second at 60 FPS).
- An FPS limit of 0 is treated as 1. AutoSplit divides by zero.

Capture and hotkeys:

- The capture device is found by its name first, then by the saved index. AutoSplit only uses the index, which changes when devices are added or removed.
- On Linux and macOS, devices capture at their current size and format. On Linux, AutoSplit gets OpenCV's default of 640x480. On macOS, OpenCV picks a format by its "Medium" preset.
- A capture region with a width or height of 0 (or none in the profile) compares nothing, with a warning. AutoSplit's settings window makes them at least 1, so it compares a 1-pixel area, which can't match anything useful either.
- A lost device is opened again every 2 seconds. AutoSplit stops capturing until the profile is reloaded.
- A profile using another capture method (like Windows Graphics Capture) uses its capture device instead, with a warning.
- Hotkey combos match their exact keys: `ctrl+num 1` doesn't fire for Ctrl+1. When sent, every modifier is pressed. AutoSplit's combos fire for either key, and can drop modifiers when sending.
- Hotkeys that AutoSplit presses itself are ignored by its own hotkeys for half a second. Otherwise "start also resets" would reset the run it just started, and the last split could start a new run.
- When two hotkeys are the same, the first one keeps it, with a warning (combos are registered before single keys). AutoSplit clears all but the last one from the profile.
- With `--auto-controlled`, the screenshot and toggle auto reset image hotkeys from the profile work. AutoSplit only has them work once set again in the settings window.
- With `--auto-controlled`, AutoSplit exits when LiveSplit closes its input. AutoSplit keeps running.
- Hotkey names follow the keyboard layout on Windows, including localized key names, like AutoSplit. On Linux and macOS they follow the US layout.
- Navigation keys (`home`, `end`, `up`, ...) only fire for those keys, not for the numpad keys that share them with Num Lock off. `num 7` fires whatever the Num Lock state. AutoSplit tells these apart by Num Lock state, which the hotkey library used here doesn't report.
- OBS Virtual Camera's "not started" placeholder frames are skipped, so the last real frame stays current, and they don't count as frames: after a second, `autosplit` says there are no frames, or that OBS Virtual Camera isn't started. AutoSplit means to skip them too, but checks a color they don't convert to with OpenCV 4.11, so it compares the gray placeholder.
- Linux: rows that a driver pads are read with their stride. OpenCV assumes tightly packed rows and gets a sheared image. BGR3 frames are converted; OpenCV 4.11 can't.
- Relative split image and screenshot folders are read from the profile's folder. AutoSplit reads them from wherever it was started.

Settings and display:

- Keys missing from a profile get the settings window's defaults (threshold 0.90). AutoSplit uses a second set of defaults for those (threshold 0.95).
- Profile values of the wrong type, like `fps_limit = 60.0`, make the profile invalid. AutoSplit uses them as they are.
- Error messages are reworded and name the file or value involved.
- Similarities and thresholds are shown with 3 decimals, truncated. AutoSplit shows 2, and its truncation sometimes comes out one step low (0.29 shows as 0.28).
- Countdowns say "1.0 second" but "0.5 seconds". AutoSplit uses "second" for anything up to 1.

Image math:

- pHash computes the DCT in f64 where OpenCV uses f32. A hash bit can differ when a coefficient is within float noise of the mean. None of the test inputs hit this.

## Performance

Cost of one comparison, including the resize from a 1920x1080 capture. Median of 5 runs, each a process pinned to one core of an AMD Ryzen 7 7800X3D. AutoSplit is its own Python code on OpenCV 5.0, limited to one thread.

| Split image | Method | This rewrite | AutoSplit |
| --- | --- | --- | --- |
| Opaque (compared at 320x240) | L2 norm | 69 µs | 66 µs |
| Opaque | Histograms | 67 µs | 186 µs |
| Opaque | pHash | 60 µs | 174 µs |
| Masked, large visible area (compared at 887x499) | L2 norm | 72 µs | 514 µs |
| Masked, large visible area | Histograms | 71 µs | 879 µs |
| Masked, large visible area | pHash | 363 µs | 3090 µs |
| Masked, small visible area (real AutoMask reset image) | L2 norm | 3.7 µs | 159 µs |
| Masked, small visible area | Histograms | 5.0 µs | 276 µs |
| Masked, small visible area | pHash | 135 µs | 1132 µs |

Opaque L2 is the one case where OpenCV is faster: its SSE2/AVX2 code computes the squared differences quicker than what LLVM generates from safe Rust. For masked images only the visible pixels are resized and compared. Nothing is allocated per frame, the split image's histogram and hash are computed once at load, and a split image and the reset image of the same size share one resize per frame. Run `autosplit bench` to measure on your machine.

Converting a whole 1080p frame to BGRA takes 0.26 to 0.64 ms, more than most comparisons above. Median of 5 runs on one core of the same CPU, using AVX2. The areas are cut from the middle of a 1920x1080 frame. A raw copy is what keeping a frame to convert later costs. The test frame fits in this CPU's 96 MB L3 cache, so a frame fresh from the device can take longer.

| Format | Whole frame | 640x360 | 320x240 | Raw copy |
| --- | --- | --- | --- | --- |
| NV12 | 568 µs | 68 µs | 21 µs | 98 µs |
| YUYV | 644 µs | 76 µs | 24 µs | 131 µs |
| BGR24, bottom-up | 256 µs | 30 µs | 9.2 µs | 196 µs |

So:

- Only the capture region is converted (except MJPEG frames, which libjpeg-turbo decodes whole), using AVX2 or SSE4.1 when the CPU has them.
- Frames are only converted while something compares them. During pauses, delays and while waiting for the split hotkey, a frame is copied instead when that's cheaper than converting its region, and the newest one is converted when it's needed.
- A frame that was already compared isn't compared again: ticks at the FPS limit often come faster than the device's frames (60 FPS against a 30 FPS OBS Virtual Camera). AutoSplit compares the same frame again; the results are the same, so splits happen at the same time.

## Building

Needs a recent stable Rust toolchain. Linux builds also need CMake, and NASM for x64, to build libjpeg-turbo.

```sh
cargo build --release
cargo test --workspace
```

The tests need no network, Python or capture devices.

### Release builds

`build.ps1` (PowerShell 7) builds release zips into `build/`, named `AutoSplit-v{version}-{target}.zip`:

```sh
./build.ps1                  # pick a target from a menu
./build.ps1 linux-arm64      # one or more targets
./build.ps1 --all            # every target this machine can build
./build.ps1 --debug ...      # keep debug symbols
```

Targets are `windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` and `macos-universal` (one binary for Apple Silicon and Intel Macs). Each target builds natively when it matches this machine's OS. Other targets are cross-compiled:

| Building on | Linux targets | Windows targets | macOS |
| --- | --- | --- | --- |
| Windows | [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), needs [Zig](https://ziglang.org/download) or `pip install ziglang` | native | not possible |
| Linux | native, or cargo-zigbuild for the other architecture | [cargo-xwin](https://github.com/rust-cross/cargo-xwin), needs clang and lld | not possible |
| macOS | cargo-zigbuild | cargo-xwin | native |

The script installs the Rust targets and the cargo tools it needs. macOS builds need a Mac, because Apple's SDK may only be used there. Cross-compiled Linux binaries need glibc 2.35 or newer (Ubuntu 22.04). Windows ARM64 builds on Windows need the MSVC ARM64 build tools from the Visual Studio installer.

### CI

- `.github/workflows/validate.yml` runs on pull requests and pushes to `main`: formatting and clippy, then the tests and a release build for every target, natively on Windows, Linux and macOS, on x64 and ARM64. The x64 half of the macOS build is tested under Rosetta 2.
- `.github/workflows/release.yml` runs on `v*` tags. It checks that the tag matches the version in `Cargo.toml`, runs the validate workflow, and creates a draft GitHub release with the zips. Tags with a `-` (like `v0.2.0-beta`) become prereleases. Linux release builds run on Ubuntu 22.04, so they work on 22.04 and newer.

## CLI

```sh
# List capture devices
autosplit devices

# Save a frame from a capture device, or with --profile only the part that gets compared
autosplit grab --device "OBS Virtual Camera" frame.png [--profile settings.toml]

# Split
autosplit run settings.toml [--split-dir folder]

# Validate a profile's split image folder and show how each image is read
autosplit check settings.toml [--split-dir folder]

# Similarity of a split image against a captured frame, with every method
autosplit compare "split_(0.95).png" frame.png [--profile settings.toml]

# Run the split state machine over a folder of recorded PNG frames
autosplit replay settings.toml frames/ --fps 60 [--auto-controlled]

# Time comparisons
autosplit bench [split images...] --capture 1920x1080
```

## Layout

- `crates/autosplit-core`: everything that doesn't depend on the OS. `runner.rs` is the split state machine. It never blocks: the host calls `tick` with the latest frame when `next_wakeup` is due, and acts on the effects it returns.
- `crates/autosplit-capture`: video capture devices, one backend per OS, and the pixel format conversions.
- `crates/autosplit-input`: global hotkeys (through livesplit-hotkey), sending key presses and AutoSplit's hotkey names.
- `crates/autosplit-cli`: the `autosplit` command. `run.rs` drives the runner from capture (`device.rs`), hotkeys (`hotkeys.rs`), LiveSplit and LiveSplit One (`livesplit_one.rs`).
- `tests/fixtures`: split images, captures and profiles. `real/` holds split images made with AutoMask.
- `tests/golden`: expected values from AutoSplit and OpenCV.
- `tools/golden/generate.py`: regenerates the fixtures and golden values. Instructions are at the top of the file. Only rerun it when you mean to change the baseline.

## License

GPL-3.0-only, like AutoSplit. Parts of the image processing are ports of OpenCV, see `THIRD_PARTY_NOTICES.md`.
