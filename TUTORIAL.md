# Tutorial

How to set up and use `autosplit`, the Rust rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit). It reads AutoSplit's split images and profiles, so AutoSplit's guides mostly apply too. [COMPATIBILITY.md](COMPATIBILITY.md#deliberate-differences) lists the differences.

Only video capture devices work for now, meaning capture cards, webcams and OBS Virtual Camera. To split a game on the same computer, or a capture card that OBS already uses, go through [Setting up OBS](#setting-up-obs) first.

## Getting started

1. Download the zip for your system from the releases (`windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` or `macos-universal`) and unzip it.
2. macOS blocks the first start because the program isn't notarized. Run `xattr -d com.apple.quarantine autosplit` in the folder, or click Open Anyway in System Settings > Privacy & Security.
3. On Linux and macOS, set up the [permissions](#permissions).
4. Start `autosplit` (`AutoSplit.exe` on Windows). If there's exactly one `.toml` file next to it, it loads that profile, like AutoSplit. Otherwise use File > Load Profile, or set things up and save with File > Save Profile As.
5. In File > Settings, pick your capture device. With Live Capture Region on, the left preview shows what gets compared.
6. Set the [capture region](#capture-region).
7. Pick your split image folder with Browse..., set the hotkeys your timer uses in Settings > Hotkeys, and press Start Auto Splitter (or your split hotkey).

If it doesn't split, Help > View Log says why. The same log is in `autosplit.log` next to the program, or in the temp folder when that folder is read-only.

## Which timer setup to use

- Windows with LiveSplit: add LiveSplit's AutoSplit Integration component, and in its settings choose `AutoSplit.exe` and your profile. LiveSplit starts `autosplit` and tells it when the timer starts, splits, skips, undoes and resets, like it does for AutoSplit. The window title says "(externally controlled)". If the component already runs AutoSplit, point it at `AutoSplit.exe` instead and keep your profile.
- LiveSplit One, on any system: see [LiveSplit One](#livesplit-one). It needs no hotkeys and no permissions.
- Molok, on Linux: see [Molok](#molok). It needs no hotkeys and no permissions.
- Any other timer: hotkeys, like AutoSplit. `autosplit` presses the profile's hotkeys, and the timer listens for them as global hotkeys.

![LiveSplit's Layout Settings with the AutoSplit Integration component, AutoSplit.exe and a profile picked](docs/images/livesplit-integration.png)

## Setting up OBS

A capture card can only be used by one program at a time. If OBS shows your game, let OBS pass it on through its Virtual Camera instead. These steps give `autosplit` exactly the game picture, with no black bars around it, which matters once you make split images.

![The same game feed twice: uncropped with black bars around it, and cropped and scaled to fill the frame](docs/images/obs-black-bars.png)

### 1. Crop with the Crop/Pad filter

Right-click your capture source, choose Filters, and add a Crop/Pad filter. Crop until the black bars are gone, with the game running so you can see where the picture ends.

Don't crop by alt-dragging the source in OBS's preview. That only changes how the source sits on your canvas, and the Virtual Camera still sends the uncropped source. If your source is set up that way, right-click it, choose Transform > Reset Transform, and crop with the filter instead.

### 2. Scale to 4:3 with the Scaling/Aspect Ratio filter

Add a Scaling/Aspect Ratio filter below Crop/Pad. Set the resolution to the 4:3 size that fills your canvas's height: 1440x1080 on a 1920x1080 canvas, 960x720 on a 1280x720 canvas. Set the scale filtering to Area, or Point for a sharper pixel look.

A known size makes the capture region in step 5 easy to work out. Other sizes work too, but then you do your own math.

![OBS's filter window for the capture source, with Crop/Pad above Scaling/Aspect Ratio set to 1440x1080 and Area](docs/images/obs-filters.png)

### 3. Color correct now, if ever

Changing colors later means making every split image again. [AutoLUT](https://torjeamundsen.github.io/AutoLUT/) makes a LUT that corrects Wii and N64 capture to the console's real colors, for OBS's Apply LUT filter. Put Apply LUT first, then Crop/Pad, then Scaling/Aspect Ratio.

### 4. Start the Virtual Camera

Click the gear next to Start Virtual Camera in OBS. Set Output Type to Source and Output Selection to your capture source, then start the Virtual Camera. It now sends that source with its filters applied.

![OBS's Virtual Camera settings with Output Type set to Source and the capture source picked](docs/images/obs-virtual-camera.png)

### 5. Pick it in `autosplit` and set the capture region

In File > Settings > Capture Settings, pick OBS Virtual Camera as the capture device.

![autosplit's settings window on Capture Settings, with OBS Virtual Camera picked](docs/images/settings-capture-device.png)

The Virtual Camera sends a picture the size of OBS's canvas, with your source in the middle. Set the capture region to just the source:

- 1920x1080 canvas, 1440x1080 source: X 240, Y 0, Width 1440, Height 1080.
- 1280x720 canvas, 960x720 source: X 160, Y 0, Width 960, Height 720.
- Any other size: X = (canvas width - source width) / 2, Width = source width, Y 0 and Height the canvas height.

![The live preview before and after setting the capture region: black bars on both sides, then only the game](docs/images/capture-region.png)

## The main window

![The main window, with the split image folder, the capture region boxes, the live preview, the current split image and the buttons marked](docs/images/main-window-annotated.png)

### Split image folder

The folder with your split images, picked with Browse.... They're used in file name order, so number them `001_SplitName.png`, `002_SplitName.png` and so on. Only PNG images work. The file name also sets how each image is used, see [Split image file names](#split-image-file-names).

Take Screenshot saves the capture region as the next free `NNN_SplitImage.png` in the screenshot folder (Settings > Capture Settings), or in the split image folder when none is set. Its file name shows under the button for a few seconds. It's the most accurate way to make split images, since it saves exactly what gets compared. [Make Split Images](#make-split-images) turns those screenshots into masked, named split images when your category has a preset. Check them with the [Live Tester](#live-tester).

### Capture region

The part of the capture device's picture that split images are compared with, usually the whole game screen. X and Y are where it starts, in pixels of the device's frames, and Width and Height its size. The arrow keys change the number in a box by 1.

A new profile starts with 640x480 at the top left, like AutoSplit. With OBS Virtual Camera, whose frames are the whole OBS canvas, usually 1920x1080, that region becomes the whole frame once frames arrive. A region you set yourself is never changed.

Crop to 4:3 sets the region to the 4:3 picture in the middle of the capture, which is how OBS shows a 4:3 game in a 16:9 canvas: X 240, Y 0, Width 1440 and Height 1080 in 1920x1080.

Once the region is right, you can turn off Live Capture Region in the settings to stop the preview and save a little CPU.

### Current image

Shows the split image being compared, with its file name above it. `<` and `>` move to the previous and next image without telling the timer. The similarity viewer below the live preview shows:

- Live: how similar the capture is to the image now, from 0 to 1.
- Highest: the highest it got for this image.
- Threshold: what it has to reach to split.

The second row is the same for the reset image. Image Loop shows which loop of a looping image this is, and Start Image Status whether the start image is being looked for.

### Buttons

- Start Auto Splitter starts comparing the split images. It doesn't start the timer, so start that yourself or use the split hotkey, which the timer hears too. With a start image, `autosplit` looks for it as soon as the folder loads, and Start Auto Splitter skips it.
- Reset stops comparing, without resetting the timer.
- Undo and Skip move to the previous or next split and press the undo and skip hotkeys, so the timer moves too.
- Reload Split Images loads the split image folder again and looks for the start image. Changes to the folder are only picked up by this button, a run start, or changing the folder or profile.

### Live Tester

File > Live Tester (Ctrl+T) compares one split image with the capture region without starting a run, to check a new image before you rely on it. It opens beside the main window, which can't be used while the tester is open: close the tester to change the capture region. Settings still opens beside it, from File > Settings, Ctrl+, or the Settings button in the main window, and during a run the main window keeps Undo, Skip and Reset next to that button. It opens the first image of the split image folder, and switches to the new folder's first image when you change the folder or load another profile. Without a split image folder, it asks for an image. Open Image... picks another one, from any folder, and `<` and `>` next to it open the previous and next PNG in the same folder, in file name order. The image loads again by itself when its file changes, for example when Make Split Images saves it again. Reload does the same by hand.

The image is read like in the split image folder: its file name sets the threshold, comparison method, delay and flags, and the loaded profile's defaults fill in the rest, also right after you change them. The file name next to the buttons is colored by what each part does, the way the split image folder reads it: threshold, pause, delay, loops, comparison method, flags and the start or reset keyword. Hover over a part to see what it does. Values that are ignored are orange, and listed below the boxes. The boxes under the buttons say what the image does and how it's compared, and mark the values that come from the profile. The tester takes one image at a time, so the images of a [grouped split](#several-images-for-one-split) are tested one by one.

Live, Highest and Threshold work like in the similarity viewer. The bar under Live fills up to the similarity. Its green end is where the similarity reaches the threshold, marked with a black line, and Live turns green while it's there. That's often a single frame, so an orange mark on the bar shows the highest similarity, and Highest keeps a green dot once it has reached the threshold, until you press Start Over. The line next to them says what the tester waits for.

The right side keeps the frames that would have made the image act, decided like a run decides:

- A plain image keeps the first frame that reaches the threshold.
- With a delay, it also keeps the frame when the delay is over, which is when the split happens. The delay runs out even when the capture is lost.
- With `{b}`, it keeps the frame that reached the threshold and the frame that dropped back below it, plus the frame after the delay if there is one.
- A dummy image (`{d}`) acts right away, without its delay, like in a run.
- A reset image keeps the frame that reached the threshold. Delays and `{b}` don't apply to reset images.

`<` and `>` above the frames go through them. After that, nothing more is kept until you press Start Over, which also clears Highest. The tester uses the capture device, capture region and FPS limit from the main window and the settings, and works during a run too.

### Make Split Images

File > Make Split Images (Ctrl+M) makes split images from a preset: a list of a category's splits, each with a mask that keeps only the part of the screen that tells the split apart, and the threshold, pause, delay and flags it needs. Presets come with `autosplit`, in the `presets` folder next to it. Pick the game and the preset at the top. Like the Live Tester, it opens beside the main window, which can't be used while it's open apart from the Help menu and File > Settings (also Ctrl+, or the Settings button in the main window), so set the capture region first. The split images are saved into the profile's split image folder, which Browse... next to it changes.

The list shows each split's name over its file name, colored like in the Live Tester, and what its image is made from. The box on the left shows the grab or screenshot, unmasked, and stays empty until there is one. The file names use the profile's defaults for what the preset leaves out, and are numbered from 0 like AutoSplit reads them, with as many digits as the last number needs, so `00_` with 10 to 99 numbers. There are two ways to make the images:

- Grab takes the capture region now, masks it for the selected split, saves it into the split image folder and selects the next split. The screenshot hotkey does the same while the window is open, so you can play through the category and press it at each split. Until a split has an image, the Output Preview shows the capture masked as it would be saved.
- Use Screenshots... uses screenshot files instead, like the ones Take Screenshot saves. Pick one for the selected split, or several for the splits in order from the first. Save Selected and Save All then save them.

Saving asks before replacing split images, and before leaving older images of the same splits under other names, for example after a threshold changed, since the split image folder would use those too. Delete Old Images removes them, Keep Old Images keeps them. The main window reads the split image folder again once Make Split Images closes, not while it's saving split images one at a time. A green check mark over the Output Preview shows when they're saved. Screenshots that were in the split image folder move into its `base` folder, where they aren't read as split images. Test opens the selected split's saved image in the Live Tester. What you grabbed or picked is kept, in memory until `autosplit` closes, when you close the window and open it again on the same preset, so Save All can save it again into another split image folder. Splits are only marked saved while their split image is in the split image folder.

Click the Output Preview, or Transparency grid, to show what the mask removes as a checkerboard.

Generate Profile... saves the settings as a new profile named for the game and preset, for example `tlozoot_no_wrong_warp.toml` for The Legend of Zelda: Ocarina of Time's No Wrong Warp, with the split image folder in it. Then it asks whether to load it, so the main window uses it from now on like after Load Profile, or to keep the current profile loaded. For a preset that compares several images for one split, the generated profile has Compare images with the same number for one split turned on, which the folder needs. Until that's on, the line at the bottom of the window says so, with a Turn It On button.

A preset can have something the editor wouldn't save, like a preset written by hand where splits compared together have the same name, which would give their savestate copies the same name. Then the line at the bottom of the window says what to fix, the split's name in the list is red, and Grab, Use Screenshots..., saving, Test, Copy Savestates and Generate Profile... wait until it's fixed. Fix It opens the preset in the editor.

Some presets come with savestates for a practice tool that loads savestates on the console, like gz for Ocarina of Time (`.gzs` files), to get to each split's screen for its screenshot. Emulators don't need them, since they can auto split by reading the game's memory. Copy Savestates puts them on the clipboard, numbered like the split images, to paste where the practice tool loads them from. Don't copy them out of the preset's `savestates` folder yourself, as its `DO_NOT_COPY.txt` says: presets use some savestates for several splits, and the file names there aren't in split order. The copies are in the `AutoSplit savestates` folder in the temp folder, which the next Copy Savestates empties first. Savestate Instructions shows what to do with the savestates that need more than loading, under the names Copy Savestates gives them.

#### Editing presets

Edit, next to the preset, switches the window to editing it, and New starts a new preset for the same game. The game becomes a list of the games that have presets, with New game... to type another, and the preset's name a field. A game typed in another case or with other spaces than one that has presets is saved as that one. The buttons under the list add, duplicate, remove and move splits. Alt+Up and Alt+Down move the selected split too, Ctrl+Z brings back removed splits with their grabs or screenshots until something else is changed, Ctrl+S saves, or lists what to fix first, F8 goes to the next split with something to fix, and Escape cancels. The right side edits the selected split:

- Type: Split, Start Image or Reset Image. AutoSplit tells them apart by name, so Start Image and Reset Image name the split `start_auto_splitter` or `reset`, and the Name field can't be changed then. Typing one of those names makes it one too. A preset can have up to 4 of each, any of which matches, and the second one on is named `reset_2`, `reset_3` and so on.
- Name: the part of the file name between the number and the settings. It can't have `reset` or `start_auto_splitter` anywhere in it, like `Preset Boss`, since AutoSplit reads those as a reset or start image wherever they are, or `#@{}()[]^`, which AutoSplit reads as settings. The list shows the file name it makes.
- Mask: a PNG that's opaque where the screen is compared. The Output Preview shows it applied to the capture while you edit, or to the split's grab or screenshot when it has one, so you can see what it keeps.
- Threshold, Delay, Pause, Dummy and Split below threshold: what the file name carries. Unchecked, Threshold, Delay and Pause use the profile's default, which the value shows, and leave it out of the file name. Changing the value checks it.
- Same split as the one above: the split is compared for the same split as the split above it, and gets its number, so either one matching splits. Splits compared together need their own names, or their savestates would be copied with the same name. See [Several images for one split](#several-images-for-one-split). Up to 4 splits can be together. The list shows them as one block, with a bar down its left side and no line between them. Reset and start images can't be joined. Splits that are together stay together: Add puts the new split after them, Duplicate puts the copy after them (a copy of the first is a split of its own, a copy of another joins them), and removing the first leaves the rest together. Moving a split up or down moves it among them, and from the first or last one, moves them all past the split above or below. Only the checkbox makes a split join or leave them.
- Savestate and its instructions, for Copy Savestates and Savestate Instructions.

Splits with something to fix have a red name, and the line at the bottom says what. Save writes the preset into its folder in `presets`. Masks and savestates picked from elsewhere are copied in, masks named after their split, and savestates and masks the preset doesn't use anymore are deleted. Save as New, for a preset saved before, saves the changes as a new preset and leaves the old one as it was. Cancel, closing the window or quitting asks before throwing changes away. Delete Preset deletes the preset's folder. Grabs and screenshots stay with their splits while you add, remove and move them, and grabbing waits until editing ends.

## Settings

### Capture settings

- Comparison FPS Limit: how often the capture is compared, 20 to 240 times a second. Set it near what the game runs at. More only costs CPU.
- Live Capture Region: show the live preview.
- Show Pause Timer: show the seconds left of a pause or a split delay next to Current Image.
- Capture device: the device to capture from. `autosplit` finds it by name, so it keeps working when devices are added or removed. A profile without one gets OBS Virtual Camera when it's connected, or else the first device in the list. The Refresh button lists the devices again.
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
- Default Pause Time: how long to stop comparing after a split, in seconds. It stops two identical images in a row from splitting at once, and saves CPU.
- Loop Last Split Image to First Split Image: after the last image splits, start over with the first one. Otherwise comparing stops.
- Start also Resets: when the start image matches, reset the timer too. The timer gets a reset and then a start: with hotkeys, the reset hotkey and then the split hotkey, like AutoSplit 2.3.4.
- Enable auto reset image: use the reset image. The toggle auto reset image hotkey turns it on and off, for example when a mistake in the run would show the reset image.
- Compare images with the same number for one split: split images that start with the same number count as one split, and any of them can match it. See [Several images for one split](#several-images-for-one-split). AutoSplit doesn't have this setting.
- Show a split's images in a 2x2 grid: with the setting above on, a split with several images shows all of them in the current image area, and `None` in the tiles without an image.

### Hotkeys

The split, reset, undo, skip and pause hotkeys must match your timer's, with global hotkeys turned on in the timer. `autosplit` presses them and also listens for them. Pressing the split hotkey starts the run, reset stops it, and undo and skip move along with the timer. The pause hotkey is only pressed for images with the `{p}` flag. The screenshot hotkey takes a screenshot like Take Screenshot, and the toggle auto reset image hotkey turns the reset image on and off.

To set one, click Set Hotkey and press the key, with any modifiers. Escape clears it, and clicking Set Hotkey again cancels. You can also type a name in the field and press Enter, like `num 1`, `ctrl+shift+f5`, `page up` or `plus`. A hotkey can only be used once, so setting it for one action clears it from another.

![The Hotkeys tab with hotkeys set and one Set Hotkey button waiting for a key](docs/images/settings-hotkeys.png)

Keypad keys are named `num 0` to `num 9`, `decimal`, `plus`, `-`, `*` and `/`, whether Num Lock is on or not. Set Hotkey names the key without what Shift types on it, so Shift+1 is `shift+1`.

When LiveSplit or LiveSplit One controls the timer, only the screenshot and toggle auto reset image hotkeys are used. With Molok, `autosplit` doesn't press any hotkeys, but still listens for them, so set them to the keys you control Molok with if you want `autosplit` to follow along.

### Other

- Timer: Any timer (hotkeys), LiveSplit One (WebSocket), or Molok (POSIX signals, listed on Linux only). The change applies right away, or after the current run ends.
- LiveSplit One port: the port LiveSplit One connects to, 16835 by default. Change it if another program uses that port. A typed port is used once you press Enter or leave the box.

## Split image file names

Each image can set its own values in its file name. They override the defaults from the settings window.

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

Values need their closing character, so `(0.9` sets nothing.

### Several images for one split

Some splits can look different from run to run, like a boss that dies in one of two rooms. With Compare images with the same number for one split turned on, `autosplit` compares up to 4 images for the same split, and the first one that matches splits:

- `01_boss_left.png` and `01_boss_right_(0.85).png`: split 1 matches either room.
- `02_door.png`: split 2.

Every split image needs a number at the start of its name, or the folder won't load. Leading zeros don't matter, so `1_a.png` and `01_b.png` are the same split. Splits run in number order, and the images of a split in name order. Start and reset images need no number.

With the setting on, a folder can also have up to 4 start images and up to 4 reset images, like `reset.png` and `reset_2.png`, and any of them matching starts or resets the run. Without it, a folder with more than one is refused, like in AutoSplit.

Each image keeps its own threshold, comparison method and `{b}` flag. The image that matched decides the rest: its delay, pause time and `{d}` or `{p}` flag. The images of a split need the same loop count. Above the current image, the file name becomes "Comparing against 2 images" (or 3, or 4). The current image area shows the split's first image, or all of them in a 2x2 grid with Show a split's images in a 2x2 grid turned on. The Similarity Viewer gets a row for each image, Current Image (1) to (4), with its own live, highest and threshold values. Several start images show the same way while `autosplit` looks for them. For several reset images, the Reset Image row shows the best similarity, with that image's threshold.

### Start image

One image can have `start_auto_splitter` in its name, as a whole word, or up to 4 with [several images for one split](#several-images-for-one-split) turned on. Before the run, `autosplit` looks for it and starts the timer when it matches. Its pause time is the wait before comparing the first split image, and its delay is the wait before starting the timer. With several, the one that matched decides both. After you change it, Reload Split Images loads it again.

### Reset image

One image can have `reset` in its name, as a whole word, so `preset_menu.png` is a normal split image. With [several images for one split](#several-images-for-one-split) turned on, up to 4 can. During the run, `autosplit` resets the timer when one matches. They aren't splits. A reset image's pause time is how long after the start it waits before looking for it, as in `reset_(0.95)_[10].png`. With several, each waits for its own.

### Masked images

When only part of the capture is always the same, like text over a changing background, make everything else transparent. Take a screenshot of the whole capture region and erase the rest in an editor like paint.net or GIMP. Only the parts that aren't transparent are compared. Use L2 Norm or Histograms for these images, not pHash.

![A split image before and after masking: the full screenshot, then only the text box left and the rest transparent](docs/images/masked-image.png)

## Profiles

A profile (`.toml`) has all the settings, including the capture region. File > Save Profile saves to the loaded profile, File > Save Profile As to a new file, and File > Load Profile opens one. Profiles from AutoSplit load as they are, and profiles saved here load in AutoSplit. Closing the window with unsaved changes asks whether to save them.

### Making a profile by hand

AutoSplit has no macOS version, so on a Mac it can be easier to write a profile in a text editor. Keys you leave out get the settings window's defaults.

```toml
split_hotkey = "num 1"
reset_hotkey = "num 3"
undo_split_hotkey = "num 8"
skip_split_hotkey = "num 2"
capture_method = "VIDEO_CAPTURE_DEVICE"
capture_device_name = "OBS Virtual Camera"  # as `autosplit devices` shows it
split_image_directory = "/Users/me/Splits/OoT Any%"  # on Windows, use / or 'single quotes'
default_similarity_threshold = 0.95
timer_connection = "livesplit_one"  # optional, see LiveSplit One and Molok
group_split_images = true  # optional, see Several images for one split
split_image_grid = true  # optional, shows those images in a 2x2 grid

[capture_region]  # in pixels of the device's frames; `autosplit grab` shows their size
x = 0
y = 0
width = 1920
height = 1080
```

Keys after `[capture_region]` belong to the capture region and are ignored, so put new ones above it. The four hotkeys aren't needed with `timer_connection = "livesplit_one"` or `"molok"`. Relative folders are read from the profile's folder.

### Using a profile from another computer

- Change the split image folder to where the images are now.
- Device names can differ between systems. Pick the device again in Settings.
- If the device's frames have another size there, change the capture region to match.

## LiveSplit One

`autosplit` can control LiveSplit One, the web version or the desktop app, without hotkeys. Choose LiveSplit One (WebSocket) in Settings > Other, and `autosplit` starts listening right away. Then choose "Connect to Server" in LiveSplit One's menu and enter `ws://localhost:16835`, or the port you set.

![LiveSplit One's Connect to Server dialog with ws://localhost:16835 entered](docs/images/livesplit-one-connect.png)

- `autosplit` sends starts, splits, resets and pauses (images with the `{p}` flag). Resets save the attempt without asking, like LiveSplit does for AutoSplit.
- Starting, splitting, skipping, undoing and resetting in LiveSplit One moves `autosplit` along too.
- Start `autosplit` first. LiveSplit One doesn't reconnect by itself, so after restarting `autosplit` or changing the port or the connection, choose Connect to Server again.
- LiveSplit One must run on the same computer. Only LiveSplit One can connect, not other web pages.
- Only one LiveSplit One can be connected. A new connection replaces the older one.
- If the browser refuses to connect (Safari has blocked this in the past), use Chrome, Edge or Firefox, or the LiveSplit One desktop app. Chrome and Edge may ask whether one.livesplit.org may access devices on your network. Allow it.

AutoSplit ignores these settings, so the profile keeps working with it.

## Molok

Molok is a Linux timer controlled with POSIX signals. Choose Molok (POSIX signals) in Settings > Other, and `autosplit` sends signals to every running process named `molok`:

| Command | Signal |
| --- | --- |
| Start and split | `SIGRTMIN+0` |
| Skip split | `SIGRTMIN+1` |
| Undo split | `SIGRTMIN+2` |
| Reset | `SIGRTMIN+3` |
| Pause (images with the `{p}` flag) | `SIGRTMIN+4` |

- Molok doesn't report back. When you split, reset or undo in Molok yourself, `autosplit` only follows if you used the same keys as its hotkeys.
- Start the run with the start image, Start Auto Splitter or the split hotkey. Start Auto Splitter doesn't start Molok.
- Molok must run as your user, or `autosplit` isn't allowed to signal it. If Molok isn't running, the log says so.
- With Start also Resets, the start signal goes 5 milliseconds after the reset. Molok would otherwise get both at once and handle the start first, since Linux delivers the lower signal number first.

## Permissions

- Linux:
  - Global hotkeys need to read the keyboards: `sudo usermod -a -G input $USER`, then log out and back in. Any program you run can then read your keystrokes.
  - Pressing hotkeys needs `/dev/uinput`: `echo 'KERNEL=="uinput", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/50-uinput.rules`, then restart. If `/dev/uinput` doesn't exist, load it with `sudo modprobe uinput` (and `echo uinput | sudo tee /etc/modules-load.d/uinput.conf` to keep it after a restart). Under WSL, key presses can't be sent.
  - Capture devices need the `video` group, which desktop users usually have.
  - The window needs fontconfig and xkbcommon, which every desktop has.
  - Without these, `autosplit` still runs and says what's missing. The LiveSplit One and Molok connections need none of them.
- macOS: in System Settings > Privacy & Security, allow `autosplit` under Camera for capture devices, Accessibility for pressing hotkeys and Input Monitoring for listening to them, then restart it. When you start it from a terminal, allow the terminal app instead, like Terminal or iTerm.

## From a terminal

Everything also works without the window. In the commands below, type `.\autosplit` on Windows and `./autosplit` on macOS and Linux. Put paths with spaces in "quotes".

1. Run `autosplit devices` and note your capture device's name.
2. Run `autosplit grab --device "<name>" frame.png` and check that `frame.png` shows the game. Add `--profile settings.toml` to save only the capture region, which is exactly what gets compared.
3. Run `autosplit check settings.toml` to check the split images.
4. Run `autosplit run settings.toml`. It prints what happens, with the current split image and its similarity on the last line. Press Ctrl+C to stop. `--split-dir <folder>` uses another split image folder.

Without a capture device, `run` keeps going, shows `NO DEVICE` and looks for the device every 2 seconds.

All the commands, which `autosplit --help` also lists:

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

The Windows zip has two programs. `AutoSplit.exe` opens the window without a console, and `AutoSplit.com` is for terminals. Typing `autosplit` in a terminal runs `AutoSplit.com`, which runs `AutoSplit.exe` inside the terminal so output and Ctrl+C work. Without a command, it opens the window and returns to the prompt.
