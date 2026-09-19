# elfinfo.ps1 -- report section sizes and emit flashable images from the ELF.
#
#   .\tools\elfinfo.ps1 board-r4\target\thumbv7em-none-eabihf\release\arduinokernel
#
# Stands in for arm-none-eabi-size and objcopy, which are not installed.
# 32-bit little-endian ELF only, which is all a Cortex-M4 produces.
#
# This replaced an identical Python script so the project needs no interpreter
# beyond the PowerShell that is already required to build it. The port was
# checked by diffing the .bin and .hex it produces against the Python output
# byte for byte.

param(
    [Parameter(Mandatory = $true)][string]$Elf
)

$ErrorActionPreference = 'Stop'

$FLASH_BYTES  = 240 * 1024
$RAM_BYTES    = 32 * 1024
$FLASH_ORIGIN = 0x00004000

$SHF_ALLOC     = 0x2
$SHF_EXECINSTR = 0x4
$SHT_NOBITS    = 8

if (-not (Test-Path -LiteralPath $Elf)) { throw "no such file: $Elf" }
$path = (Resolve-Path -LiteralPath $Elf).Path
$data = [System.IO.File]::ReadAllBytes($path)

if ($data.Length -lt 52 -or
    $data[0] -ne 0x7f -or $data[1] -ne 0x45 -or $data[2] -ne 0x4c -or $data[3] -ne 0x46 -or
    $data[4] -ne 1 -or $data[5] -ne 1) {
    throw 'not a 32-bit little-endian ELF'
}

function Get-U32([int]$off) { [BitConverter]::ToUInt32($data, $off) }
function Get-U16([int]$off) { [BitConverter]::ToUInt16($data, $off) }

$e_phoff     = Get-U32 0x1C
$e_shoff     = Get-U32 0x20
$e_phentsize = Get-U16 0x2A
$e_phnum     = Get-U16 0x2C
$e_shentsize = Get-U16 0x2E
$e_shnum     = Get-U16 0x30
$e_shstrndx  = Get-U16 0x32

# Section header string table, so sections can be named.
$o = $e_shoff + $e_shstrndx * $e_shentsize
$strOff = Get-U32 ($o + 0x10)

$sections = foreach ($i in 0..($e_shnum - 1)) {
    $o = $e_shoff + $i * $e_shentsize
    $nameIdx = Get-U32 $o
    $end = $strOff + $nameIdx
    while ($data[$end] -ne 0) { $end++ }
    [pscustomobject]@{
        Name   = [Text.Encoding]::ASCII.GetString($data, $strOff + $nameIdx, $end - ($strOff + $nameIdx))
        Type   = Get-U32 ($o + 0x04)
        Flags  = Get-U32 ($o + 0x08)
        Addr   = Get-U32 ($o + 0x0C)
        Offset = Get-U32 ($o + 0x10)
        Size   = Get-U32 ($o + 0x14)
    }
}

$segments = foreach ($i in 0..($e_phnum - 1)) {
    $o = $e_phoff + $i * $e_phentsize
    $type   = Get-U32 $o
    $filesz = Get-U32 ($o + 0x10)
    if ($type -eq 1 -and $filesz -gt 0) {   # PT_LOAD with contents
        [pscustomobject]@{
            Offset = Get-U32 ($o + 0x04)
            Paddr  = Get-U32 ($o + 0x0C)
            Filesz = $filesz
        }
    }
}

# ---------------------------------------------------------------- sizes

$text = 0; $rodata = 0; $bss = 0; $sdata = 0
'{0,-20}{1,12}{2,10}' -f 'section', 'addr', 'size'
'-' * 42
foreach ($s in $sections) {
    if ((($s.Flags -band $SHF_ALLOC) -eq 0) -or $s.Size -eq 0) { continue }
    '{0,-20}{1,12}{2,10}' -f $s.Name, ('0x' + $s.Addr.ToString('x')), $s.Size
    if ($s.Type -eq $SHT_NOBITS)                 { $bss    += $s.Size }
    elseif ($s.Flags -band $SHF_EXECINSTR)       { $text   += $s.Size }
    elseif ($s.Addr -ge 0x20000000)              { $sdata  += $s.Size }
    else                                         { $rodata += $s.Size }
}

$flash = $text + $rodata + $sdata    # .data ships in flash, copied to RAM
$ram   = $bss + $sdata
'-' * 42
'  text   {0,8}   rodata {1,8}   data {2,6}   bss {3,6}' -f $text, $rodata, $sdata, $bss
# InvariantCulture, or a machine set to Romanian prints "24,8%" -- harmless
# here but a nuisance to anyone grepping the build log for a number.
$inv = [cultureinfo]::InvariantCulture
''
[string]::Format($inv, '  FLASH  {0,8} / {1}  ({2:F1}%)', $flash, $FLASH_BYTES, 100 * $flash / $FLASH_BYTES)
[string]::Format($inv, '  RAM    {0,8} / {1}  ({2:F1}%)  static only', $ram, $RAM_BYTES, 100 * $ram / $RAM_BYTES)

if ($flash -gt $FLASH_BYTES) { throw 'image does not fit in flash' }
if ($ram   -gt $RAM_BYTES)   { throw 'statics do not fit in RAM' }

# ---------------------------------------------------------------- images

# Flatten LOAD segments into a raw image based at the lowest paddr.
#
# Segments below FLASH_ORIGIN are dropped: linkers emit a PT_LOAD at 0 covering
# the ELF and program headers themselves, and including it would prepend 16 KB
# of padding plus the header bytes to the image. objcopy avoids this by working
# from allocatable sections; the same result comes out of filtering. p_paddr
# (not p_vaddr) is used throughout so a non-empty .data lands at its load
# address in flash rather than its RAM address.
$load = @($segments | Where-Object { $_.Paddr -ge $FLASH_ORIGIN })
if ($load.Count -eq 0) { throw 'no loadable segments at or above the flash origin' }

$base = ($load | Measure-Object -Property Paddr -Minimum).Minimum
$end  = ($load | ForEach-Object { $_.Paddr + $_.Filesz } | Measure-Object -Maximum).Maximum
$img  = New-Object byte[] ($end - $base)
foreach ($s in $load) {
    [Array]::Copy($data, [int]$s.Offset, $img, [int]($s.Paddr - $base), [int]$s.Filesz)
}

if ($base -ne $FLASH_ORIGIN) {
    Write-Warning ('image base 0x{0:x}, expected 0x{1:x}' -f $base, $FLASH_ORIGIN)
}

# Intel HEX, which is what rfp-cli and most Renesas tooling want.
$sb = [Text.StringBuilder]::new()
$upper = -1
for ($i = 0; $i -lt $img.Length; $i += 16) {
    $addr = $base + $i
    $hi = ($addr -shr 16) -band 0xFFFF
    if ($hi -ne $upper) {
        # Extended linear address record.
        $sum = (2 + 0 + 0 + 4 + (($hi -shr 8) -band 0xFF) + ($hi -band 0xFF))
        [void]$sb.Append((':02000004{0:X4}{1:X2}' -f $hi, ((-$sum) -band 0xFF))).Append("`r`n")
        $upper = $hi
    }
    $n = [Math]::Min(16, $img.Length - $i)
    $sum = $n + (($addr -shr 8) -band 0xFF) + ($addr -band 0xFF)
    [void]$sb.Append((':{0:X2}{1:X4}00' -f $n, ($addr -band 0xFFFF)))
    for ($k = 0; $k -lt $n; $k++) {
        $b = $img[$i + $k]
        $sum += $b
        [void]$sb.Append($b.ToString('X2'))
    }
    [void]$sb.Append(('{0:X2}' -f ((-$sum) -band 0xFF))).Append("`r`n")
}
[void]$sb.Append(":00000001FF`r`n")

$binPath = [IO.Path]::ChangeExtension($path, '.bin')
$hexPath = [IO.Path]::ChangeExtension($path, '.hex')
[IO.File]::WriteAllBytes($binPath, $img)
[IO.File]::WriteAllText($hexPath, $sb.ToString(), (New-Object Text.UTF8Encoding $false))

''
"  wrote $binPath  ($($img.Length) bytes)"
"  wrote $hexPath"
