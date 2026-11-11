"""
Times Toufool's AutoSplit comparing split images, for the table in PERFORMANCE.md.

Prints the same rows as `autosplit bench`: one comparison, including the resize of the capture,
through AutoSplit's own `AutoSplitImage.compare_with_capture`. Only `error_messages` is stubbed,
because it pulls in PySide6. OpenCV is limited to one thread.

Usage (from the repository root):
    uv venv -p 3.14 .bench-venv
    VIRTUAL_ENV=.bench-venv uv pip install opencv-contrib-python-headless==5.0.0.93 "numpy>=2.3.3,<2.6" "Levenshtein>=0.27.5" tomli-w
    .bench-venv/bin/python tools/bench/toufool.py --autosplit-src ../AutoSplit/src <split images...>

OpenCV matches AutoSplit's uv.lock. AutoSplit's compare.py imports Levenshtein, so it's needed
to import it. On Windows its utils.py imports pywin32 too: add `pywin32` to the install.
"""

from __future__ import annotations

import argparse
import sys
import time
import types
from pathlib import Path

import cv2
import numpy as np

METHODS = ("L2 Norm", "Histograms", "pHash")


def import_autosplit_image(autosplit_src: Path):
    class ErrorMessagesStub(types.ModuleType):
        def __getattr__(self, name: str):
            return lambda *_args, **_kwargs: None

    sys.modules["error_messages"] = ErrorMessagesStub("error_messages")
    # Written by AutoSplit's build script, so a fresh checkout doesn't have it
    build_vars = types.ModuleType("gen.build_vars")
    build_vars.AUTOSPLIT_BUILD_NUMBER = ""
    build_vars.AUTOSPLIT_GITHUB_REPOSITORY = ""
    sys.modules.setdefault("gen", types.ModuleType("gen"))
    sys.modules.setdefault("gen.build_vars", build_vars)
    sys.path.insert(0, str(autosplit_src))
    import AutoSplitImage  # noqa: PLC0415

    return AutoSplitImage.AutoSplitImage


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("split_images", nargs="+", type=Path)
    parser.add_argument("--autosplit-src", required=True, type=Path)
    parser.add_argument("--capture", default="1920x1080")
    parser.add_argument("--iterations", default=2000, type=int)
    args = parser.parse_args()

    cv2.setNumThreads(1)
    width, height = (int(side) for side in args.capture.split("x"))
    # The capture `autosplit bench` compares, without the alpha AutoSplit's captures don't have
    bgra = (np.arange(width * height * 4, dtype=np.uint64) * 7 % 251).astype(np.uint8)
    capture = np.ascontiguousarray(bgra.reshape(height, width, 4)[:, :, :3])

    AutoSplitImage = import_autosplit_image(args.autosplit_src)
    print(f"OpenCV {cv2.__version__}, capture {width}x{height}, {args.iterations} iterations each")
    for path in args.split_images:
        image = AutoSplitImage(str(path))
        for index, method in enumerate(METHODS):
            image.compare_with_capture(index, capture)
            started = time.perf_counter()
            for _ in range(args.iterations):
                image.compare_with_capture(index, capture)
            per_compare = (time.perf_counter() - started) / args.iterations
            print(f"{path.name:<32} {method:<12} {per_compare * 1e6:>9.1f} us")


if __name__ == "__main__":
    main()
