#!/bin/sh
# Adds AutoSplit to the desktop's app menu, with its icon, for the folder this script is in.
# Run it again after moving the folder. With --remove, takes it out of the menu again.
set -eu

app_id=io.github.torjeamundsen.autosplit
applications="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
entry="$applications/$app_id.desktop"

if [ "${1-}" = "--remove" ]; then
    rm -f "$entry"
    echo "Removed AutoSplit from the app menu."
    exit 0
fi

folder=$(cd "$(dirname "$0")" && pwd -P)
if [ ! -x "$folder/autosplit" ]; then
    echo "There's no autosplit next to this script, in $folder." >&2
    exit 1
fi

# Exec's quoting puts a backslash before a backslash, double quote, backtick or dollar sign,
# then every backslash is doubled like in any value, and % is doubled
exec_path=$(printf '%s' "$folder/autosplit" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g' -e 's/%/%%/g')
icon_path=$(printf '%s' "$folder/icon.png" | sed -e 's/\\/\\\\/g')

mkdir -p "$applications"
# The file name and StartupWMClass match the app id autosplit gives its windows, so the desktop
# shows the icon on them too
cat > "$entry" << ENTRY
[Desktop Entry]
Type=Application
Name=AutoSplit
Comment=Image comparison auto splitter for speedrunning
Exec="$exec_path"
Icon=$icon_path
Terminal=false
Categories=Game;Utility;
StartupWMClass=$app_id
ENTRY

# Some desktops only notice new entries once their cache is updated
if command -v update-desktop-database > /dev/null 2>&1; then
    update-desktop-database "$applications" > /dev/null 2>&1 || true
fi
echo "Added AutoSplit to the app menu. Run this again if you move the folder."
