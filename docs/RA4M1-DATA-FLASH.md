# RA4M1 data flash — the interface the User's Manual withholds

The RA4M1 has **8 KB of data flash, rated 100,000 erase cycles**, entirely
unused by this project. Making `A:` writable needs it, and the reason that has
been blocked until now is documented here so nobody re-derives it.

## Why the manual is not enough

§44.7 of the RA4M1 Group User's Manual (R01UH0887EJ0100), titled *Programming
Commands*, reads in full:

> The FCB controls the programming commands.

Table 44.1 says erasing and programming happen "through the **FCB commands
specified in the registers**". Those registers are named nowhere in the 1436
pages. Searching the extracted text for `FCR`, `FSTATR`, `FSARL`, `FPMCR` or
`FACI` returns nothing. Renesas withholds the Flash Control Block interface
from the manual and ships it only inside FSP.

**This is why guessing was refused.** It was not caution about a hard problem —
there was nothing to guess *from*, and the sequence below shows that no amount
of care would have arrived at it.

## Where it actually comes from

[Renesas FSP](https://github.com/renesas/fsp), which is public and supports
`EK-RA4M1`:

- `ra/fsp/src/bsp/cmsis/Device/RENESAS/Include/R7FA4M1AB.h` — register layout
- `ra/fsp/src/r_flash_lp/r_flash_lp.c` — the sequences

## Register map

`R_FACI_LP` base address **`0x407EC000`**, offsets from there:

| Offset | Register | Width | Purpose |
|---|---|---|---|
| `0x090` | `DFLCTL` | 8 | Data flash access enable |
| `0x100` | `FPMCR` | 8 | P/E mode control |
| `0x104` | `FASR` | 8 | Area select |
| `0x108` | `FSARL` | 16 | Start address, low |
| `0x110` | `FSARH` | 16 | Start address, high |
| `0x114` | `FCR` | 8 | Command |
| `0x118` | `FEARL` | 16 | End address, low |
| `0x120` | `FEARH` | 32 | End address, high |
| `0x124` | `FRESETR` | 32 | Reset |
| `0x128` | `FSTATR00` | 32 | Status 0 |
| `0x12C` | `FSTATR1` | 32 | Status 1 — holds `FRDY` |
| `0x130` | `FWBL0` | 32 | Write buffer, low |
| `0x138` | `FWBH0` | 32 | Write buffer, high |
| `0x180` | `FPR` | 32 | Protection unlock (write-only) |
| `0x184` | `FPSR` | 32 | Unlock status |
| `0x1F0` | `FSTATR2` | 32 | Status 2 |

Data flash itself is at **`0x40100000`**, 8 KB.

## Values

```
FENTRYR   16-bit, 0xAA in the high byte as a key
          0xAA80  data flash P/E mode
          0xAA01  code flash P/E mode      <-- never write this
          0xAA00  read mode

FPMCR     0x10  data flash P/E
          0x08  read
          0x40  LVPE, OR'd in when OPCCR.OPCM != 0 (not high-speed mode)
          0x82  code flash P/E            <-- never write this

FCR       0x81  write        0x84  erase
          0x83  blank check  0x00  clear
          bit 7 also reads back as "processing"

FPR       0xA5  unlock key
```

## The part that could not have been guessed

`FPMCR` is not a normal register. Writing it takes **three writes — value,
its complement, value again** — after unlocking with `FPR`:

```c
R_FACI_LP->FPR   = 0xA5;
R_FACI_LP->FPMCR = value;
R_FACI_LP->FPMCR = (uint8_t) ~value;
R_FACI_LP->FPMCR = value;
/* then read back and confirm it equals value */
```

There is no way to infer a write-complement-write protocol from a register
name. This single detail justifies the whole refusal to proceed on guesswork.

## Entering data flash P/E mode

```
DFLCTL = 1                       (once, if not already 1)
FENTRYR = 0xAA80
delay 6 us                       (TDSTOP)
write_fpmcr(0x10)                (OR 0x40 if not in high-speed mode)
delay 3 us                       (TDIS)
```

Leaving: `FENTRYR = 0xAA00`, then poll `FENTRYR` until it reads `0`.

Commanding: set `FSARH`/`FSARL` and `FEARH`/`FEARL`, load `FWBL0`/`FWBH0` for a
write, set `FCR` to the command, wait for `FSTATR1.FRDY`, clear `FCR`, wait for
`FRDY` to drop.

## Why this is safer than it sounds

**Data flash P/E mode cannot touch code flash.** The mode is selected by
`FENTRYR`, and `0xAA80` selects the data flash sequencer only. A bug in data
flash code destroys saved settings, not the firmware — the board still boots
and can still be reflashed. The dangerous values are `FENTRYR = 0xAA01` and
`FPMCR = 0x82`, and a data flash driver has no reason to contain either.

One caveat from the manual, Table 44.12: background operation allows executing
from **code** flash while writing **data** flash. So a data-flash-only driver
does not need to run from RAM. FSP marks its routines RAM-resident because the
same code also handles code flash, where it obviously must.

## The third thing, found the hard way: `FISR.PCKA`

`R_FACI_LP->FISR` at offset **`0x1D8`**, bits 5:0, is the **flash clock
notification**. The sequencer times its own program and erase pulses from it,
and it is writable **only in P/E mode**.

```c
R_FACI_LP->FISR_b.PCKA = (fclk_mhz - 1) & 0x1F;   /* MF3 / VERSION == 3 */
```

Leaving it unset does not fail. That is the entire problem:

- erases work, and the block reads back all `0xFF`
- `FRDY` comes up on schedule after every command
- every command reports success
- and the **programming is marginal**

Bytes needing a few bits cleared come out correct. Bytes needing all eight
cleared come out as noise. A save record is mostly small numbers and zero
padding, so what it produced was a record in which *every failing byte was a
`0x00`* and every non-zero byte was perfect:

```
1800  41 56 01 56 6C 55 72 73 75 44 C0 88 B0 38 90 06   <- name padding should be 00
1820  0C 01 00 D8 0C 52 61 70 40 88 40 2A 0F 00 40 C1   <- xp, coin, flags should be 00
```

After setting `PCKA`, the same write:

```
1800  41 56 01 56 6C 55 72 73 75 00 00 00 00 00 00 06
1820  10 01 00 00 0C 00 00 00 00 00 00 00 00 00 00 00
```

`FCLK` is derived at runtime from `SCKDIVCR` (`0x4001E020`, `FCK` in bits 30:28,
`ICK` in 26:24) rather than hardcoded, so a future clock change fixes the flash
timing instead of silently corrupting writes.

## Poll `FSTATR2`, not just `FRDY`

`FSTATR2` at offset **`0x1F0`** is where the sequencer says a command *failed*.
`FRDY` only says it stopped. FSP checks this after **every** command:

| Mask | Meaning |
|---|---|
| `0x12` | write error (`FLASH_LP_FSTATR2_WRITE_ERROR_BITS`) |
| `0x11` | erase error (`FLASH_LP_FSTATR2_ERASE_ERROR_BITS`) |

Omitting it is why the `PCKA` bug survived a full round of testing: the driver
had no way to say anything except "done".

## Two things FSP knows that nothing else tells you

Both cost a debugging round on real hardware.

**Reads need `DFLCTL = 1` too**, not just program and erase. With the data
flash controller powered down the array reads back as `0x00` — which is a
convincing wrong answer, because erased flash reads `0xFF`. Zeros mean "not
powered", not "empty".

**The array has two address spaces.** Reads come from `0x40100000`. `FSAR` and
`FEAR` take **`0xFE000000`**. FSP notes it in a one-line comment — *"Conversion
to the P/E address from the read address"* — and nothing in the register names
or the manual suggests it. Getting this wrong fails *silently*: an erase aimed
at the read address reports success and does nothing, which is indistinguishable
from a working erase for as long as the block happens to be blank already.

## Timings that matter

| Operation | Worst case |
|---|---|
| Data flash block erase | **504 ms** (`FLASH_LP_MAX_ERASE_DF_BLOCK_TIME_US`) |
| Data flash byte program | 886 µs |

A first draft of the driver used a fixed loop count worth about 20 ms, which
would have aborted every erase a twenty-fifth of the way through and then
written mode registers while the sequencer was still running.

## Status

**Implemented and proven on hardware**, in
[`board-r4/src/dataflash.rs`](../board-r4/src/dataflash.rs), exposed as the
`DF` command. Erase → blank, write → read back, reboot → still there, erase a
block with data in it → blank again.

## What lives where

Three consumers, deliberately separated so that any one of them going wrong
cannot take the others with it.

| Blocks | Offset | Holds | Written by |
|---|---|---|---|
| 0 – 5 | 0x0000 | A story pack, 6 KB | `DLC LOAD` |
| 6 | 0x1800 | The RPG save, ~450 bytes | `RPG` → Save |
| 7 | 0x1C00 | Nothing | `DF`, the diagnostic |

The save is **not** in the last block, because `DF ERASE` operates on that one
and a diagnostic that wipes your character is a trap laid for the person most
likely to run it. The pack is at the bottom because it is the only consumer big
enough to care where it starts.

`A:` is still read-only and settings still live in the ESP32's NVS. This is a
driver with three tenants, not a filesystem.
