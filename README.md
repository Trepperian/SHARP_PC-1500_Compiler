# Dull

A BASIC to binary compiler (a glorified parser) for the Sharp PC-1500 series of computers.

## Features

- **BASIC Parsing**: Tokenize and parse Sharp PC-1500 BASIC programs
- **Semantic Analysis**: Type checking and validation
- **Stack-based IR**: Generate intermediate stack-based code
- **Native Compilation**: Compile to LH5801 machine code
- **Binary Output**: Generate optimized `.lh5` binary files (92% smaller!)
- **ROM Routines**: Uses PC-1500 ROM routines for I/O operations (see [RUTINAS_ROM.md](RUTINAS_ROM.md))

## Compilation Modes

### 1. BASIC Tokenization (Default)
```bash
cargo run programa.bas -o programa.bin
```
Generates tokenized BASIC compatible with PC-1500 format.

### 2. Stack-based Intermediate Code
```bash
cargo run programa.bas --stack-code -o programa.p
```
Generates intermediate stack instructions. Add `-e` to execute:
```bash
cargo run programa.bas --stack-code -e
```

### 3. Native LH5801 Machine Code (Recommended)
```bash
cargo run programa.bas --native-code -o programa.lh5
```
Generates optimized binary with LH5801 opcodes:
- **Input**: BASIC source code
- **Output**: 688 bytes (vs 8,640 with tokenized loader)
- **Reduction**: 92% smaller
- **Format**: Binary with 4-byte header + machine code

See [FORMATO_LH5.md](FORMATO_LH5.md) for emulator integration.

## Quick Start

```bash
# Compile test program to native code
cargo run test_native.bas --native-code

# View generated file
ls -lh a.lh5
hexdump -C a.lh5 | head

# Compare with old format (if available)
cargo run test_native.bas --native-code -o nuevo.lh5
du -h nuevo.lh5 viejo.bas
```

## ROM Routines

The compiler uses PC-1500 ROM routines to simplify I/O operations:

| Operation | ROM Address | Status |
|-----------|-------------|--------|
| PRINT char | 0x04BC | ✅ Implemented |
| PRINT newline | 0x0563 | ✅ Implemented |
| PRINT string | 0x04F3 | ⚠️ Pending |
| PRINT number | 0x0527 | ⚠️ Pending |
| INPUT | 0x0638/0x06A4 | ⚠️ Pending |
| INKEY$ | 0x0612 | ⚠️ Pending |
| BEEP | 0x0824 | ⚠️ Pending |
| CLS | 0x05A1 | ⚠️ Pending |

See [RUTINAS_ROM.md](RUTINAS_ROM.md) for complete documentation.

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

## Memory Layout

```
0x3800-0x5800  Code área (max 8KB)
0x5800-0x5FEF  Software stack (752 bytes)
0x5FF0-0x5FF1  Stack pointer (little-endian)
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

## Thanks

- [Sharp PC-1500 (TRS-80 PC-2) resource page](http://www.pc1500.com/)
- [Sharp_PC-1500_ROM_Disassembly](https://github.com/Jeff-Birt/Sharp_PC-1500_ROM_Disassembly)
- [Sharp_CE-158](https://github.com/Jeff-Birt/Sharp_CE-158)
- [Schematics](https://www.kaibader.de/sharp-pc-15001600-schematics-collection/)
