# Dull

A BASIC to native machine code compiler for the Sharp PC-1500 series of computers.

## Two ways to use it

The project builds **two binaries** from the same compiler core (`dull::actions`), so
picking one over the other never changes what gets generated — just how you ask for it:

| Binary | What it is | When to use it |
|---|---|---|
| **`dull`** | An interactive text menu — pick an option, answer a couple of questions | Casual use, or handing the compiler to someone who doesn't want to remember flags |
| **`dull-cli`** | The classic flag-based CLI (`--native-code`, `-o`, ...) | Scripts, automation, or once you already know the flags |

```bash
# Interactive menu
./dull

# Flags, for scripting
./dull-cli programa.bas --native-code -o programa.lh5
```

## Building

This repository depends, at build time, on its sibling project
[PC-1500-Emulator](https://github.com/Trepperian/PC-1500-Emulator) (used by this
compiler's own test suite to run generated code against a real LH5801 CPU and the actual
Sharp PC-1500 ROM — see `src/codegen/test_oracle.rs`). Clone both repositories next to
each other:

```bash
git clone <this-repo-url> SHARP_PC-1500_Compiler
git clone https://github.com/Trepperian/PC-1500-Emulator.git PC-1500-Emulator
cd SHARP_PC-1500_Compiler
cargo build --release
```

The two binaries land in `target/release/dull` and `target/release/dull-cli` — each is a
self-contained executable (only links against standard system libraries: `libc`, `libm`,
`libgcc_s` on Linux), no need to have Rust installed to *run* them, only to build them.

> Cloning the sibling repository is enough to **build**; you do **not** need to also
> initialize its own ROM submodule (`git submodule update --init`) unless you want to run
> this compiler's own test suite (`cargo test`), which loads compiled programs into a real
> ROM dump to verify them.

## Features

- **BASIC Parsing**: Tokenize and parse Sharp PC-1500 BASIC programs
- **Semantic Analysis**: Type checking and validation (`dull-cli --analyze`)
- **Stack-based IR**: Generate intermediate stack-based code, with a built-in interpreter
  to run it directly (`dull-cli --stack-code -e`)
- **Native Compilation**: Compile straight to LH5801 machine code that runs on real
  hardware (with the CE-161 RAM expansion) or in
  [PC-1500-Emulator](https://github.com/Trepperian/PC-1500-Emulator)
- **Optional authentic timing**: `--authentic-timing` paces native code to match the speed
  of the original tokenized BASIC interpreter, instead of running at native-code speed
- **Binary Output**: Compact `.lh5` binary format (4-byte header + machine code)
- **ROM Routines**: Reuses verified PC-1500 ROM routines for I/O instead of reimplementing
  them (see [ROM Routines](#rom-routines) below)

## Compilation Modes

All of these are `dull-cli` flags (the interactive `dull` menu offers the same six modes
without needing to remember any of this):

### 1. BASIC Tokenization (Default)
```bash
dull-cli programa.bas -o programa.bin
```
Generates tokenized BASIC compatible with the PC-1500's own format.

### 2. Stack-based Intermediate Code
```bash
dull-cli programa.bas --stack-code -o programa.p
```
Generates intermediate stack instructions. Add `-e` to run them immediately in the
built-in interpreter (`-v` for a verbose execution trace):
```bash
dull-cli programa.bas --stack-code -e
```

### 3. Native LH5801 Machine Code (Recommended)
```bash
dull-cli programa.bas --native-code -o programa.lh5
```
Compiles straight to LH5801 opcodes — no interpreter loop, no tokenized loader, just the
machine code the CPU executes directly. Add `--authentic-timing` to pace execution like
the original interpreter instead of running at full native speed.

## ROM Routines

The compiler calls into real Sharp PC-1500 ROM routines for I/O instead of
reimplementing them — every address below was found by reading the actual ROM
disassembly, not guessed from a routine's name (see `src/codegen/rom_routines.rs` for the
full catalogue, including inputs/outputs/preserved-registers per routine):

| Operation | ROM Address | Routine |
|-----------|-------------|---------|
| Print a character | `0xED4D` | `CHAR_OUT` |
| BCD arithmetic (+, -, ×, ÷) | `0xEFBA`/`0xEFB6`/`0xF01A`/`0xF084` | `ADDIT`/`SUBTR`/`MULTIPLY`/`DIVISION` |
| Clear screen | `0xEE71` + `0xECAE` | `LCD_CLR` + `INIT_CURS` |
| Keyboard polling | `0xE418`/`0xE42C` | `ISKEY`/`KEY_2_ASCII` |
| Beep | `0xE66F` | `BEEP` |
| Timed delay | `0xE88C` | `TIME_DELAY` |
| Graphics dot output | `0xEDEF`/`0xEDB1` | `GPRINT_OUT`/`MTRX_INC` |

`RND` is the one deliberate exception: no verified ROM entry point for `RND(n>0)` was
found (see the comment on `RAND_GEN` in `rom_routines.rs`), so it's implemented as a
self-contained 8-bit Galois LFSR instead — documented explicitly as not
hardware-authentic.

## Architecture

```
BASIC Source Code (.bas)
         ↓
    Lexer (tokens)
         ↓
    Parser (AST)
         ↓
  Semantic Analysis
         ↓
Stack Instruction Generator
         ↓
   LH5801 Backend
         ↓
  Binary LH5 File (.lh5)
```

Both `dull` and `dull-cli` are thin front-ends over this same pipeline
(`dull::actions`, in `src/actions.rs`) — neither binary reimplements any part of it.

## Memory Layout

Native code targets a Sharp PC-1500 with the **CE-161** RAM expansion
(18176 bytes of contiguous user RAM):

```
0x0100-0x47FF  User RAM (18176 bytes), split into three windows:
               - generated machine code, from 0x0100
               - variables/buffers, right after the code
                 (computed per-program, see compile_native_two_pass_with_timing)
               - a compiled program's own software stack, using the LH5801's
                 hardware S register directly (initialized in the prologue)
```

## Programs that compile

All original files are encoded in ISO-8859 and found in [Sharp PC-1500 (TRS-80 PC-2) resource page](http://www.pc1500.com/).

| Program                | Parses | Comments                                                               |
| ---------------------- | ------ | ---------------------------------------------------------------------- |
| atterisage             | [x]    |                                                                        |
| bataille-dans-l-espace | [x]    |                                                                        |
| bathyscap              | [x]    | '\\'' replaced by '^'                                                  |
| battlecars             | [ ]    | 3 letter identifiers                                                   |
| blackjack              | [x]    |                                                                        |
| bombing                | [x]    |                                                                        |
| bowling                | [x]    |                                                                        |
| course                 | [x]    |                                                                        |
| dames                  | [x]    |                                                                        |
| decathlon              | [x]    |                                                                        |
| donkey-kong            | [x]    |                                                                        |
| DungeonQuest           | [x]    |                                                                        |
| formula1               | [x]    |                                                                        |
| ghosthouse             | [x]    | 'GRPINT' at line 771 corrected                                         |
| gloupman               | [x]    | Removed extra ',' at end of DATA in line 1271                          |
| invader                | [x]    |                                                                        |
| force                  | [ ]    | Extra ',' at ON GOTO, Tape commands: PRINT#                            |
| jackpot                | [x]    |                                                                        |
| jeu-des-blocks         | [?]    | Line 28 probably has an extra '"'                                      |
| labyrinthe             | [x]    |                                                                        |
| loup-des-mers          | [x]    |                                                                        |
| meteorites             | [x]    | '\\'' replaced by '^'                                                  |
| micromur               | [x]    |                                                                        |
| minenboot              | [x]    | Errors in bas file corrected from image listing: 'GCUROSR' at line 690 |
| mole                   | [x]    |                                                                        |
| monstres&merveilles    | [x]    | Replaced [5D] by π                                                     |
| morpion                | [x]    |                                                                        |
| othello                | [x]    |                                                                        |
| pacman                 | [x]    |                                                                        |
| Pilesjr                | [x]    |                                                                        |
| rasemottes             | [x]    |                                                                        |
| scrabble               | [x]    |                                                                        |
| simulateur-de-vol      | [x]    | Replaced [5D] by π                                                     |
| slalom                 | [x]    |                                                                        |
| tank                   | [x]    |                                                                        |
| tempter                | [ ]    | IF without THEN clause in line 40                                      |
| trio                   | [x]    |                                                                        |

## Related project

[PC-1500-Emulator](https://github.com/Trepperian/PC-1500-Emulator) is the companion
emulator for this compiler: it runs the `.lh5` files this compiler produces against a
real LH5801 CPU implementation loaded with the actual Sharp PC-1500 ROM, with an `egui`
GUI to load a `.lh5` and watch it run. It's also this compiler's own test oracle (see
`src/codegen/test_oracle.rs`) — every backend test that says "on_real_rom" runs its
generated code through this same emulator core, not just against expected bytes.

## Thanks

- [Sharp PC-1500 (TRS-80 PC-2) resource page](http://www.pc1500.com/)
- [Sharp_PC-1500_ROM_Disassembly](https://github.com/Jeff-Birt/Sharp_PC-1500_ROM_Disassembly)
- [Sharp_CE-158](https://github.com/Jeff-Birt/Sharp_CE-158)
- [Schematics](https://www.kaibader.de/sharp-pc-15001600-schematics-collection/)
