#!/usr/bin/env pwsh
# Builds release zips of AutoSplit.
#
#   ./build.ps1                     pick a target from a menu
#   ./build.ps1 windows-x64 ...     build these targets
#   ./build.ps1 --all               build every target this machine can build
#   ./build.ps1 --debug ...         keep debug symbols
#
# Same OS as this machine: plain cargo. Linux from another OS: cargo-zigbuild. Windows from
# another OS: cargo-xwin. macOS only builds on a Mac, because Apple's SDK is only licensed there.

$ErrorActionPreference = "Stop"

$targets = [ordered]@{
    "windows-x64"     = @("x86_64-pc-windows-msvc")
    "windows-arm64"   = @("aarch64-pc-windows-msvc")
    "linux-x64"       = @("x86_64-unknown-linux-gnu")
    "linux-arm64"     = @("aarch64-unknown-linux-gnu")
    "macos-universal" = @("aarch64-apple-darwin", "x86_64-apple-darwin")
}

# Oldest glibc Linux builds support when cross-compiled, matching the Ubuntu 22.04 CI runners
$glibcVersion = "2.35"

$debug = $args -contains "--debug"
$all = $args -contains "--all"
$requested = @($args | Where-Object { -not $_.StartsWith("--") })

$cargoToml = Get-Content "$PSScriptRoot/Cargo.toml" -Raw
if ($cargoToml -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
    Write-Error "Couldn't find [workspace.package] version in Cargo.toml"
}
$version = $Matches[1]

$hostOs = if ($IsWindows) { "windows" } elseif ($IsMacOS) { "macos" } else { "linux" }
$hostArch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "Arm64") { "aarch64" } else { "x86_64" }
$targetDir = (cargo metadata --format-version 1 --no-deps --manifest-path "$PSScriptRoot/Cargo.toml" | ConvertFrom-Json).target_directory

function OsOf($triple) {
    if ($triple -like "*-windows-*") { "windows" } elseif ($triple -like "*-apple-*") { "macos" } else { "linux" }
}

function CanBuild($name) {
    foreach ($triple in $targets[$name]) {
        if ((OsOf $triple) -eq "macos" -and $hostOs -ne "macos") { return $false }
    }
    return $true
}

function Require($command, $hint) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        Write-Error "$command is needed for this target. $hint"
    }
}

function RequireZig() {
    # cargo-zigbuild finds Zig on PATH or as the ziglang Python package
    if (Get-Command "zig" -ErrorAction SilentlyContinue) { return }
    foreach ($python in @("python3", "python")) {
        if (Get-Command $python -ErrorAction SilentlyContinue) {
            & $python -m ziglang version *> $null
            if ($LASTEXITCODE -eq 0) { return }
        }
    }
    Write-Error "Zig is needed to build Linux targets on $hostOs. Install it from https://ziglang.org/download, or run: pip install ziglang"
}

function RequireCargoTool($tool) {
    if (-not (Get-Command "cargo-$tool" -ErrorAction SilentlyContinue)) {
        Write-Host "  Installing cargo-$tool..."
        cargo install --locked "cargo-$tool" | Out-Host
        if ($LASTEXITCODE -ne 0) { Write-Error "Installing cargo-$tool failed" }
    }
}

function Invoke-Cargo($cargoArgs) {
    Write-Host "  cargo $($cargoArgs -join ' ')"
    & cargo @cargoArgs | Out-Host
    if ($LASTEXITCODE -ne 0) { Write-Error "cargo failed with exit code $LASTEXITCODE" }
}

# Builds one Rust target and returns the folder with its binaries.
function BuildTriple($triple) {
    rustup target add $triple | Out-Host
    $os = OsOf $triple
    $profileArgs = @("--release", "-p", "autosplit")
    # Windows and macOS toolchains build for both architectures, Linux needs a cross linker
    $native = $os -eq $hostOs -and ($os -ne "linux" -or $triple -like "$hostArch-*")

    if ($os -eq "linux") {
        # libjpeg-turbo, for MJPEG capture devices, is built from source
        Require "cmake" "libjpeg-turbo is built with CMake (https://cmake.org/download)."
        if ($triple -like "x86_64-*") {
            Require "nasm" "libjpeg-turbo's x64 SIMD code is assembled with NASM (https://nasm.us)."
        }
    }

    if ($native) {
        Invoke-Cargo (@("build") + $profileArgs + @("--target", $triple))
    }
    elseif ($os -eq "linux") {
        RequireZig
        RequireCargoTool "zigbuild"
        # Zig's linker says it ignores the -O1 that rustc passes, which rustc shows as a warning
        $rustflags = "CARGO_TARGET_$($triple.ToUpper() -replace '-', '_')_RUSTFLAGS"
        $previous = [Environment]::GetEnvironmentVariable($rustflags)
        [Environment]::SetEnvironmentVariable($rustflags, "-A linker-messages")
        try {
            Invoke-Cargo (@("zigbuild") + $profileArgs + @("--target", "$triple.$glibcVersion"))
        }
        finally {
            [Environment]::SetEnvironmentVariable($rustflags, $previous)
        }
    }
    elseif ($os -eq "windows") {
        Require "clang" "cargo-xwin links with LLVM (clang and lld)."
        RequireCargoTool "xwin"
        Invoke-Cargo (@("xwin", "build") + $profileArgs + @("--target", $triple))
    }

    return "$targetDir/$triple/release"
}

# Compress-Archive drops the executable bit, so Linux and macOS binaries are added with Unix
# permissions set
function Zip($directory, $zip, $executable) {
    Add-Type -AssemblyName System.IO.Compression
    $stream = [System.IO.File]::Open($zip, [System.IO.FileMode]::Create)
    $archive = [System.IO.Compression.ZipArchive]::new($stream, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in Get-ChildItem $directory -File) {
            $entry = $archive.CreateEntry($file.Name, [System.IO.Compression.CompressionLevel]::Optimal)
            $mode = if ($file.Name -eq $executable) { 0x81ED } else { 0x81A4 }
            $entry.ExternalAttributes = $mode -shl 16
            $writer = $entry.Open()
            try {
                $reader = [System.IO.File]::OpenRead($file.FullName)
                try { $reader.CopyTo($writer) } finally { $reader.Dispose() }
            }
            finally { $writer.Dispose() }
        }
    }
    finally {
        $archive.Dispose()
        $stream.Dispose()
    }
}

function Build($name) {
    Write-Host "Building AutoSplit ($name)..."
    if (-not (CanBuild $name)) {
        Write-Warning "Skipping ${name}: macOS builds need a Mac."
        return
    }

    $output = "$PSScriptRoot/build/$name"
    if (Test-Path $output) { Remove-Item $output -Recurse -Force }
    New-Item -ItemType Directory -Path $output | Out-Null

    $folders = @($targets[$name] | ForEach-Object { BuildTriple $_ })
    if ((OsOf $targets[$name][0]) -eq "windows") {
        # autosplit.exe is a GUI program, so the command prompt doesn't wait for it. Typing
        # autosplit runs autosplit.com first, which runs autosplit.exe as a console program.
        $binaryName = "autosplit.exe"
        Copy-Item "$($folders[0])/$binaryName" "$output/$binaryName"
        Copy-Item "$($folders[0])/autosplit-console.exe" "$output/autosplit.com"
    }
    else {
        $binaryName = "autosplit"
        if ($folders.Count -gt 1) {
            lipo -create -output "$output/$binaryName" @($folders | ForEach-Object { "$_/$binaryName" })
            if ($LASTEXITCODE -ne 0) { Write-Error "lipo failed with exit code $LASTEXITCODE" }
        }
        else {
            Copy-Item "$($folders[0])/$binaryName" "$output/$binaryName"
        }
    }
    # The licenses have to ship with the binaries
    foreach ($file in @("LICENSE", "THIRD_PARTY_NOTICES.txt")) {
        Copy-Item "$PSScriptRoot/$file" $output
    }

    $zip = "$PSScriptRoot/build/AutoSplit-v$version-$name.zip"
    Zip $output $zip $binaryName
    Write-Host "  Zipped: $zip"
}

if ($debug) {
    $env:CARGO_PROFILE_RELEASE_DEBUG = "full"
    $env:CARGO_PROFILE_RELEASE_STRIP = "none"
}

$selected = if ($all) {
    @($targets.Keys | Where-Object { CanBuild $_ })
}
elseif ($requested.Count -gt 0) {
    foreach ($name in $requested) {
        if (-not $targets.Contains($name)) {
            Write-Error "Unknown target '$name'. Targets: $($targets.Keys -join ', ')"
        }
    }
    $requested
}
else {
    Write-Host "Select target:"
    $names = @($targets.Keys)
    for ($i = 0; $i -lt $names.Count; $i++) {
        $note = if (CanBuild $names[$i]) { "" } else { " (needs a Mac)" }
        Write-Host "  [$($i + 1)] $($names[$i])$note"
    }
    $choice = Read-Host "Choice"
    @($names[[int]$choice - 1])
}

foreach ($name in $selected) { Build $name }
Write-Host "Build complete."
