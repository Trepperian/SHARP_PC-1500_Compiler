//! `dull`: el ejecutable principal del compilador — un menú de texto
//! interactivo, sin banderas que recordar, pensado para poder repartirlo a
//! quien no vaya a usar la línea de comandos habitualmente.
//!
//! Para la CLI clásica basada en banderas (`--native-code`, `-o`, `--parse`,
//! ...), pensada para scripts y para quien ya conoce las opciones, ver el
//! binario hermano `dull-cli` (`src/bin/dull-cli.rs`). Es también lo que
//! ejecuta `cargo run` sin `--bin` durante el desarrollo (fijado como
//! `default-run` en `Cargo.toml`), precisamente porque durante el
//! desarrollo suele ser más rápido pasar los parámetros como argumentos que
//! contestar un menú cada vez.
//!
//! Ambos binarios comparten exactamente la misma lógica de compilación,
//! implementada una sola vez en `dull::actions` — este archivo solo
//! pregunta, muestra el menú, y llama a esas mismas funciones.
//!
//! La primera opción del menú es, deliberadamente, la más simple de las
//! seis: compilar a código máquina nativo (`.lh5`) pidiendo solo el archivo
//! de entrada y, opcionalmente, el de salida. El resto de opciones exponen
//! el resto de modos que ya existían (tokenizado clásico, código intermedio
//! de pila, solo lexer, solo parser, análisis semántico), con las mismas
//! preguntas que sus banderas equivalentes en `dull-cli`.

use dull::actions;
use dull::lex::RemarkLexOption;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Modo de tratamiento de comentarios `REM` que usa todo este binario. La
/// CLI clásica (`dull-cli --remark-mode`) permite elegirlo por si algún
/// fuente concreto lo necesita, pero es una opción rara — para mantener el
/// menú simple, `dull` siempre usa el valor por defecto de la CLI
/// (`TrimWhitespace`) sin preguntar por él en ninguna de las seis opciones.
const REMARK_MODE: RemarkLexOption = RemarkLexOption::TrimWhitespace;

fn main() {
    println!("========================================");
    println!(" dull — Compilador BASIC Sharp PC-1500");
    println!("========================================");

    loop {
        print_menu();
        // `None` significa que `stdin` se agotó (EOF: Ctrl+D, o una entrada
        // canalizada que llegó a su fin) — se trata igual que elegir "0", no
        // como una respuesta vacía repetida: sin esta distinción, cada
        // lectura tras EOF devuelve "" sin bloquear y el `match` de abajo la
        // trataría como "opción no reconocida" para siempre, consumiendo CPU
        // sin parar en vez de terminar (bug real encontrado probando este
        // mismo menú con la entrada canalizada desde un script).
        let Some(choice) = prompt("Elige una opción") else {
            println!("Hasta luego.");
            break;
        };
        match choice.as_str() {
            "1" => native_code_menu(),
            "2" => tokenize_menu(),
            "3" => stack_code_menu(),
            "4" => lex_menu(),
            "5" => parse_menu(),
            "6" => analyze_menu(),
            "0" | "q" | "salir" => {
                println!("Hasta luego.");
                break;
            }
            other => {
                println!("Opción no reconocida: \"{other}\". Elige un número del menú.\n");
            }
        }
    }
}

fn print_menu() {
    println!();
    println!("¿Qué quieres hacer?");
    println!();
    println!("  1) Compilar a código máquina nativo LH5801 (.lh5)");
    println!("  2) Compilar a BASIC tokenizado clásico (.bin)");
    println!("  3) Generar código intermedio de pila (.p)");
    println!("  4) Solo tokenizar (mostrar los tokens del lexer)");
    println!("  5) Solo parsear (mostrar el código reconstruido)");
    println!("  6) Analizar semánticamente (comprobación de tipos)");
    println!("  0) Salir");
    println!();
}

// =============================================================================
// UNA OPCIÓN DE MENÚ POR CADA MODO DE dull::actions
// =============================================================================

/// Opción 1 — la más sencilla a propósito: solo entrada y salida.
fn native_code_menu() {
    let Some(input) = prompt_input_path() else { return };
    let output = prompt_output_path(&default_output_for(&input, "lh5"));
    let authentic_timing = prompt_yes_no(
        "¿Activar el ritmo de ejecución auténtico (--authentic-timing)?",
        false,
    );
    println!();
    let _ = actions::run_native_code(&input, REMARK_MODE, Some(output), authentic_timing);
    pause();
}

fn tokenize_menu() {
    let Some(input) = prompt_input_path() else { return };
    let output = prompt_output_path(&default_output_for(&input, "bin"));
    let with_header = prompt_yes_no("¿Incluir cabecera de programa?", true);
    let program_name = if with_header {
        let raw = prompt("Nombre del programa (vacío = nombre del archivo)").unwrap_or_default();
        if raw.is_empty() { None } else { Some(raw) }
    } else {
        None
    };
    let preserve = prompt_yes_no(
        "¿Preservar los paréntesis tal como están en el fuente?",
        false,
    );
    println!();
    let _ = actions::run_tokenize(&input, REMARK_MODE, Some(output), !with_header, preserve, program_name);
    pause();
}

fn stack_code_menu() {
    let Some(input) = prompt_input_path() else { return };
    let output = prompt_output_path(&default_output_for(&input, "p"));
    let execute = prompt_yes_no("¿Ejecutar el código generado con el intérprete?", false);
    let verbose = execute && prompt_yes_no("¿Traza detallada de la ejecución (verbose)?", false);
    println!();
    let _ = actions::run_stack_code(&input, REMARK_MODE, Some(output), execute, verbose);
    pause();
}

fn lex_menu() {
    let Some(input) = prompt_input_path() else { return };
    println!();
    let _ = actions::run_lex(&input, REMARK_MODE);
    pause();
}

fn parse_menu() {
    let Some(input) = prompt_input_path() else { return };
    let preserve = prompt_yes_no(
        "¿Preservar los paréntesis tal como están en el fuente?",
        false,
    );
    println!();
    let _ = actions::run_parse(&input, REMARK_MODE, preserve);
    pause();
}

fn analyze_menu() {
    let Some(input) = prompt_input_path() else { return };
    println!();
    let _ = actions::run_analyze(&input, REMARK_MODE);
    pause();
}

// =============================================================================
// HELPERS DE ENTRADA POR TERMINAL
// =============================================================================

/// Pide una línea de texto, mostrando `message` como indicación.
///
/// Devuelve `None` únicamente cuando `stdin` se ha agotado de verdad (EOF —
/// Ctrl+D en un terminal interactivo, o el final de una entrada canalizada),
/// nunca para una línea en blanco: `Ok(0)` bytes leídos es la señal de EOF
/// del propio `read_line`, distinta de leer una línea vacía (`Ok(n>0)` con
/// solo el salto de línea). Confundir ambos casos fue un bug real de la
/// primera versión de este archivo: una vez agotada la entrada, cada
/// llamada devolvía cadena vacía sin bloquear, y el menú principal la
/// interpretaba como "opción no reconocida" indefinidamente, en un bucle
/// que consumía CPU sin parar en vez de terminar.
fn prompt(message: &str) -> Option<String> {
    print!("{message}: ");
    io::stdout().flush().ok();
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line.trim().to_string()),
        Err(_) => None,
    }
}

/// Pide una ruta de archivo de entrada, repitiendo hasta que exista. Una
/// línea vacía o el fin de la entrada (`prompt` devolviendo `None`)
/// cancelan la operación y vuelven al menú.
fn prompt_input_path() -> Option<PathBuf> {
    loop {
        let raw = prompt("Archivo BASIC de entrada (vacío para cancelar)")?;
        if raw.is_empty() {
            return None;
        }
        let path = PathBuf::from(&raw);
        if path.is_file() {
            return Some(path);
        }
        println!("No se encontró el archivo \"{raw}\". Inténtalo de nuevo.\n");
    }
}

/// Ruta de salida: una línea vacía (o el fin de la entrada) usa `default`
/// tal cual.
fn prompt_output_path(default: &str) -> PathBuf {
    let raw = prompt(&format!("Archivo de salida [{default}]")).unwrap_or_default();
    if raw.is_empty() {
        PathBuf::from(default)
    } else {
        PathBuf::from(raw)
    }
}

/// Pregunta de sí/no con valor por defecto si se pulsa Enter sin escribir
/// nada, si la respuesta no se reconoce, o si la entrada terminó — para no
/// bloquear el menú ni exigir una respuesta exacta.
fn prompt_yes_no(message: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "S/n" } else { "s/N" };
    let raw = prompt(&format!("{message} ({hint})"))
        .unwrap_or_default()
        .to_lowercase();
    match raw.as_str() {
        "" => default_yes,
        "s" | "si" | "sí" | "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

fn pause() {
    let _ = prompt("\nPulsa Enter para volver al menú");
}

/// Ruta de salida por defecto sugerida para el archivo de entrada `input`,
/// cambiando su extensión por `extension` (p.ej. "programa.bas" → "programa.lh5").
fn default_output_for(input: &Path, extension: &str) -> String {
    input.with_extension(extension).to_string_lossy().into_owned()
}
