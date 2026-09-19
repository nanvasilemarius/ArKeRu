# install-autostart.ps1 -- open a terminal automatically for the board.
#
#   .\tools\install-autostart.ps1                install (Spawn mode)
#   .\tools\install-autostart.ps1 -Mode Attach   one always-open window instead
#   .\tools\install-autostart.ps1 -Uninstall     remove it
#
# Two modes, and the difference matters:
#
#   Spawn (default)  A hidden watcher polls for the board. Plug it in and a NEW
#                    terminal window appears. Nothing is on screen until you do.
#                    This is what people usually mean by "opens automatically".
#
#   Attach           One window runs `akterm --watch` from login and reconnects
#                    inside itself when the board appears. Lighter, but there is
#                    always a window sitting there, and if you close it nothing
#                    happens on plug-in.
#
# Per-user Startup shortcut either way: no admin rights, no service, nothing
# running as SYSTEM. Delete the shortcut from Explorer if you change your mind.

param(
    [ValidateSet('Spawn', 'Attach')]
    [string]$Mode = 'Spawn',
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$startup = [Environment]::GetFolderPath('Startup')
$link = Join-Path $startup 'ArduinoKernel Terminal.lnk'

if ($Uninstall) {
    if (Test-Path $link) { Remove-Item $link; Write-Host "Removed $link" -ForegroundColor Green }
    else { Write-Host 'Not installed.' -ForegroundColor Yellow }
    Get-Process akterm -ErrorAction SilentlyContinue | Stop-Process -Force
    Get-CimInstance Win32_Process -Filter "Name='powershell.exe' OR Name='pwsh.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -like '*usb-watch.ps1*' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Write-Host 'Stopped any running watcher.' -ForegroundColor Green
    return
}

$akterm = Join-Path $root 'target\release\akterm.exe'
if (-not (Test-Path $akterm)) {
    throw "akterm not built. Run: cargo build --release -p launcher"
}

# Windows Terminal renders the POST screen's box drawing and ANSI colours more
# reliably than conhost.
$wt = (Get-Command wt.exe -ErrorAction SilentlyContinue).Source
$shell = New-Object -ComObject WScript.Shell
$sc = $shell.CreateShortcut($link)

if ($Mode -eq 'Spawn') {
    $watch = Join-Path $root 'tools\usb-watch.ps1'
    $sc.TargetPath = (Get-Command powershell.exe).Source
    $sc.Arguments = "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$watch`""
} else {
    if ($wt) {
        $sc.TargetPath = $wt
        $sc.Arguments = "--title `"ArduinoKernel`" `"$akterm`" --watch --post"
    } else {
        $sc.TargetPath = $akterm
        $sc.Arguments = '--watch --post'
    }
}
$sc.WorkingDirectory = $root
$sc.Description = "Opens a terminal to the ArduinoKernel board ($Mode mode)"
$sc.Save()

Write-Host "Installed: $link" -ForegroundColor Green
Write-Host "Mode:      $Mode"
Write-Host ("Terminal:  " + $(if ($wt) { 'Windows Terminal' } else { 'console host' }))
Write-Host ''
if ($Mode -eq 'Spawn') {
    Write-Host 'Plug the board in and a terminal window will open.' -ForegroundColor Cyan
    Write-Host 'Starting the watcher now so you can test without logging out:' -ForegroundColor Cyan
    Start-Process (Get-Command powershell.exe).Source `
        -ArgumentList "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$(Join-Path $root 'tools\usb-watch.ps1')`"" `
        -WindowStyle Hidden
    Write-Host '  watcher started.'
} else {
    Write-Host "Starts at next login, or run now:  $akterm --watch" -ForegroundColor Cyan
}
