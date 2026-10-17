# Performance

Cost of one comparison, including the resize from a 1920x1080 capture. Median of 5 runs, each a process pinned to one core of an AMD Ryzen 7 7800X3D. Toufool's AutoSplit is its own Python code on OpenCV 5.0, limited to one thread.

| Split image | Method | AutoSplit-rs | Toufool's AutoSplit |
| --- | --- | --- | --- |
| Opaque (compared at 320x240) | L2 norm | 69 us | 66 us |
| Opaque | Histograms | 67 us | 186 us |
| Opaque | pHash | 60 us | 174 us |
| Masked, large visible area (compared at 887x499) | L2 norm | 72 us | 514 us |
| Masked, large visible area | Histograms | 71 us | 879 us |
| Masked, large visible area | pHash | 363 us | 3090 us |
| Masked, small visible area (real AutoMask reset image) | L2 norm | 3.7 us | 159 us |
| Masked, small visible area | Histograms | 5.0 us | 276 us |
| Masked, small visible area | pHash | 135 us | 1132 us |

Opaque L2 is the one case where OpenCV wins. Its hand-written SSE2/AVX2 code beats what LLVM generates from safe Rust. The rest is faster because masked images only resize and compare their visible pixels, nothing is allocated per frame, histograms and hashes are computed once at load, and a split image and reset image of the same size share one resize. Run `autosplit bench` to measure on your machine.

Converting a whole 1080p frame to BGRA takes 0.26 to 0.64 ms, more than most comparisons above. Same CPU and method, using AVX2, with the areas cut from the middle of the frame. Raw copy is the cost of keeping a frame to convert later. The test frame fits in this CPU's 96 MB L3 cache, so a frame fresh from the device can take longer.

| Format | Whole frame | 640x360 | 320x240 | Raw copy |
| --- | --- | --- | --- | --- |
| NV12 | 568 us | 68 us | 21 us | 98 us |
| YUYV | 644 us | 76 us | 24 us | 131 us |
| BGR24, bottom-up | 256 us | 30 us | 9.2 us | 196 us |

That's why AutoSplit-rs avoids converting frames:

- It converts only the capture region, with AVX2 or SSE4.1 when the CPU has them. MJPEG frames are the exception, since libjpeg-turbo decodes them whole.
- It converts only while something compares. During pauses, delays and while waiting for the split hotkey, it copies the frame instead when that's cheaper, and converts the newest one when needed.
- It never compares the same frame twice. Ticks at the FPS limit often outrun the device, like 60 FPS against a 30 FPS OBS Virtual Camera. Toufool's AutoSplit compares the repeat frame and gets the same result, so splits land at the same time either way.
