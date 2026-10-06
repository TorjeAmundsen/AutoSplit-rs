# Tutorial

How to set up and use `autosplit`, the Rust rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit). It works like AutoSplit and reads the same split images and profiles, so AutoSplit's own guides mostly apply too. The differences are listed in the [README](README.md#deliberate-differences-from-autosplit).

Only video capture devices work for now: capture cards, webcams and OBS Virtual Camera. Screen and window capture come later. To split from a game on the same computer, start OBS's Virtual Camera and pick it as the capture device.

## Getting started

1. Download the zip for your system from the releases (`windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` or `macos-universal`) and unzip it.
2. macOS blocks the first start because the program isn't notarized. Run `xattr -d com.apple.quarantine autosplit` in the folder, or allow it in System Settings > Privacy & Security > Open Anyway.
3. On Linux and macOS, set up the [permissions](#permissions).
4. Start `autosplit` (`autosplit.exe` on Windows). Like AutoSplit, it loads the profile next to it when there's exactly one `.toml` file there. Otherwise use File > Load Profile, or set things up and save them with File > Save Profile As.
5. In File > Settings, pick your capture device. With Live Capture Region on, the left preview shows what gets compared.
6. Set the capture region: type X, Y, Width and Height, or use Align Region.
7. Pick your split image folder with Browse..., set the hotkeys your timer uses in Settings > Hotkeys, and press Start Auto Splitter (or your split hotkey).

If it doesn't split, Help > View Log says why. Everything there is also in `autosplit.log` next to the program (or in the temp folder when that folder is read-only).

## Which timer setup to use

- Windows with LiveSplit: add LiveSplit's AutoSplit Integration component, and in its settings choose `autosplit.exe` and your profile. LiveSplit then starts `autosplit` and tells it when the timer starts, splits, skips, undoes and resets, like it does for AutoSplit. The window's title then says "(externally controlled)".
- LiveSplit One, on any system: see [LiveSplit One](#livesplit-one). It needs no hotkeys and no permissions.
- Any other timer: hotkeys, like AutoSplit. `autosplit` presses the profile's hotkeys, and the timer listens for them as global hotkeys.

## The main window

### Split image folder

The folder with your split images, picked with Browse.... They're used in file name order, so number them: `001_SplitName.png`, `002_SplitName.png`, and so on. Only PNG images work. How a file is named sets how it's used, see [Split image file names](#split-image-file-names).

Take Screenshot saves the capture region as the next free `NNN_SplitImage.png` in the screenshot folder (Settings > Capture Settings), or in the split image folder when none is set. That's the most accurate way to make split images.

### Capture region

The part of the capture device's picture that split images are compared with, usually the whole game screen. X and Y are where it starts, in pixels of the device's frames, and Width and Height its size. The arrow keys change the number in a box by 1.

Align Region asks for a split image (or any reference image cut from the capture), finds it in the current capture region at sizes from 20% to 300%, and sets the region to where it matched. The capture region has to contain it already.

Select Region and Select Window are for screen and window capture, which don't exist yet.

Turn off Live Capture Region in the settings once the region is right. The preview then stops, which saves a little CPU.

### Max FPS

Compares each split image with 10 new frames and shows how many comparisons a second that makes. It can't be more than the device's frame rate. Set the FPS limit in the settings near what the game runs at; more only costs CPU.

### Current image

Shows the split image being compared, with its file name above it. `<` and `>` move to the previous and next image without telling the timer. The similarity viewer below the live preview shows:

- Live: how similar the capture is to the image now, from 0 to 1.
- Highest: the highest it got for this image.
- Threshold: what it has to reach to split.

The second row is the same for the reset image. Image Loop shows which loop of a looping image this is, and Start Image Status whether the start image is being looked for.

### Buttons

- Start Auto Splitter starts the run, like the split hotkey. With a start image, it starts looking for it instead.
- Reset stops the run, like the reset hotkey.
- Undo and Skip move to the previous or next split, like the undo and skip hotkeys.
- Reload Start Image loads the split image folder again and looks for the start image.

## Settings

### Capture settings

- Comparison FPS Limit: how often the capture is compared, 20 to 240 times a second.
- Live Capture Region: show the live preview.
- Capture device: the device to capture from. `autosplit` finds it by name, so it keeps working when devices are added or removed.
- Screenshots folder: where Take Screenshot and the "Screenshot on" options save. Open screenshots on capture opens each one.
- Screenshot on Start, Split, Reset, Skip, Undo and Pause: take a screenshot when that happens, to check later what split.

### Image settings

These are the defaults for images whose file names don't set their own.

- Default Comparison Method:
  - L2 Norm compares every pixel. Fast, and fine for most images, but moving or rotating content changes it a lot.
  - Histograms compares the colors. Good for masked images.
  - pHash compares the overall shape. Don't use it with masked images.
- Default Similarity Threshold: the similarity an image has to reach to split.
- Default Delay Time: how long to wait after a match before splitting, in milliseconds.
- Default Pause Time: how long to stop comparing after a split, in seconds. It keeps two identical images in a row from splitting twice, and saves CPU.
- Loop Last Split Image to First Split Image: after the last image splits, start over with the first one. Otherwise comparing stops.
- Start also Resets: starting (with the start image) also resets the timer first, so a running timer starts over.
- Enable auto reset image: use the reset image. The toggle auto reset image hotkey turns it on and off, for example when a mistake in the run would show the reset image.

### Hotkeys

The split, reset, undo, skip and pause hotkeys must be the same as your timer's, with global hotkeys on in the timer. `autosplit` presses them, and also listens for them: pressing the split hotkey starts the run, reset stops it, and undo and skip move along with the timer. The pause hotkey is only pressed for images with the `{p}` flag.

To set one, click Set Hotkey and press the key, with any modifiers. Escape clears it, and clicking Set Hotkey again cancels. You can also type a name in the field and press Enter, like `num 1`, `ctrl+shift+f5`, `page up` or `plus`. A hotkey can only be used once: setting it for one action clears it from another.

Keypad keys are named `num 0` to `num 9`, `decimal`, `plus`, `-`, `*` and `/`, whether Num Lock is on or not. Set Hotkey names the key without what Shift types on it: Shift+1 is `shift+1`.

When LiveSplit or LiveSplit One controls the timer, only the screenshot and toggle auto reset image hotkeys are used.

### Other

- Tell the timer with: hotkeys, or LiveSplit One. It's used after a restart.
- LiveSplit One port: the port LiveSplit One connects to, 16835 unless it's taken.

## Split image file names

Each image can have its own settings, written in its file name. They override the defaults from the settings window.

| Setting | Written as | Example |
| --- | --- | --- |
| Similarity threshold | between parentheses | `(0.95)` |
| Pause time after the split, in seconds | between square brackets | `[10]` |
| Delay before splitting, in milliseconds | between hash signs | `#3500#` |
| Times to split on this image in a row | between at signs | `@3@` |
| Comparison method: 0 L2 Norm, 1 Histograms, 2 pHash | between carets | `^1^` |
| Flags | between curly brackets | `{db}` |

Flags, several of which can go in one pair of brackets:

- `{d}`: dummy split. When it matches, `autosplit` moves to the next image without splitting. Skip and undo keep dummies together with the real split after them, so they stay in step with the timer.
- `{b}`: split when the similarity drops below the threshold, after it first went above it.
- `{p}`: press the pause hotkey instead of the split hotkey.

Examples:

- `001_SplitName_(0.9)_[10].png`: threshold 0.9, then stop comparing for 10 seconds.
- `002_SplitName_(0.9)_[10]_{d}.png`: the same, as a dummy split.
- `003_SplitName_(0.85)_[20]_#3500#.png`: threshold 0.85, splits 3.5 seconds after matching, then pauses 20 seconds. Skip and undo don't work during the delay.
- `004_SplitName_(0.9)_[10]_#3500#_@3@_{b}.png`: also splits 3 times in a row, each time when the similarity drops below 0.9.

Values need their closing character: `(0.9` sets nothing.

### Start image

One image can have `start_auto_splitter` in its name, not next to other letters. Before the run, `autosplit` looks for it, and starts the timer when it matches. Its pause time is how long to wait before comparing the first split image, and its delay how long to wait before starting the timer. Reload Start Image loads it again after you change it.

### Reset image

One image can have `reset` in its name, not next to other letters (`preset_menu.png` is a normal split image). During the run, `autosplit` resets the timer when it matches. It isn't one of the splits. Its pause time is how long after the start it waits before looking for it. Example: `reset_(0.95)_[10].png`.

### Masked images

When only part of the capture is always the same, like text over a changing background, make everything else transparent in the image: take a screenshot of the whole capture region, and erase the rest in an editor like paint.net or GIMP. Only the parts that aren't transparent are compared. Use L2 Norm or Histograms for them, not pHash.

## Profiles

A profile (`.toml`) has all the settings, including the capture region. File > Save Profile saves to the loaded profile, File > Save Profile As to a new file, and File > Load Profile opens one. Profiles from AutoSplit load as they are, and profiles saved here load in AutoSplit. Closing the window with unsaved changes asks whether to save them.

### Making a profile by hand

AutoSplit has no macOS version, so Mac users may like to write a profile in a text editor. Keys that are left out get the defaults of the settings window.

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

Keys after `[capture_region]` belong to the capture region and are ignored, so put new ones above it. The four hotkeys aren't needed with `timer_connection = "livesplit_one"`. Relative folders are read from the profile's folder.

### Using a profile from another computer

- Change the split image folder to where the images are now.
- Device names can differ between systems. Pick the device again in Settings.
- If the device's frames have another size there, change the capture region to match.

## LiveSplit One

LiveSplit One (the web version or the desktop app) can be controlled without hotkeys. Choose LiveSplit One in Settings > Other, save, and restart `autosplit`. Then in LiveSplit One's menu choose "Connect to Server" and enter `ws://localhost:16835` (or the port you set).

- `autosplit` sends starts, splits, resets and pauses (images with the `{p}` flag). Resets save the attempt without asking, like LiveSplit does for AutoSplit.
- Starting, splitting, skipping, undoing and resetting in LiveSplit One moves `autosplit` along too.
- Start `autosplit` first. LiveSplit One doesn't reconnect by itself: after restarting `autosplit`, choose Connect to Server again.
- LiveSplit One must run on the same computer. Only LiveSplit One can connect, not other web pages.
- Only one LiveSplit One can be connected. A new connection replaces the older one.
- If the browser refuses to connect (Safari has blocked this in the past), use Chrome, Edge or Firefox, or the LiveSplit One desktop app. Chrome and Edge may ask whether one.livesplit.org may access devices on your network: allow it.

AutoSplit ignores these settings, so the profile keeps working with it.

## Permissions

- Linux:
  - Global hotkeys need to read the keyboards: `sudo usermod -a -G input $USER`, then log out and back in. Any program you run can then read your keystrokes.
  - Pressing hotkeys needs `/dev/uinput`: `echo 'KERNEL=="uinput", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/50-uinput.rules`, then restart. If `/dev/uinput` doesn't exist, load it with `sudo modprobe uinput` (and `echo uinput | sudo tee /etc/modules-load.d/uinput.conf` to keep it after a restart). Under WSL, key presses can't be sent.
  - Capture devices need the `video` group, which desktop users usually have.
  - The window needs fontconfig and xkbcommon, which every desktop has.
  - Without these, `autosplit` still runs and says what's missing. The LiveSplit One connection needs none of them.
- macOS: in System Settings > Privacy & Security, allow `autosplit` (or your terminal app, like Terminal or iTerm, when you start it from there) under Camera for capture devices, Accessibility for pressing hotkeys and Input Monitoring for listening to them. Then restart it.

## From a terminal

Everything also works without the window. In the commands below, type `.\autosplit` on Windows and `./autosplit` on macOS and Linux instead of `autosplit`. Put paths with spaces in "quotes".

1. Run `autosplit devices` and note your capture device's name.
2. Run `autosplit grab --device "<name>" frame.png` and check that `frame.png` shows the game. With `--profile settings.toml`, it saves only the capture region, which is exactly what gets compared.
3. Run `autosplit check settings.toml` to check the split images.
4. Run `autosplit run settings.toml`. It prints what happens, with the current split image and its similarity on the last line. Press Ctrl+C to stop. `--split-dir <folder>` uses another split image folder.

Without a capture device, `run` keeps going, looks for the device every 2 seconds and shows `NO DEVICE`. `autosplit --help` lists the other commands.

The Windows zip has two programs: `autosplit.exe` opens the window without a console, and `autosplit.com` is for terminals. Typing `autosplit` in a terminal runs `autosplit.com`, which runs `autosplit.exe` inside the terminal so its output and Ctrl+C work there. Without a command, it opens the window and returns to the prompt.
