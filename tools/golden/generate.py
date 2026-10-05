"""
Generates the golden values the Rust test suite checks against.

Runs AutoSplit's own Python modules (compare.py, split_parser.py, AutoSplitImage.py) with
OpenCV, so the expected values come from the original implementation, not from a re-derivation.
Only `error_messages` is stubbed, because it pulls in PySide6.

Usage (from the repository root):
    uv venv -p 3.14 .golden-venv
    VIRTUAL_ENV=.golden-venv uv pip install opencv-contrib-python-headless==4.11.0.86 "numpy>=2.3.3,<2.6" "Levenshtein>=0.27.1,<0.28" tomli-w==1.2.0
    .golden-venv/bin/python tools/golden/generate.py --autosplit-src ../AutoSplit/src

OpenCV and tomli-w match AutoSplit's uv.lock. numpy is the nearest version with Python 3.14 wheels,
which doesn't affect the values. AutoSplit's compare.py imports Levenshtein, so it's needed to
import it. Regenerate only when deliberately re-baselining.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
import types
from pathlib import Path

import cv2
import numpy as np

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "tests" / "fixtures"
SYNTHETIC = FIXTURES / "synthetic"
REAL = FIXTURES / "real"
GOLDEN = REPO_ROOT / "tests" / "golden"


def import_autosplit(autosplit_src: Path):
    recorded_errors: list[tuple[str, tuple[object, ...]]] = []

    class ErrorMessagesStub(types.ModuleType):
        def __getattr__(self, name: str):
            return lambda *args, **_: recorded_errors.append((name, args))

    sys.modules["error_messages"] = ErrorMessagesStub("error_messages")
    sys.path.insert(0, str(autosplit_src))
    import AutoSplitImage  # noqa: PLC0415
    import compare  # noqa: PLC0415
    import split_parser  # noqa: PLC0415

    return AutoSplitImage, compare, split_parser, recorded_errors


def smooth_noise(rng: np.random.Generator, height: int, width: int, channels: int):
    """Low-frequency content with some noise, closer to game frames than white noise."""
    coarse = rng.integers(0, 256, (max(height // 16, 2), max(width // 16, 2), channels), np.uint8)
    image = cv2.resize(coarse, (width, height), interpolation=cv2.INTER_CUBIC)
    if image.ndim == 2:
        image = image[:, :, None]
    offsets = rng.integers(-3, 4, image.shape)
    return np.clip(image.astype(np.int16) + offsets, 0, 255).astype(np.uint8)


def shapes(rng: np.random.Generator, height: int, width: int):
    """Gradient plus flat shapes: game-like and small as PNG, unlike noise."""
    x = np.linspace(0, 255, width, dtype=np.float32)[None, :]
    y = np.linspace(0, 255, height, dtype=np.float32)[:, None]
    image = np.dstack([(x + y) / 2, np.broadcast_to(x, (height, width)), np.broadcast_to(y, (height, width))])
    image = np.ascontiguousarray(image.astype(np.uint8))
    for _ in range(40):
        color = [int(c) for c in rng.integers(0, 256, 3)]
        x0, y0 = int(rng.integers(0, width)), int(rng.integers(0, height))
        if rng.random() < 0.5:
            cv2.rectangle(image, (x0, y0), (x0 + int(rng.integers(5, width // 4)), y0 + int(rng.integers(5, height // 4))), color, -1)
        else:
            cv2.circle(image, (x0, y0), int(rng.integers(3, min(width, height) // 6)), color, -1)
    return image


def with_alpha(bgr: np.ndarray, alpha: np.ndarray):
    return np.dstack([bgr[:, :, :3], alpha]).astype(np.uint8)


def write(path: Path, image: np.ndarray):
    ok, encoded = cv2.imencode(path.suffix, image)
    assert ok, path
    path.write_bytes(encoded.tobytes())


def generate_synthetic_fixtures():
    rng = np.random.default_rng(20261005)
    SYNTHETIC.mkdir(parents=True, exist_ok=True)

    write(SYNTHETIC / "opaque_bgr_960x540.png", shapes(rng, 540, 960))
    write(SYNTHETIC / "opaque_bgr_320x240.png", smooth_noise(rng, 240, 320, 3))
    opaque = smooth_noise(rng, 217, 333, 3)
    write(SYNTHETIC / "opaque_bgra_333x217.png", with_alpha(opaque, np.full((217, 333), 255)))

    # Masks large enough that the adaptive resize scales down by a non-integer factor
    big = shapes(rng, 720, 1280)
    alpha = np.zeros((720, 1280), np.uint8)
    alpha[100:420, 200:700] = 255
    write(SYNTHETIC / "mask_binary_1280x720_(0.9).png", with_alpha(big, alpha))

    # Semi-transparent pixels: AutoSplit's L2 counts the alpha delta in the masked region
    partial = smooth_noise(rng, 360, 480, 3)
    alpha = rng.integers(0, 256, (360, 480), np.uint8)
    alpha[rng.random((360, 480)) < 0.3] = 0
    write(SYNTHETIC / "mask_partial_480x360.png", with_alpha(partial, alpha))


    # Masked images that stay at their own size, to hit pHash's INTER_AREA fast paths
    for size in (16, 24, 40):
        alpha = np.full((size, size), 255, np.uint8)
        alpha[: size // 4, : size // 4] = 0
        write(SYNTHETIC / f"mask_small_{size}x{size}.png", with_alpha(smooth_noise(rng, size, size, 3), alpha))

    # Smaller than pHash's 8x8 on at least one axis: INTER_AREA becomes a bilinear upscale
    for width, height in ((5, 4), (40, 4)):
        alpha = np.full((height, width), 255, np.uint8)
        alpha[0, 0] = 0
        write(SYNTHETIC / f"mask_tiny_{width}x{height}.png", with_alpha(smooth_noise(rng, height, width, 3), alpha))


    write(SYNTHETIC / "capture_bgra_1280x720.png", with_alpha(big, np.full((720, 1280), 255)))
    write(SYNTHETIC / "capture_alpha0_640x480.png", with_alpha(shapes(rng, 480, 640), np.zeros((480, 640))))
    write(SYNTHETIC / "capture_bgra_200x150.png", with_alpha(smooth_noise(rng, 150, 200, 3), np.full((150, 200), 255)))
    real_capture = cv2.imdecode(np.fromfile(REAL / "capture_001.png", np.uint8), cv2.IMREAD_UNCHANGED)
    noise = rng.integers(-20, 21, real_capture.shape)
    noise[:, :, 3] = 0
    noisy = np.clip(real_capture.astype(np.int16) + noise, 0, 255).astype(np.uint8)
    write(SYNTHETIC / "capture_noisy_001.png", noisy)


def load_capture(path: Path):
    """Captures are always BGRA, like every AutoSplit capture method returns."""
    image = cv2.imdecode(np.fromfile(path, np.uint8), cv2.IMREAD_UNCHANGED)
    if image.shape[2] == 3:
        image = cv2.cvtColor(image, cv2.COLOR_BGR2BGRA)
    return image


def sha256(array: np.ndarray):
    return hashlib.sha256(np.ascontiguousarray(array).tobytes()).hexdigest()


def rel(path: Path):
    return path.relative_to(REPO_ROOT).as_posix()


def split_image_files():
    return sorted(
        [*REAL.glob("*.png"), *SYNTHETIC.glob("*.png")],
        key=lambda p: p.as_posix(),
    )


def capture_files():
    return sorted(
        [p for p in [*REAL.glob("*.png"), *SYNTHETIC.glob("*.png")] if p.name.startswith("capture_")],
        key=lambda p: p.as_posix(),
    )


def golden_images(AutoSplitImage, recorded_errors):
    loaded = []
    for path in split_image_files():
        if path.name.startswith("capture_"):
            continue
        recorded_errors.clear()
        image = AutoSplitImage.AutoSplitImage(str(path))
        entry = {
            "path": rel(path),
            "image_type": image.image_type.name,
            "flags": image.flags,
            "loops": image.loops,
            "errors": [name for name, _ in recorded_errors],
        }
        if image.byte_array is not None:
            entry |= {
                "width": int(image.byte_array.shape[1]),
                "height": int(image.byte_array.shape[0]),
                "has_mask": image.mask is not None,
                "pixels_sha256": sha256(image.byte_array),
                "mask_sha256": sha256(image.mask) if image.mask is not None else None,
            }
        loaded.append((path, image, entry))
    return loaded


def golden_comparisons(loaded, compare):
    comparisons = []
    for capture_path in capture_files():
        capture = load_capture(capture_path)
        for split_path, image, _ in loaded:
            if image.byte_array is None:
                continue
            resized = cv2.resize(capture, image.byte_array.shape[1::-1], interpolation=cv2.INTER_NEAREST)
            row = {
                "split": rel(split_path),
                "capture": rel(capture_path),
                "resized_capture_sha256": sha256(resized),
            }
            for method in (0, 1, 2):
                row[f"method_{method}"] = float(image.compare_with_capture(method, capture))
            comparisons.append(row)
    return comparisons


def golden_phash(compare):
    """
    Raw pHash hashes of random inputs, to measure how often the DCT port disagrees with cv2.
    Inputs are 8x8 like AutoSplit feeds them, plus smooth and flat cases.
    """
    rng = np.random.default_rng(64)
    phash = cv2.img_hash.PHash.create()
    cases = []
    for index in range(400):
        if index < 300:
            image = rng.integers(0, 256, (8, 8, 4), np.uint8)
        elif index < 390:
            image = smooth_noise(rng, 8, 8, 4)
        else:
            image = np.full((8, 8, 4), rng.integers(0, 256), np.uint8)
        cases.append({
            "input": image.reshape(-1).tolist(),
            "upscaled_sha256": sha256(cv2.resize(image, (32, 32), interpolation=cv2.INTER_LINEAR_EXACT)),
            "gray_sha256": sha256(
                cv2.cvtColor(cv2.resize(image, (32, 32), interpolation=cv2.INTER_LINEAR_EXACT), cv2.COLOR_BGR2GRAY)
            ),
            "hash": phash.compute(image).reshape(-1).tolist(),
        })
    return cases


def golden_area(loaded):
    """INTER_AREA down to 8x8 from every split image's comparison size."""
    rows = []
    for split_path, image, _ in loaded:
        if image.byte_array is None:
            continue
        rows.append({
            "split": rel(split_path),
            "area_8x8": cv2.resize(image.byte_array, (8, 8), interpolation=cv2.INTER_AREA).reshape(-1).tolist(),
        })
    return rows


FILENAMES = [
    "split.png",
    "01_(0.9).png",
    "01_(0.90)_[5]_#1000#_@3@_^2^_{b}.png",
    "x_(1.0001).png",
    "x_(-0.1).png",
    "x_(nan).png",
    "x_(0.5.png",
    "x_[inf].png",
    "x_[1e2].png",
    "x_[.5].png",
    "x_[5.].png",
    "x_#-5#.png",
    "x_#12.png",
    "x_#1.5#.png",
    "x_@0@.png",
    "x_@-3@.png",
    "x_^5^.png",
    "x_^+1^.png",
    "x_{d}.png",
    "x_{dp}.png",
    "x_{bp}.png",
    "x_{DM}.png",
    "x_{x}.png",
    "x_{}.png",
    "x_{d.png",
    "x_{b}_{d}.png",
    "reset_(0.96)_[0.1].png",
    "preset.png",
    "start_auto_splitter_reset.png",
    "x_(0.5)_(0.7).png",
]


def json_float(value: float | None):
    """JSON has no infinity or NaN, so those are written as Python's repr string."""
    return value if value is None or math.isfinite(value) else repr(value)


def golden_filenames(split_parser):
    return [
        {
            "filename": name,
            "threshold": json_float(split_parser.threshold_from_filename(name)),
            "pause": json_float(split_parser.pause_from_filename(name)),
            "delay": split_parser.delay_time_from_filename(name),
            "loops": split_parser.loop_from_filename(name),
            "comparison_method": split_parser.comparison_method_from_filename(name),
            "flags": split_parser.flags_from_filename(name),
        }
        for name in FILENAMES
    ]


def generate_profiles():
    """Profiles written with tomli_w, the way AutoSplit saves them."""
    import tomli_w  # noqa: PLC0415

    profiles = FIXTURES / "profiles"
    profiles.mkdir(parents=True, exist_ok=True)
    # Saved before the settings window was ever opened: screenshot_on holds checkbox booleans
    ui_defaults = {
        "split_hotkey": "",
        "reset_hotkey": "",
        "undo_split_hotkey": "",
        "skip_split_hotkey": "",
        "pause_hotkey": "",
        "screenshot_hotkey": "",
        "toggle_auto_reset_image_hotkey": "",
        "fps_limit": 60,
        "live_capture_region": True,
        "capture_method": "WINDOWS_GRAPHICS_CAPTURE",
        "capture_device_id": -1,
        "capture_device_name": "",
        "default_comparison_method": 0,
        "default_similarity_threshold": 0.9,
        "default_delay_time": 0,
        "default_pause_time": 10.0,
        "loop_splits": False,
        "start_also_resets": False,
        "enable_auto_reset": True,
        "split_image_directory": "",
        "screenshot_directory": "",
        "open_screenshot": True,
        "screenshot_on": [False, False, False, False, False, False],
        "captured_window_title": "",
        "capture_region": {"x": 0, "y": 0, "width": 0, "height": 0},
    }
    (profiles / "ui_defaults.toml").write_bytes(tomli_w.dumps(ui_defaults).encode())
    configured = ui_defaults | {
        "split_hotkey": "num 1",
        "reset_hotkey": "num 3",
        "undo_split_hotkey": "num 8",
        "skip_split_hotkey": "num 2",
        "pause_hotkey": "ctrl+shift+p",
        "fps_limit": 144,
        "capture_method": "VIDEO_CAPTURE_DEVICE",
        "capture_device_id": 2,
        "capture_device_name": "OBS Virtual Camera",
        "default_comparison_method": 2,
        "default_similarity_threshold": 0.96,
        "default_delay_time": 150,
        "default_pause_time": 2.5,
        "start_also_resets": True,
        "split_image_directory": "C:/Users/Runner/Splits/OoT Any%/",
        "screenshot_on": ["split", "reset"],
        "captured_window_title": "Windowed Projector (Source) - Game Capture",
        "capture_region": {"x": 12, "y": 34, "width": 640, "height": 480},
        "a_future_setting": "kept",
    }
    (profiles / "configured.toml").write_bytes(tomli_w.dumps(configured).encode())
    # Saved by an older AutoSplit, before some keys existed: the missing ones get defaults
    old = {key: configured[key] for key in ("split_hotkey", "fps_limit", "split_image_directory", "capture_region")}
    (profiles / "missing_keys.toml").write_bytes(tomli_w.dumps(old).encode())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--autosplit-src", type=Path, required=True)
    args = parser.parse_args()

    AutoSplitImage, compare, split_parser, recorded_errors = import_autosplit(args.autosplit_src.resolve())
    generate_synthetic_fixtures()
    generate_profiles()
    loaded = golden_images(AutoSplitImage, recorded_errors)

    GOLDEN.mkdir(parents=True, exist_ok=True)
    outputs = {
        "images.json": [entry for _, _, entry in loaded],
        "comparisons.json": golden_comparisons(loaded, compare),
        "phash.json": golden_phash(compare),
        "area.json": golden_area(loaded),
        "filenames.json": golden_filenames(split_parser),
    }
    for name, value in outputs.items():
        indent = None if name == "phash.json" else 1
        (GOLDEN / name).write_text(json.dumps(value, indent=indent, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"opencv {cv2.__version__}, wrote {', '.join(outputs)}")


if __name__ == "__main__":
    main()
