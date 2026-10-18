# AutoSplit rewrite

A Rust rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit), the image comparison auto splitter for speedrunning. It watches your game through a capture device and splits your timer when the picture matches your split images.

![The main window during a run, with the live capture on the left and the current split image on the right](docs/main-window.png)

## What it does

- Uses your AutoSplit split images and profiles as they are, and computes the same similarity values to the last bit.
- Runs on Windows, Linux and macOS 11 or newer.
- Captures from capture cards, webcams and OBS Virtual Camera. To split on a window or the screen, show it in OBS and use OBS Virtual Camera.
- Makes split images for you: pick your category's preset in Make Split Images, play through it and grab each split, and the masked, named split images are saved into your folder. A preset editor and a Live Tester, for checking an image against the capture, are built in too.
- Splits on fades to a color, like a fade to black between areas, without splitting on a cut to black like a console reset.
- Controls LiveSplit through its AutoSplit Integration component, LiveSplit One over a local connection, Molok through signals on Linux, or any timer through hotkeys.
- Uses little CPU, so it can run next to OBS on a weak machine. Most comparisons take well under a millisecond, see [PERFORMANCE.md](PERFORMANCE.md).

![Make Split Images with a preset's splits on the left and the selected split's Output Preview on the right](docs/make-split-images.png)

## Getting started

1. Download the zip for your system from the [releases](https://github.com/TorjeAmundsen/AutoSplit-rs/releases) and unzip it.
2. Start `autosplit` (`AutoSplit.exe` on Windows).
3. Follow [TUTORIAL.md](TUTORIAL.md), which sets up OBS, makes your split images and connects your timer, step by step. Help > View Help in the program opens it too.

## Documentation

- [TUTORIAL.md](TUTORIAL.md): setting up for your first run, step by step.
- [MANUAL.md](MANUAL.md): every window, setting and file name option, the timers, permissions and the command line.
- [COMPATIBILITY.md](COMPATIBILITY.md): what works so far, how it's tested against AutoSplit, and where it behaves differently on purpose.
- [PERFORMANCE.md](PERFORMANCE.md): comparison and capture costs, measured against AutoSplit.
- [DEVELOPMENT.md](DEVELOPMENT.md): building, release builds, CI and the code layout.

## License

GPL-3.0-only, like AutoSplit. Parts of the image processing are ports of OpenCV, and the window uses Slint under its GPL-3.0 license. See [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt). Both are built into the program: Help > About > Licenses (Help > Licenses and Links on macOS).
