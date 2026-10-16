"""Converts AutoMask's presets and splits library to AutoSplit's TOML presets.

Usage: python tools/convert_presets.py <AutoMask folder>

<AutoMask folder> is the folder with AutoMask's `presets` and `splits` folders, like
`AutoSplit-AutoMask/AutoMask`. Writes `presets/` and `splits/` at the root of this repo: each
folder's files are copied, except the JSON, which becomes `preset.toml` or `splits.toml`.
The MST preset is left out: it's unfinished.
"""

import json
import shutil
import sys
from pathlib import Path

LEFT_OUT = {"MST"}
ROOT = Path(__file__).resolve().parent.parent


def string(text):
    """A TOML basic string."""
    escaped = text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")
    return f'"{escaped}"'


def number(value):
    """A TOML float, also for whole numbers, which JSON writes without a decimal point."""
    return repr(float(value))


def split_table(split, extra=()):
    """The [[split]] table of a preset or library split, with only the values it sets."""
    lines = ["[[split]]", f"name = {string(split['name'])}", f"mask = {string(split['mask'])}"]
    for key, toml_key in extra:
        if split.get(key):
            lines.append(f"{toml_key} = {string(split[key])}")
    if "threshold" in split:
        lines.append(f"threshold = {number(split['threshold'])}")
    if "pauseTime" in split:
        lines.append(f"pause = {number(split['pauseTime'])}")
    if split.get("delay"):
        lines.append(f"delay = {int(split['delay'])}")
    if split.get("dummy"):
        lines.append("dummy = true")
    if split.get("inverted"):
        lines.append("below = true")
    if split.get("savestate"):
        lines.append(f"savestate = {string(split['savestate'].replace(chr(92), '/'))}")
        if split.get("savestateInstructions"):
            lines.append(f"instructions = {string(split['savestateInstructions'])}")
    return "\n".join(lines)


def convert(source, target, name_key, file_name, extra):
    data = json.loads((source / f"{file_name}.json").read_text(encoding="utf-8-sig"))
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target, ignore=shutil.ignore_patterns("*.json"))
    header = []
    if name_key:
        header.append(f"name = {string(data[name_key])}")
    if data.get("gameName"):
        header.append(f"game = {string(data['gameName'])}")
    tables = [split_table(split, extra) for split in data["splits"]]
    text = "\n".join(header) + "\n\n" + "\n\n".join(tables) + "\n"
    (target / f"{file_name}.toml").write_text(text, encoding="utf-8", newline="\n")
    print(f"{target.relative_to(ROOT)}: {len(tables)} splits")


def main():
    automask = Path(sys.argv[1])
    for folder in sorted((automask / "presets").iterdir()):
        if folder.is_dir() and folder.name not in LEFT_OUT and (folder / "preset.json").exists():
            convert(folder, ROOT / "presets" / folder.name, "presetName", "preset", ())
    for folder in sorted((automask / "splits").iterdir()):
        if folder.is_dir() and (folder / "splits.json").exists():
            extra = (("description", "description"), ("baseImage", "base_image"))
            convert(folder, ROOT / "splits" / folder.name, None, "splits", extra)


if __name__ == "__main__":
    main()
