//! Biblioteca del compilador `dull` (BASIC del Sharp PC-1500 → código máquina
//! LH5801).
//!
//! El proyecto se expone como librería, además de como binario, para que dos
//! interfaces de línea de comandos distintas puedan compartir exactamente la
//! misma lógica de compilación sin duplicarla:
//!
//! - `dull` (`src/main.rs`): la CLI clásica basada en banderas (`--native-code`,
//!   `-o`, `--parse`, ...), pensada para scripts y para quien ya conoce las
//!   opciones.
//! - `dull-menu` (`src/bin/dull-menu.rs`): un menú de texto interactivo para
//!   quien prefiera navegar las opciones en vez de recordar banderas.
//!
//! El punto de encuentro de ambas es [`actions`], que implementa cada uno de
//! los modos de compilación (tokenizado clásico, solo lexer, solo parser,
//! análisis semántico, código intermedio de pila, código nativo LH5801) como
//! una función reutilizable.
pub mod actions;
pub mod codegen;
pub mod error;
pub mod header;
pub mod lex;
pub mod parse;
pub mod semantic_analysis;
