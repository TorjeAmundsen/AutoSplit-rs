"""
Checks that `autosplit grab` gets the same pixels from a capture device as AutoSplit.

Run from AutoSplit's own virtual environment, so OpenCV and pygrabber are the versions
AutoSplit ships with. Show something still on the device first, like a paused OBS Virtual
Camera, since the two grabs happen one after the other.

    python capture_parity.py grab --autosplit-src ../AutoSplit/src --device 0 opencv.png
    autosplit grab --device 0 rust.png
    python capture_parity.py compare opencv.png rust.png
"""

from __future__ import annotations

import argparse
import sys
import time

import cv2
import numpy as np


def grab(autosplit_src: str, device: int, output: str, skip: int) -> None:
    sys.path.insert(0, autosplit_src)
    from capture_method.VideoCaptureDeviceCaptureMethod import is_blank  # noqa: PLC0415
    from utils import get_input_device_resolution  # noqa: PLC0415

    # Opened the way VideoCaptureDeviceCaptureMethod.__init__ does
    capture = cv2.VideoCapture(device)
    capture.setExceptionMode(True)
    resolution = get_input_device_resolution(device)
    if resolution is not None:
        try:
            capture.set(cv2.CAP_PROP_FRAME_WIDTH, resolution[0])
            capture.set(cv2.CAP_PROP_FRAME_HEIGHT, resolution[1])
        except cv2.error:
            pass

    frames = 0
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        result, image = capture.read()
        if not result or image is None or is_blank(image):
            continue
        frames += 1
        if frames > skip:
            cv2.imwrite(output, cv2.cvtColor(image, cv2.COLOR_BGR2BGRA))
            print(f"Saved a {image.shape[1]}x{image.shape[0]} frame from device {device}")
            return
    sys.exit("No frames within 10 seconds")


def compare(first: str, second: str) -> None:
    a = cv2.imread(first, cv2.IMREAD_UNCHANGED)
    b = cv2.imread(second, cv2.IMREAD_UNCHANGED)
    if a.shape != b.shape:
        sys.exit(f"Different sizes: {a.shape[1]}x{a.shape[0]} and {b.shape[1]}x{b.shape[0]}")
    difference = np.abs(a.astype(np.int16) - b.astype(np.int16))
    different = np.count_nonzero(difference.max(axis=2))
    if different == 0:
        print("Identical")
        return
    pixels = a.shape[0] * a.shape[1]
    print(f"{different} of {pixels} pixels differ, by up to {difference.max()}")
    sys.exit(1)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    grab_parser = commands.add_parser("grab", help="save a frame the way AutoSplit captures it")
    grab_parser.add_argument("--autosplit-src", required=True)
    grab_parser.add_argument("--device", type=int, required=True)
    grab_parser.add_argument("--skip", type=int, default=30, help="frames to skip while the device settles")
    grab_parser.add_argument("output")
    compare_parser = commands.add_parser("compare", help="compare two saved frames")
    compare_parser.add_argument("first")
    compare_parser.add_argument("second")
    args = parser.parse_args()
    if args.command == "grab":
        grab(args.autosplit_src, args.device, args.output, args.skip)
    else:
        compare(args.first, args.second)


if __name__ == "__main__":
    main()
