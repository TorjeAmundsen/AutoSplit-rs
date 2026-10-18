"""Checks that every gz savestate in presets/ and splits/ was saved with gz 0.3.7.

Usage: python tools/check_savestates.py [<repo folder>]

A .gzs file starts with gz's `struct state_meta`, big-endian like the N64: the game version (u16),
the state format version (u16) and the file size (u32). gz 0.3.7 saves format 6. Newer gz builds
save format 7, which 0.3.7 still loads, because it only rejects formats older than 3, but it reads
the state with the wrong layout and the game crashes. The size has to match the file, or gz
refuses to load it.
"""

import struct
import sys
from pathlib import Path

STATE_VERSION = 6
FOLDERS = ("presets", "splits")


def problem(path):
    """Why gz 0.3.7 can't load the savestate, or None if it can."""
    data = path.read_bytes()
    if len(data) < 8:
        return "too short to be a savestate"
    _, state_version, size = struct.unpack(">HHI", data[:8])
    if state_version != STATE_VERSION:
        return f"state format {state_version}, gz 0.3.7 saves format {STATE_VERSION}"
    if size != len(data):
        return f"header says {size} bytes, the file has {len(data)}"
    return None


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    paths = sorted(p for folder in FOLDERS for p in (root / folder).rglob("*.gzs"))
    bad = 0
    for path in paths:
        reason = problem(path)
        if reason:
            bad += 1
            print(f"{path.relative_to(root).as_posix()}: {reason}")
    if bad:
        print(f"{bad} of {len(paths)} savestates weren't saved with gz 0.3.7")
        sys.exit(1)
    print(f"All {len(paths)} savestates were saved with gz 0.3.7")


if __name__ == "__main__":
    main()
