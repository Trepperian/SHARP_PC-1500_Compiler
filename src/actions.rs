//! Los seis modos de compilación del proyecto, factorizados en funciones
//! reutilizables — hasta ahora vivían inline dentro de `fn main()`, cada uno
//! solo alcanzable desde la CLI basada en banderas de `clap`. Al moverlos
//! aquí, tanto `dull` (`src/main.rs`) como `dull-menu`
//! (`src/bin/dull-menu.rs`, el menú interactivo) llaman exactamente al mismo
//! código: ninguno de los dos binarios reimplementa la lógica de
//! compilación, solo deciden cómo recoger los parámetros del usuario (de
//! `argv` o de un menú de texto).
//!
//! Ninguna función de este módulo llama a `std::process::exit`: cada una
//! imprime sus propios diagnósticos (errores de lexado/parseo/semántica,
//! resumen de la compilación...) con el mismo formato que ya tenía `main.rs`,
//! y devuelve `Err(())` si la operación no llegó a completarse. Decidir qué
//! hacer ante ese fallo es cosa de quien llame: `dull` sale del proceso con
//! código 1 (comportamiento idéntico al de antes de este refactor);
//! `dull-menu` simplemente vuelve a mostrar el menú, porque una sesión
//! interactiva no debe cerrarse entera por un solo error de compilación.

use crate::codegen::interpreter::StackMachineInterpreter;
use crate::codegen::{StackCodeGenerator, compile_native_two_pass_with_timing, lh5_format};
use crate::error::{CompileError, print_error};
use crate::header::Header;
use crate::lex::{Lexer, RemarkLexOption, SpannedToken};
use crate::parse::Parser;
use crate::parse::program::Program;
use crate::semantic_analysis::analyze_program;
use std::fs;
use std::path::{Path, PathBuf};

/// Lee el archivo de `path` y lo tokeniza por completo. Común a los seis
/// modos: todos parten de la misma fuente y la misma secuencia de tokens,
/// solo difieren en lo que hacen con ellos a partir de aquí.
fn read_and_lex(
    path: &Path,
    remark_opt: RemarkLexOption,
) -> Result<(String, String, Vec<SpannedToken>), ()> {
    let source =
        fs::read_to_string(path).map_err(|e| eprintln!("Error reading file {}: {}", path.display(), e))?;
    let filename = path.display().to_string();

    let lexer = Lexer::new(&source, remark_opt);
    let mut tokens = Vec::new();
    for token_result in lexer {
        match token_result {
            Ok(spanned_token) => tokens.push(spanned_token),
            Err(lex_error) => {
                print_error(&CompileError::from(lex_error), &filename, &source);
                return Err(());
            }
        }
    }

    Ok((source, filename, tokens))
}

/// Parsea `tokens` con recuperación de errores y los reporta todos si los
/// hay. Igual que en el `main.rs` original: un solo error de parseo hace
/// fallar la operación entera, aunque el resto del programa se haya podido
/// parsear sin problema.
fn parse_with_report(tokens: Vec<SpannedToken>, filename: &str, source: &str) -> Result<Program, ()> {
    let mut parser = Parser::new(tokens.into_iter());
    let (program, parse_errors) = parser.parse_with_error_recovery();

    for parse_error in &parse_errors {
        print_error(&CompileError::from(parse_error.clone()), filename, source);
    }

    if !parse_errors.is_empty() {
        eprintln!(
            "\nWarning: {} parse error(s) occurred. Continuing with {} successfully parsed line(s).",
            parse_errors.len(),
            program.num_lines()
        );
        return Err(());
    }

    Ok(program)
}

/// Modo por defecto de `dull` (sin ninguna bandera de modo): compila a BASIC
/// tokenizado clásico — el formato nativo de la propia PC-1500 (tokens de 2
/// bytes `0xF0xx`/`0xF1xx`), con cabecera `@COM` de 27 bytes salvo que
/// `no_header` sea `true`.
pub fn run_tokenize(
    path: &Path,
    remark_opt: RemarkLexOption,
    output: Option<PathBuf>,
    no_header: bool,
    preserve_source_wording: bool,
    program_name: Option<String>,
) -> Result<(), ()> {
    let (source, filename, tokens) = read_and_lex(path, remark_opt)?;
    let program = parse_with_report(tokens, &filename, &source)?;

    let mut program_bytes = Vec::new();
    program.write_bytes(&mut program_bytes, preserve_source_wording);

    let output_bytes = if no_header {
        program_bytes
    } else {
        let default_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("PROGRAM")
            .to_string();
        let name = program_name.unwrap_or(default_name);
        let header = Header::new(&name, program_bytes.len() as u16);
        let mut output_bytes = header.to_bytes();
        output_bytes.extend(program_bytes);
        output_bytes
    };

    let output_path = output.unwrap_or_else(|| PathBuf::from("a.bin"));
    if let Err(e) = fs::write(&output_path, &output_bytes) {
        eprintln!("Error writing output file {}: {}", output_path.display(), e);
        return Err(());
    }

    let header_info = if no_header { " (no header)" } else { " (with header)" };
    println!("Compiled program written to {}{}", output_path.display(), header_info);
    Ok(())
}

/// Modo `--lex`: vuelca la secuencia de tokens del lexer, sin parsear nada.
pub fn run_lex(path: &Path, remark_opt: RemarkLexOption) -> Result<(), ()> {
    let (_source, _filename, tokens) = read_and_lex(path, remark_opt)?;
    for (i, token) in tokens.iter().enumerate() {
        print!("{token} ");
        if i < tokens.len() - 1 {
            print!(" ");
        }
    }
    println!();
    Ok(())
}

/// Modo `--parse`: parsea y reconstruye el código fuente a partir del AST
/// (`Program::show`), sin ejecutar ningún análisis adicional.
pub fn run_parse(path: &Path, remark_opt: RemarkLexOption, preserve_source_wording: bool) -> Result<(), ()> {
    let (source, filename, tokens) = read_and_lex(path, remark_opt)?;
    let program = parse_with_report(tokens, &filename, &source)?;
    println!("{}", program.show(preserve_source_wording));
    Ok(())
}

/// Modo `--analyze`: parsea y ejecuta el análisis semántico (comprobación de
/// tipos), mostrando la tabla de símbolos resultante.
pub fn run_analyze(path: &Path, remark_opt: RemarkLexOption) -> Result<(), ()> {
    let (source, filename, tokens) = read_and_lex(path, remark_opt)?;
    let program = parse_with_report(tokens, &filename, &source)?;

    match analyze_program(&program) {
        Ok(symbol_table) => {
            println!("Semantic analysis completed successfully!");
            println!("Symbol table: {symbol_table:#?}");
            Ok(())
        }
        Err(semantic_error) => {
            print_error(&CompileError::from(semantic_error), &filename, &source);
            Err(())
        }
    }
}

/// Modo `--stack-code`: genera el código intermedio de pila y, si `execute`
/// es `true`, lo ejecuta con `StackMachineInterpreter` (con traza detallada
/// si además `verbose` es `true`).
pub fn run_stack_code(
    path: &Path,
    remark_opt: RemarkLexOption,
    output: Option<PathBuf>,
    execute: bool,
    verbose: bool,
) -> Result<(), ()> {
    let (source, filename, tokens) = read_and_lex(path, remark_opt)?;
    let program = parse_with_report(tokens, &filename, &source)?;

    let mut codegen = StackCodeGenerator::new();
    let instructions = codegen.generate(&program);
    let stack_code = codegen.to_string();

    let output_path = output.unwrap_or_else(|| PathBuf::from("a.p"));
    if let Err(e) = fs::write(&output_path, &stack_code) {
        eprintln!("Error writing output file {}: {}", output_path.display(), e);
        return Err(());
    }

    println!("Código intermedio de pila generado: {}", output_path.display());
    println!("\nTotal de instrucciones: {}", instructions.len());

    if !execute {
        println!("\nPrimeras 30 instrucciones:");
        for (i, instr) in instructions.iter().take(30).enumerate() {
            println!("{:4}: {}", i + 1, instr.to_string());
        }
        println!(
            "\nPara ejecutar el código generado, repite esta operación indicando que quieres ejecutarlo."
        );
        return Ok(());
    }

    println!("\n{}", "=".repeat(60));
    println!("EJECUTANDO EL CÓDIGO GENERADO");
    println!("{}", "=".repeat(60));

    let mut interpreter = StackMachineInterpreter::new(instructions);
    interpreter.set_verbose(verbose);
    interpreter.ejecuta();

    println!("\n{}", "=".repeat(60));
    Ok(())
}

/// Modo `--native-code`, el camino principal del proyecto: compila a código
/// máquina LH5801 nativo (`compile_native_two_pass_with_timing`) y lo
/// empaqueta en el formato binario `.lh5` (ver `codegen::lh5_format`).
///
/// `authentic_timing`: `None` desactiva el mecanismo de ritmo de ejecución
/// (el `.lh5` generado es exactamente igual que sin él). `Some(n)` lo
/// activa insertando una espera calibrada de `n` vueltas tras cada
/// sentencia compilada, para acercar el ritmo de ejecución al del BASIC
/// tokenizado interpretado en la ROM real — `n =
/// codegen::lh5801_backend::AUTHENTIC_TIMING_DELAY_ITERATIONS` es el valor
/// "auténtico" calibrado a mano; un `n` menor acelera la ejecución sin
/// desactivar el mecanismo del todo (ver el comentario largo de
/// `StackInstruction::AuthenticTimingDelay`).
pub fn run_native_code(
    path: &Path,
    remark_opt: RemarkLexOption,
    output: Option<PathBuf>,
    authentic_timing: Option<u8>,
) -> Result<(), ()> {
    let (source, filename, tokens) = read_and_lex(path, remark_opt)?;
    let program = parse_with_report(tokens, &filename, &source)?;

    // 0x0100-0x47FF: RAM de usuario de una Sharp PC-1500 con expansión
    // CE-161 (18176 bytes) — ver `Lh5801Backend`/`compile_native_two_pass_with_timing`.
    let (load_address, machine_code, _variable_addresses) =
        compile_native_two_pass_with_timing(&program, 0x0100, 0x47FF, authentic_timing);

    println!("Generados {} bytes de código máquina LH5801", machine_code.len());
    println!("Dirección de carga: 0x{:04X}", load_address);
    if let Some(iterations) = authentic_timing {
        let vueltas = if iterations == 1 { "vuelta" } else { "vueltas" };
        println!(
            "Ritmo de ejecución: espera calibrada activada ({iterations} {vueltas} por sentencia)"
        );
    }

    // Aviso temprano si el código generado no cabe en la RAM de usuario REAL
    // de una Sharp PC-1500 con expansión CE-161 (18176 bytes, 0x0100-0x47FF)
    // — no es un error duro: el archivo se sigue escribiendo igualmente,
    // útil para inspección/depuración.
    let real_ram_budget = 0x47FF - 0x0100 + 1usize;
    if machine_code.len() > real_ram_budget {
        eprintln!(
            "AVISO: el código generado ({} bytes) excede la RAM de usuario real de una Sharp PC-1500 con expansión CE-161 ({} bytes, 0x0100-0x47FF) — este programa no cabría en hardware real ni en el emulador, aunque el archivo se escriba igualmente.",
            machine_code.len(),
            real_ram_budget
        );
    }

    let output_path = output.unwrap_or_else(|| PathBuf::from("a.lh5"));
    if let Err(e) = lh5_format::write_lh5_file(&output_path, load_address, &machine_code) {
        eprintln!("Error writing output file {}: {}", output_path.display(), e);
        return Err(());
    }

    let file_size = 4 + machine_code.len();
    println!("\nArchivo binario LH5 generado:");
    println!("  Archivo: {}", output_path.display());
    println!("  Tamaño total: {} bytes", file_size);
    println!("    - Encabezado: 4 bytes");
    println!("    - Código máquina: {} bytes", machine_code.len());

    println!("\nEstructura del archivo:");
    println!(
        "  Offset 0x0000-0x0001: Dirección de carga = 0x{:04X} (little-endian)",
        load_address
    );
    println!(
        "  Offset 0x0002-0x0003: Longitud código = {} bytes (little-endian)",
        machine_code.len()
    );
    println!("  Offset 0x0004-0x{:04X}: Código máquina LH5801", file_size - 1);

    println!("\nMemoria requerida:");
    println!(
        "  Código: 0x{:04X}-0x{:04X} ({} bytes)",
        load_address,
        load_address as usize + machine_code.len() - 1,
        machine_code.len()
    );
    println!("  Pila (registro S): crece hacia abajo desde 0x47FF");

    println!("\nPrimeros 32 bytes del código máquina (hex):");
    for (i, &byte) in machine_code.iter().take(32).enumerate() {
        if i % 16 == 0 && i > 0 {
            println!();
        }
        print!("{:02X} ", byte);
    }
    println!();

    Ok(())
}
