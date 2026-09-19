# flash.ps1 -- build and flash the kernel, then optionally open a terminal.
#
#   .\tools\flash.ps1              build, flash, and connect with akterm
#   .\tools\flash.ps1 -NoTerm      build and flash only
#   .\tools\flash.ps1 -Probe       flash the bring-up probe instead
#   .\tools\flash.ps1 -Port COM7   override port detection
#
# Wraps the arduino-cli invocation with ARDUINO_DIRECTORIES_* pointed at the
# project-local toolchain, so nothing has to be installed globally.

param(
    [switch]$NoTerm,
    [switch]$Probe,
    [string]$Port = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

$env:ARDUINO_DIRECTORIES_DATA      = Join-Path $root 'tools\arduino15'
$env:ARDUINO_DIRECTORIES_DOWNLOADS = Join-Path $root 'tools\arduino15\staging'
$env:ARDUINO_DIRECTORIES_USER      = Join-Path $root 'tools\arduino-user'

$cli    = Join-Path $root 'tools\arduino-cli\arduino-cli.exe'
$akterm = Join-Path $root 'target\release\akterm.exe'
$name   = if ($Probe) { 'probe' } else { 'arduinokernel' }
$elf    = Join-Path $root "board-r4\target\thumbv7em-none-eabihf\release\$name"

if (-not (Test-Path $cli)) {
    throw "arduino-cli not found at $cli. See README.md."
}

# Print the version being flashed. A version bump after a flash leaves the
# board reporting the old number while the repo claims the new one, which is
# invisible until someone reads VER on the board and notices.
$ver = (Select-String -Path (Join-Path $root 'kernel\Cargo.toml') -Pattern '^version\s*=' |
        Select-Object -First 1).Line -replace '.*"(.*)".*', '$1'
Write-Host "== building v$ver ==" -ForegroundColor Cyan
Push-Location (Join-Path $root 'board-r4')
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location }

# elfinfo throws if the image does not fit, and a throw in a called script is a
# terminating error here -- so there is no exit code left to check.
Write-Host '== size ==' -ForegroundColor Cyan
& (Join-Path $root 'tools\elfinfo.ps1') $elf

# Find the board if no port was given. akterm knows the VID/PID.
if (-not $Port) {
    if (Test-Path $akterm) {
        $line = & $akterm --list 2>&1 | Select-String 'ArduinoKernel board'
        if ($line) { $Port = ($line -split '\s+')[1] }
    }
    if (-not $Port) {
        $Port = 'COM3'
        Write-Warning "Could not auto-detect the board; assuming $Port."
    }
}

Write-Host "== flashing $name to $Port ==" -ForegroundColor Cyan
& $cli upload --input-file "$elf.bin" --fqbn arduino:renesas_uno:unor4wifi -p $Port
if ($LASTEXITCODE -ne 0) { throw 'upload failed' }

if (-not $NoTerm) {
    if (Test-Path $akterm) {
        Start-Sleep -Milliseconds 1500
        Write-Host '== connecting (Ctrl-] to quit) ==' -ForegroundColor Cyan
        & $akterm --port $Port
    } else {
        Write-Warning "akterm not built. Run: cargo build --release -p launcher"
    }
}
