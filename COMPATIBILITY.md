# Compatibility with Toufool's AutoSplit

AutoSplit-rs aims to behave like [Toufool's AutoSplit](https://github.com/Toufool/AutoSplit), the original Python program, so existing split images, profiles and the LiveSplit integration keep working. [Added in AutoSplit-rs](#added-in-autosplit-rs) lists what Toufool's AutoSplit doesn't have.

## Feature status

What Toufool's AutoSplit does, and how far AutoSplit-rs has it.

| Area | State |
| --- | --- |
| Split image filename conventions (`(0.95)`, `[10]`, `#500#`, `@3@`, `^1^`, `{dbp}`, `start_auto_splitter`, `reset`) | Done |
| Image loading, masks and the adaptive resize for transparent images | Done, PNG only |
| L2 norm, histograms and pHash comparisons | Done, bit-exact with Toufool's AutoSplit |
| OCR split files (tesseract) | Not supported for now. They're skipped with a warning |
| Split image folder validation, with the error messages of Toufool's AutoSplit | Done |
| Settings profiles (`.toml`) | Done, reads and writes the files of Toufool's AutoSplit |
| Split state machine (delays, pauses, below flag, dummies, loops, reset image, start image, skip and undo) | Done |
| LiveSplit's AutoSplit Integration component (`--auto-controlled`) | Done, as a drop-in replacement for the executable of Toufool's AutoSplit |
| Video capture devices | Done: DirectShow on Windows (pixel-identical setup to the OpenCV of Toufool's AutoSplit), V4L2 on Linux |
| Global hotkeys and sending key presses | Done, with the hotkey names of Toufool's AutoSplit |
| Screenshots | Done |
| Screen and window capture | Not planned: OBS Virtual Camera captures any window or screen, and PC games are better split by a memory-reading autosplitter |
| Window (Slint): main window, settings, Set Hotkey | Done, for capture devices |
| Align Region | Removed: split images are made for one capture setup, so the region is set once with its boxes |
| Select Region and Select Window | Removed, with screen and window capture |

## Added in AutoSplit-rs

Toufool's AutoSplit has none of these:

- macOS, with AVFoundation for capture devices. Shortcuts follow its other apps: Cmd+M minimizes and Cmd+W closes any window, so Make Split Images is Cmd+Shift+M and Help is Cmd+?. Help > About is called Licenses and Links there, since the app menu has its own About. The Capture Region picture redraws at most 30 times a second, since each preview frame redraws the whole window there. Comparisons aren't affected.
- Two more timer connections besides hotkeys and LiveSplit: LiveSplit One over a WebSocket, and Molok through POSIX signals on Linux. The settings window has a fourth tab, Other, to pick one and set the LiveSplit One port. It's disabled when LiveSplit started AutoSplit-rs.
- Up to 4 split images compared for one split, grouped by their number, and up to 4 start and reset images, off by default. A 2x2 grid can show a split's images.
- The Live Tester, which compares one split image with the capture outside of a run. It replaces AutoMask's tester.
- Make Split Images, which applies a preset's masks to the capture or to screenshots and saves the split images, with savestates to copy for a practice tool. It replaces AutoMask, and its preset editor replaces AutoMask's.
- Commands that work without the window: `devices`, `grab`, `run`, `check`, `compare`, `replay` and `bench`. See [Command line](MANUAL.md#command-line).

## Tests

The tests compare AutoSplit-rs with values from the Python modules of Toufool's AutoSplit on OpenCV 4.11.0.86, the version its lockfile pins. They cover:

- Every comparison method on real masked split images (made with AutoMask) and synthetic ones, against several capture sizes, including captures with alpha 0 like BitBlt produces. They go through the same code the runner uses, and match to the last bit.
- Intermediate steps: nearest, area and linear-exact resizes, grayscale conversion and 400 pHash hashes.
- Filename parsing: thresholds, pauses, delays, loops, methods and flags, including edge cases like `inf`, `1e2`, `.5`, `+1`, unclosed delimiters and conflicting flags.
- Profiles written with tomli_w, the writer Toufool's AutoSplit uses. Profiles saved here load back in Python with the same values and key order.
- Capture: OpenCV's YUV to BGR conversions (NV12, NV21, I420, YV12, YUY2, UYVY) and MJPEG decoding through libjpeg-turbo, the decoder OpenCV uses. The OBS Virtual Camera placeholder frame is detected by the color it really converts to.

CI can't test capture devices. To check yours, `tools/capture_parity/capture_parity.py` grabs a frame the way Toufool's AutoSplit does and compares it with `autosplit grab`. Instructions are at the top of the file.

## Deliberate differences

Similarity values, file formats, the LiveSplit protocol and how a run plays out match Toufool's AutoSplit exactly. Where Toufool's AutoSplit has a bug or a Python quirk, AutoSplit-rs doesn't copy it.

Split images and filenames:

- Only PNG split images are used. Other files in the folder, like JPEGs and OCR split files (`.txt`), are skipped, and loading the folder warns about them. Toufool's AutoSplit uses any image OpenCV reads, so a folder with other formats splits in a different order here.
- Grayscale and 16-bit PNGs load. Toufool's AutoSplit crashes on them.
- A fully transparent split image is a load error, since there's nothing to compare. Toufool's AutoSplit compares it as an opaque image.
- A start or reset image that can't be used makes the folder invalid, like a broken split image. Toufool's AutoSplit lets the folder run, and that image never matches.
- `reset` and `start_auto_splitter` only count as whole words, so `preset_menu.png` is a normal split image. Toufool's AutoSplit matches them anywhere in the name.
- Numbers in filenames are parsed more strictly than Python does. Spaces around the value (`( 0.95 )`), underscores (`#1_000#`) and digits from other scripts (`٣`) aren't accepted, and the default is used instead.
- Ignored filename values get a warning when the folder loads. That covers values that aren't numbers, values out of range (`(1.5)`, `@0@`), unknown comparison methods (`^5^`, which compares as 0) and invalid or conflicting flags (`{x}`, `{dp}`). They're still ignored, like in Toufool's AutoSplit. Delimiters that don't make exactly one value (`x_(0.9`, `x_(0.5)_(0.7)`) are ignored and warned about like Toufool's AutoSplit 2.3.4 does.

Runs:

- Undo works when there's a previous split, and skip when there's a later one. Toufool's AutoSplit goes by its buttons' enabled state, which is sometimes stale, so undo is blocked during the pause right after the first split.
- Next image (or the skip hotkey) does nothing on the last split. In Toufool's AutoSplit it ends the run, while LiveSplit stays on the last split.
- Skipping a split during the start image's pause keeps the skipped-to split. Toufool's AutoSplit sends the skip to the timer, then jumps back to the first split anyway.
- Starting with the split hotkey during the start image's delay drops the pending start. Toufool's AutoSplit sends it once the delay ends, which the timer reads as a split.
- When LiveSplit controls the timer, AutoSplit-rs doesn't send its skips and undos back to it. Toufool's AutoSplit prints them, and LiveSplit ignores them. "Screenshot on" skip and undo still take screenshots.
- During pauses and delays the reset image is checked at its FPS limit. Toufool's AutoSplit checks it every millisecond, which costs a lot of CPU and catches no extra frames.
- The reset image's highest similarity is cleared for every run. Without a start image, Toufool's AutoSplit keeps the highest value across runs.
- The start image is checked every 1/FPS seconds exactly. The timer of Toufool's AutoSplit rounds to whole milliseconds, so it checks 62.5 times a second at 60 FPS.
- An FPS limit of 0 is treated as 1. Toufool's AutoSplit divides by zero.

Capture and hotkeys:

- The capture device is found by its name first, then by the saved index. Toufool's AutoSplit only uses the index, which changes when devices are added or removed.
- On Linux, devices capture at their current size and format. Toufool's AutoSplit gets OpenCV's default of 640x480.
- A capture region with a width or height of 0, or none in the profile, compares nothing and logs a warning. The settings window of Toufool's AutoSplit makes them at least 1, and a 1-pixel area can't match anything useful either.
- A lost device is opened again every 2 seconds. Toufool's AutoSplit stops capturing until the profile is reloaded.
- A profile using another capture method (like Windows Graphics Capture) uses its capture device instead, with a warning.
- Hotkey combos match their exact keys: `ctrl+num 1` doesn't fire for Ctrl+1. When sent, every modifier is pressed. In Toufool's AutoSplit, combos fire for either key, and can drop modifiers when sending.
- For half a second after AutoSplit-rs presses a hotkey, its own hotkey listener ignores that key. Otherwise "start also resets" would reset the run it just started, and the last split could start a new run.
- When two hotkeys are the same, the first one keeps it and a warning is logged. Combos register before single keys. Toufool's AutoSplit clears all but the last one from the profile.
- With `--auto-controlled`, the screenshot and toggle auto reset image hotkeys from the profile work. Toufool's AutoSplit only has them work once set again in the settings window.
- With `--auto-controlled`, AutoSplit-rs exits when LiveSplit closes its input. Toufool's AutoSplit keeps running.
- Under Wayland, message boxes are drawn inside the windows they block instead of in a window of their own, since Wayland doesn't let a program keep a window in front. Elsewhere they're separate windows on top, like in Toufool's AutoSplit.
- Hotkey names follow the keyboard layout on Windows, including localized key names, like Toufool's AutoSplit. On Linux and macOS they follow the US layout, and Set Hotkey names a key by where it is on a US keyboard, so the key pressed is the key listened for on any layout.
- Navigation keys (`home`, `end`, `up`, ...) only fire for those keys, not for the numpad keys that double as them with Num Lock off. `num 7` fires whatever the Num Lock state. Toufool's AutoSplit tells them apart by Num Lock state, which livesplit-hotkey doesn't report.
- OBS Virtual Camera's "not started" placeholder frames are skipped and the last real frame stays current. After a second of them, AutoSplit-rs says OBS Virtual Camera isn't started. Toufool's AutoSplit tries to skip them too, but checks for a color they don't convert to in OpenCV 4.11, so it compares the gray placeholder.
- On Linux, padded rows from a driver are read with their stride. OpenCV assumes packed rows and gets a sheared image. BGR3 frames work, which OpenCV 4.11 can't convert.
- Relative split image and screenshot folders are read from the profile's folder. Toufool's AutoSplit reads them from wherever it was started.

Settings and display:

- Keys missing from a profile get the settings window's defaults (threshold 0.90). Toufool's AutoSplit uses a second set of defaults for those (threshold 0.95).
- Profile values of the wrong type, like `fps_limit = 60.0`, make the profile invalid. Toufool's AutoSplit uses them as they are.
- Error messages are reworded and name the file or value involved.
- Similarities and thresholds are shown with 3 decimals, truncated. Toufool's AutoSplit shows 2, and its truncation sometimes comes out one step low (0.29 shows as 0.28).

Window:

- Help > View Log shows everything AutoSplit-rs logged, which is also in `autosplit.log`. Toufool's AutoSplit only shows errors, in message boxes, and those are here too.
- Split image folder errors only get a message box when Start Auto Splitter, Reload Split Images or a run start loaded the folder. Otherwise they're logged, and the current image area says the folder can't be used. Toufool's AutoSplit mostly does the same.
- Reload Split Images replaces the Reload Start Image button of Toufool's AutoSplit, since it loads and checks the whole folder. It doesn't show a message box when the folder has no start image.
- Help > View Help opens [TUTORIAL.md](TUTORIAL.md), and the settings' Tutorial button the file name options in [MANUAL.md](MANUAL.md#split-image-file-names), not the tutorial of Toufool's AutoSplit.
- Hotkey fields can be typed in, like `ctrl+shift+f5`, with the same check Toufool's AutoSplit does when setting one. In Toufool's AutoSplit, they're read-only.
- Set Hotkey reads the key while the settings window has focus, not globally. Escape clears the hotkey, and clicking Set Hotkey again cancels.
- Set Hotkey names keys without what Shift types on them (`shift+1`, not `shift+!`). Keypad keys are `num 7` whether Num Lock is on or not, where Toufool's AutoSplit records `home` with Num Lock off.
- Keyboard shortcuts (Ctrl+S and the others) work in the main and settings windows, not in About or the log window. In Toufool's AutoSplit, they work in all of its windows.
- There's no Max FPS button. It waited for a new frame per comparison, so it only showed the capture device's frame rate.
- There's no save prompt when nothing changed. Toufool's AutoSplit asks for every new profile, because its window's defaults differ from its profile defaults.
- Warnings about a loaded profile (keys in the wrong place) are shown in one message box.
- The Capture Region picture stops updating while the window is minimized or covered, which saves the work of converting frames nobody sees.
- Buttons and spin boxes copy the Qt Fusion style of Toufool's AutoSplit. Text uses grayscale smoothing, so it looks a little softer than Qt's ClearType.
- The settings window is larger (380x384 instead of 284x334) and laid out differently, and its check boxes use Slint's style.
- Capture devices are listed by name only. Toufool's AutoSplit adds the backend and whether the device is in use.
- About shows the version, links to AutoSplit-rs and Toufool's AutoSplit on GitHub, and the licenses. It has no update check or donate button.

Image math:

- pHash computes the DCT in f64 where OpenCV uses f32. A hash bit can differ when a coefficient is within float noise of the mean. None of the test inputs hit this.
