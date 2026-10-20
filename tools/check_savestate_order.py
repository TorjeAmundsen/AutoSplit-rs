"""Checks that Ocarina of Time splits compared together are in the order gz lists their savestates.

Usage: python tools/check_savestate_order.py [<repo folder>]

This is for Ocarina of Time only, whose practice tool is gz. Presets and libraries of other games
are left out, since their practice tools can sort savestates another way.

Copy Savestates names each savestate after its split, like `05_ScrubsDummyHigh.gzs`, and splits
compared together share a number. gz's file browser sorts files ignoring case and compares
numbers as numbers, so within a group it lists them by name. When a preset or the split library
has them in another order, the savestates come up in gz in a different order than the splits in
Make Split Images. This lists every group, in presets/ and splits/, that's out of that order.
"""

import functools
import sys
import tomllib
from pathlib import Path

KEYWORDS = {"reset", "start_auto_splitter"}
# The game whose practice tool is gz, as presets name it
GAME = "The Legend of Zelda: Ocarina of Time"


def gz_order(a, b):
    """gz 0.3.7's dir_entry_comp for two files: digits as numbers, letters ignoring case, then
    the shorter first. Negative when `a` comes first."""
    i = j = 0
    while i < len(a) and j < len(b):
        if a[i].isdigit() and b[j].isdigit():
            start_a, start_b = i, j
            while i < len(a) and a[i].isdigit():
                i += 1
            while j < len(b) and b[j].isdigit():
                j += 1
            number_a, number_b = a[start_a:i].lstrip("0"), b[start_b:j].lstrip("0")
            if len(number_a) != len(number_b):
                return len(number_a) - len(number_b)
            if number_a != number_b:
                return -1 if number_a < number_b else 1
            continue
        # Only ASCII letters, like gz
        char_a = a[i].upper() if "a" <= a[i] <= "z" else a[i]
        char_b = b[j].upper() if "a" <= b[j] <= "z" else b[j]
        if char_a != char_b:
            return -1 if char_a < char_b else 1
        i += 1
        j += 1
    return len(a) - len(b)


def copy_name(split):
    """The name Copy Savestates gives the split's savestate, but its number, which a group shares."""
    see = "_SEE_INSTRUCTIONS" if split.get("instructions") else ""
    extension = Path(split.get("savestate", "")).suffix
    return f"{split['name']}{see}{extension}"


def groups(splits):
    """The places of the splits compared together, like the app groups them: a split with
    `with_previous` joins the one before, unless either is a reset or start image."""
    group = []
    for index, split in enumerate(splits):
        joins = (
            group
            and split.get("with_previous")
            and split["name"] not in KEYWORDS
            and splits[index - 1]["name"] not in KEYWORDS
        )
        if group and not joins:
            yield group
            group = []
        group.append(index)
    if group:
        yield group


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    files = sorted([*root.glob("presets/*/preset.toml"), *root.glob("splits/*/splits.toml")])
    out_of_order = 0
    for path in files:
        loaded = tomllib.loads(path.read_text(encoding="utf-8"))
        if loaded.get("game", "").strip().casefold() != GAME.casefold():
            continue
        splits = loaded.get("split", [])
        for group in groups(splits):
            with_savestates = [index for index in group if splits[index].get("savestate")]
            if len(with_savestates) < 2:
                continue
            names = [splits[index]["name"] for index in with_savestates]
            copies = [copy_name(splits[index]) for index in with_savestates]
            wanted = sorted(copies, key=functools.cmp_to_key(gz_order))
            if copies != wanted:
                out_of_order += 1
                in_gz = [names[copies.index(copy)] for copy in wanted]
                print(f"{path.relative_to(root).as_posix()}: {', '.join(names)} should be {', '.join(in_gz)}")
    if out_of_order:
        print(f"{out_of_order} groups aren't in the order gz lists their savestates")
        sys.exit(1)
    print("Every group is in the order gz lists its savestates")


if __name__ == "__main__":
    main()
