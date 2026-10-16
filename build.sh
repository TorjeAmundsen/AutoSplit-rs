#!/usr/bin/env bash
# Builds release zips of AutoSplit on Linux and macOS, like build.ps1 does with PowerShell.
#
#   ./build.sh                     pick a target from a menu
#   ./build.sh linux-x64 ...       build these targets
#   ./build.sh --all               build every target this machine can build
#   ./build.sh --debug ...         keep debug symbols
#
# Same OS as this machine: plain cargo. Linux from another OS or architecture: cargo-zigbuild.
# Windows: cargo-xwin. macOS only builds on a Mac, because Apple's SDK is only licensed there.
# Written for the bash 3.2 macOS ships, so it has no associative arrays.

set -euo pipefail

root=$(cd "$(dirname "$0")" && pwd -P)
names="windows-x64 windows-arm64 linux-x64 linux-arm64 macos-universal"

# Oldest glibc Linux builds support when cross-compiled, matching the Ubuntu 22.04 CI runners
glibc_version=2.35

# The Rust targets of a zip
triples_of() {
    case $1 in
        windows-x64) echo x86_64-pc-windows-msvc ;;
        windows-arm64) echo aarch64-pc-windows-msvc ;;
        linux-x64) echo x86_64-unknown-linux-gnu ;;
        linux-arm64) echo aarch64-unknown-linux-gnu ;;
        macos-universal) echo aarch64-apple-darwin x86_64-apple-darwin ;;
        *) return 1 ;;
    esac
}

fail() {
    echo "error: $*" >&2
    exit 1
}

case $(uname -s) in
    Linux) host_os=linux ;;
    Darwin) host_os=macos ;;
    *) fail "build.sh is for Linux and macOS. On Windows, use build.ps1." ;;
esac
case $(uname -m) in
    arm64 | aarch64) host_arch=aarch64 ;;
    *) host_arch=x86_64 ;;
esac

version=$(awk '
    /^\[workspace\.package\]/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
' "$root/Cargo.toml")
[ -n "$version" ] || fail "Couldn't find [workspace.package] version in Cargo.toml"

os_of() {
    case $1 in
        *-windows-*) echo windows ;;
        *-apple-*) echo macos ;;
        *) echo linux ;;
    esac
}

can_build() {
    local triple
    for triple in $(triples_of "$1"); do
        if [ "$(os_of "$triple")" = macos ] && [ "$host_os" != macos ]; then
            return 1
        fi
    done
}

require() {
    command -v "$1" > /dev/null 2>&1 || fail "$1 is needed for this target. $2"
}

require_zig() {
    # cargo-zigbuild finds Zig on PATH or as the ziglang Python package
    command -v zig > /dev/null 2>&1 && return
    local python
    for python in python3 python; do
        if command -v "$python" > /dev/null 2>&1 && "$python" -m ziglang version > /dev/null 2>&1; then
            return
        fi
    done
    fail "Zig is needed to build Linux targets on $host_os. Install it from https://ziglang.org/download, or run: pip install ziglang"
}

require_cargo_tool() {
    if ! command -v "cargo-$1" > /dev/null 2>&1; then
        echo "  Installing cargo-$1..."
        cargo install --locked "cargo-$1" || fail "Installing cargo-$1 failed"
    fi
}

run_cargo() {
    echo "  cargo $*"
    cargo "$@" || fail "cargo failed"
}

# Builds one Rust target. Its binaries end up in $target_dir/<triple>/release.
build_triple() {
    local triple=$1 os native=false
    if command -v rustup > /dev/null 2>&1; then
        rustup target add "$triple"
    elif [ "$triple" != "$(rustc -vV | sed -n 's/^host: //p')" ]; then
        # A Rust from the system's packages builds for this machine only
        fail "rustup is needed to build for $triple. Install Rust from https://rustup.rs"
    fi
    os=$(os_of "$triple")
    # macOS toolchains build for both architectures, Linux needs a cross linker
    if [ "$os" = "$host_os" ] && { [ "$os" != linux ] || [ "${triple%%-*}" = "$host_arch" ]; }; then
        native=true
    fi

    if [ "$os" = linux ]; then
        # libjpeg-turbo, for MJPEG capture devices, is built from source
        require cmake "libjpeg-turbo is built with CMake (https://cmake.org/download)."
        case $triple in
            x86_64-*) require nasm "libjpeg-turbo's x64 SIMD code is assembled with NASM (https://nasm.us)." ;;
        esac
    fi

    if $native; then
        run_cargo build --release -p autosplit --target "$triple"
    elif [ "$os" = linux ]; then
        require_zig
        require_cargo_tool zigbuild
        # Zig's linker says it ignores the -O1 that rustc passes, which rustc shows as a warning
        local rustflags
        rustflags="CARGO_TARGET_$(echo "$triple" | tr '[:lower:]-' '[:upper:]_')_RUSTFLAGS"
        env "$rustflags=-A linker-messages" cargo zigbuild --release -p autosplit --target "$triple.$glibc_version" ||
            fail "cargo zigbuild failed"
    else
        require clang "cargo-xwin links with LLVM (clang and lld)."
        require_cargo_tool xwin
        run_cargo xwin build --release -p autosplit --target "$triple"
    fi
}

# Zips the folder $1 into $2, with the files after it executable, so they run after unzipping
make_zip() {
    local folder=$1 zip=$2
    shift 2
    find "$folder" -type d -exec chmod 755 {} +
    find "$folder" -type f -exec chmod 644 {} +
    local file
    for file in "$@"; do
        chmod 755 "$folder/$file"
    done
    rm -f "$zip"
    if command -v zip > /dev/null 2>&1; then
        (cd "$folder" && zip -qr -X "$zip" .)
    elif command -v python3 > /dev/null 2>&1; then
        # Python's zipfile keeps the Unix permissions it's given
        python3 - "$folder" "$zip" << 'PYTHON'
import os, sys, zipfile
folder, zip_path = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as archive:
    for directory, _, files in os.walk(folder):
        for name in sorted(files):
            path = os.path.join(directory, name)
            archive.write(path, os.path.relpath(path, folder))
PYTHON
    else
        fail "zip or python3 is needed to make the zip."
    fi
}

build() {
    local name=$1
    echo "Building AutoSplit ($name)..."
    if ! can_build "$name"; then
        echo "warning: Skipping $name: macOS builds need a Mac." >&2
        return
    fi

    local output="$root/build/$name" triples triple
    rm -rf "$output"
    mkdir -p "$output"
    triples=$(triples_of "$name")
    for triple in $triples; do
        build_triple "$triple"
    done
    set -- $triples
    local first=$1 binary executables
    if [ "$(os_of "$first")" = windows ]; then
        # AutoSplit.exe is a GUI program, so the command prompt doesn't wait for it. Typing
        # autosplit runs AutoSplit.com first, which runs AutoSplit.exe as a console program.
        # Named like AutoSplit's, where Cargo names them in lowercase.
        binary=AutoSplit.exe
        cp "$target_dir/$first/release/autosplit.exe" "$output/$binary"
        cp "$target_dir/$first/release/autosplit-console.exe" "$output/AutoSplit.com"
    else
        binary=autosplit
        if [ $# -gt 1 ]; then
            local inputs=()
            for triple in "$@"; do
                inputs+=("$target_dir/$triple/release/$binary")
            done
            lipo -create -output "$output/$binary" "${inputs[@]}" || fail "lipo failed"
        else
            cp "$target_dir/$first/release/$binary" "$output/$binary"
        fi
    fi
    # The split image presets and the splits library, which are read from next to the binary
    cp -R "$root/presets" "$root/splits" "$output/"
    executables=("$binary")
    # Adds it to the app menu on Linux, with the icon
    if [ "$(os_of "$first")" = linux ]; then
        cp "$root/crates/autosplit-cli/assets/icon.png" "$output/icon.png"
        cp "$root/crates/autosplit-cli/assets/install-desktop-entry.sh" "$output/install-desktop-entry.sh"
        executables+=(install-desktop-entry.sh)
    fi

    local zip="$root/build/AutoSplit-v$version-$name.zip"
    make_zip "$output" "$zip" "${executables[@]}"
    echo "  Zipped: $zip"
}

debug=false
all=false
requested=()
for argument in "$@"; do
    case $argument in
        --debug) debug=true ;;
        --all) all=true ;;
        --*) fail "Unknown option '$argument'." ;;
        *)
            triples_of "$argument" > /dev/null || fail "Unknown target '$argument'. Targets: $names"
            requested+=("$argument")
            ;;
    esac
done

if $debug; then
    export CARGO_PROFILE_RELEASE_DEBUG=full CARGO_PROFILE_RELEASE_STRIP=none
fi

selected=()
if $all; then
    for name in $names; do
        if can_build "$name"; then
            selected+=("$name")
        fi
    done
elif [ ${#requested[@]} -gt 0 ]; then
    selected=("${requested[@]}")
else
    echo "Select target:"
    set -- $names
    index=1
    for name in "$@"; do
        note=""
        can_build "$name" || note=" (needs a Mac)"
        echo "  [$index] $name$note"
        index=$((index + 1))
    done
    read -r -p "Choice: " choice
    case $choice in
        '' | *[!0-9]*) fail "Pick a number from 1 to $#." ;;
    esac
    [ "$choice" -ge 1 ] && [ "$choice" -le $# ] || fail "Pick a number from 1 to $#."
    eval "selected=(\"\${$choice}\")"
fi

[ ${#selected[@]} -gt 0 ] || fail "There's no target this machine can build."
command -v cargo > /dev/null 2>&1 || fail "Rust is needed. Install it from https://rustup.rs, then open a new terminal."

target_dir=$(cargo metadata --format-version 1 --no-deps --manifest-path "$root/Cargo.toml" |
    grep -o '"target_directory":"[^"]*"' | sed 's/^"target_directory":"//; s/"$//')
[ -n "$target_dir" ] || fail "Couldn't find Cargo's target folder."

for name in "${selected[@]}"; do
    build "$name"
done
echo "Build complete."
