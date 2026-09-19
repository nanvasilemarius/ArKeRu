# usb-watch.ps1 -- open a terminal window each time the board is plugged in.
#
# Runs hidden in the background from a login item. Polls for the board's USB
# VID/PID and, on the transition from absent to present, launches a NEW
# terminal window running akterm.
#
# This is the difference between the two autostart modes:
#
#   Attach  `akterm --watch` in one always-open window. Reconnects inside that
#           window. Nothing happens if the window is closed.
#   Spawn   this script. No window until you plug the board in, then one
#           appears. What most people mean by "it opens automatically".
#
# Polling rather than a device-arrival event is deliberate: Task Scheduler
# cannot subscribe to USB arrival without either enabling an event channel that
# is off by default or registering a permanent WMI consumer, and both need
# admin rights. One PnP query every two seconds is free by comparison.

param(
    [int]$IntervalSec = 2
)

$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
$akterm = Join-Path $root 'target\release\akterm.exe'
$wt = (Get-Command wt.exe -ErrorAction SilentlyContinue).Source

function Board-Present {
    $d = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue |
         Where-Object { $_.InstanceId -like 'USB\VID_2341&PID_1002*' }
    return [bool]$d
}

# Start as "absent" so a board that is already plugged in at login still opens
# a window, rather than waiting for you to unplug it first.
$was = $false

while ($true) {
    $now = Board-Present

    if ($now -and -not $was) {
        # Don't stack windows if one is already attached to the board.
        $running = Get-Process akterm -ErrorAction SilentlyContinue
        if (-not $running) {
            # Give the ESP32 bridge a moment to finish enumerating before we
            # try to open its port. --post then asks the board to redraw its
            # boot screen: the real one goes out the wire while the terminal
            # is still starting, so it can never be caught by racing it.
            Start-Sleep -Milliseconds 1200
            if ($wt) {
                Start-Process $wt -ArgumentList @('--title', 'ArduinoKernel', $akterm, '--post')
            } else {
                Start-Process $akterm -ArgumentList '--post'
            }
        }
    }

    $was = $now
    Start-Sleep -Seconds $IntervalSec
}
