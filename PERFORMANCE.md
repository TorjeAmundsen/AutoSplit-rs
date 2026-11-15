# Performance

Cost of one comparison, including the resize from a 1920x1080 capture. Median of 15 runs, each a process pinned to one core of an AMD Ryzen 7 7800X3D. Toufool's AutoSplit is its own Python code, from its main branch of 10 November 2026, on OpenCV 5.0 limited to one thread.

| Split image | Method | AutoSplit-rs | Toufool's AutoSplit |
| --- | --- | --- | --- |
| Opaque (compared at 320x240) | L2 norm | 42 us | 66 us |
| Opaque | Histograms | 67 us | 191 us |
| Opaque | pHash | 48 us | 173 us |
| Masked, large visible area (compared at 887x499) | L2 norm | 48 us | 519 us |
| Masked, large visible area | Histograms | 69 us | 885 us |
| Masked, large visible area | pHash | 82 us | 3083 us |
| Masked, small visible area (real AutoMask reset image) | L2 norm | 2.1 us | 159 us |
| Masked, small visible area | Histograms | 4.2 us | 276 us |
| Masked, small visible area | pHash | 13 us | 1138 us |

The images are `opaque_bgr_320x240.png` and `mask_binary_1280x720_(0.9).png` in `tests/fixtures/synthetic`, and `reset_(0.96)_[0.1].png` in `tests/fixtures/real`.

AutoSplit-rs is faster because masked images only resize and compare their visible pixels, nothing is allocated per frame, histograms and hashes are computed once at load, and a split image and reset image of the same size share one resize. The sums of the L2 norm, pHash's shrinking and fade images use AVX2 or SSE4.1 when the CPU has them. About 28 us of every opaque row is the resize of the capture.

pHash shrinks the image to 8x8 first. For a masked image that only reads the visible pixels too: a hidden pixel counts as 0, and adding 0 changes no sum.

Fade images, which Toufool's AutoSplit doesn't have, compare with the image's color. These are the images `autosplit bench` makes without arguments. The masked one shows 200x150 pixels of 480x360.

| Fade image | Ignore noise | AutoSplit-rs |
| --- | --- | --- |
| Opaque (compared at 320x240) | Off | 48 us |
| Opaque | On | 62 us |
| Masked | Off | 20 us |
| Masked | On | 30 us |

Run `autosplit bench` to measure on your machine, with split images as arguments to time your own. `tools/bench/toufool.py` times Toufool's AutoSplit the same way. Its instructions are at the top of the file.

Converting a whole 1080p frame to BGRA takes 0.26 to 0.64 ms, more than most comparisons above. Same CPU, median of 5 runs, using AVX2, with the areas cut from the middle of the frame. Raw copy is the cost of keeping a frame to convert later. The test frame fits in this CPU's 96 MB L3 cache, so a frame fresh from the device can take longer.

| Format | Whole frame | 640x360 | 320x240 | Raw copy |
| --- | --- | --- | --- | --- |
| NV12 | 568 us | 68 us | 21 us | 98 us |
| YUYV | 644 us | 76 us | 24 us | 131 us |
| BGR24, bottom-up | 256 us | 30 us | 9.2 us | 196 us |

That's why AutoSplit-rs avoids converting frames:

- It converts only the capture region, with AVX2 or SSE4.1 when the CPU has them. MJPEG frames are the exception, since libjpeg-turbo decodes them whole.
- It converts only while something compares. During pauses, delays and while waiting for the split hotkey, it copies the frame instead when that's cheaper, and converts the newest one when needed.
- It never compares the same frame twice. Ticks at the FPS limit often outrun the device, like 60 FPS against a 30 FPS OBS Virtual Camera. Toufool's AutoSplit compares the repeat frame and gets the same result, so splits land at the same time either way.
