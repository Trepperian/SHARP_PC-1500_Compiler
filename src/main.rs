pub mod error;
mod header;
mod lex;
mod parse;
mod semantic_analysis;
mod codegen;

use crate::error::{CompileError, print_error};
use crate::header::Header;
use crate::lex::{Lexer, RemarkLexOption, SpannedToken};
use crate::parse::Parser;
use crate::semantic_analysis::analyze_program;
use crate::codegen::StackCodeGenerator;
use crate::codegen::interpreter::StackMachineInterpreter;
use clap::Parser as ClapParser;
use std::{fs, path::PathBuf};

#[derive(clap::ValueEnum, Clone, Debug)]
enum RemarkMode {
    TrimWhitespace,
    KeepWhole,
}

impl From<RemarkMode> for RemarkLexOption {
    fn from(mode: RemarkMode) -> Self {
        match mode {
            RemarkMode::TrimWhitespace => RemarkLexOption::TrimWhitespace,
            RemarkMode::KeepWhole => RemarkLexOption::KeepWhole,
        }
    }
}

/// A BASIC lexer and parser
#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input BASIC file to process
    #[arg(value_name = "FILE")]
    input: Option<PathBuf>,

    /// Just tokenize the input (output tokens only)
    #[arg(short, long)]
    lex: bool,

    /// Parse the input into an AST instead of compiling
    #[arg(short, long)]
    parse: bool,

    /// Run semantic analysis on the parsed AST (implies --parse)
    #[arg(short, long)]
    analyze: bool,

    /// Generate stack-based intermediate code (código intermedio de pila)
    #[arg(short = 'g', long, conflicts_with = "native_code")]
    stack_code: bool,

    /// Execute the generated stack code (requires --stack-code)
    #[arg(short = 'e', long, requires = "stack_code")]
    execute: bool,

    /// Show verbose execution trace
    #[arg(short = 'v', long, requires = "execute")]
    verbose: bool,

    /// Generate native LH5801 machine code from stack instructions
    #[arg(long, conflicts_with = "stack_code")]
    native_code: bool,

    /// Compile without header (only program bytes)
    #[arg(long)]
    no_header: bool,

    /// Output file for compiled bytes (defaults to a.bin)
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Preserve source parentheses in output
    #[arg(short = 'w', long)]
    preserve_source_wording: bool,

    /// Program name to use in header (defaults to input filename with extension)
    #[arg(short = 'n', long, value_name = "NAME")]
    program_name: Option<String>,

    /// Remark handling mode: trim-whitespace or keep-whole
    #[arg(long, value_enum, default_value_t = RemarkMode::TrimWhitespace)]
    remark_mode: RemarkMode,
}

fn main() {
    let args = Args::parse();

    // Get the input source
    let (input, filename, program_name) = match args.input {
        Some(file_path) => {
            // Read from file
            match fs::read_to_string(&file_path) {
                Ok(content) => {
                    let default_prog_name = file_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("PROGRAM")
                        .to_string();
                    let prog_name = args.program_name.unwrap_or(default_prog_name);
                    (content, file_path.display().to_string(), prog_name)
                }
                Err(e) => {
                    eprintln!("Error reading file {}: {}", file_path.display(), e);
                    std::process::exit(1);
                }
            }
        }
        None => {
            eprintln!("Error: Must specify input file");
            std::process::exit(1);
        }
    };

    // Create lexer and tokenize
    let lexer = Lexer::new(&input, args.remark_mode.into());
    let mut tokens: Vec<SpannedToken> = Vec::new();

    for token_result in lexer {
        match token_result {
            Ok(spanned_token) => tokens.push(spanned_token),
            Err(lex_error) => {
                let compile_error = CompileError::from(lex_error);
                print_error(&compile_error, &filename, &input);
                std::process::exit(1);
            }
        }
    }

    if args.analyze {
        // Parse the tokens into an AST and run semantic analysis
        let mut parser = Parser::new(tokens.into_iter());

        let (program, parse_errors) = parser.parse_with_error_recovery();

        // Report any parse errors but continue if we got some lines
        for parse_error in &parse_errors {
            let compile_error = CompileError::from(parse_error.clone());
            print_error(&compile_error, &filename, &input);
        }

        if !parse_errors.is_empty() {
            eprintln!(
                "\nWarning: {} parse error(s) occurred. Continuing with {} successfully parsed line(s).",
                parse_errors.len(),
                program.num_lines()
            );
            std::process::exit(1);
        }

        match analyze_program(&program) {
            Ok(symbol_table) => {
                println!("Semantic analysis completed successfully!");
                println!("Symbol table: {symbol_table:#?}");
            }
            Err(semantic_error) => {
                let compile_error = CompileError::from(semantic_error);
                print_error(&compile_error, &filename, &input);
                std::process::exit(1);
            }
        }
    } else if args.parse {
        // Parse the tokens into an AST
        let mut parser = Parser::new(tokens.into_iter());

        let (program, parse_errors) = parser.parse_with_error_recovery();

        // Report any parse errors but continue if we got some lines
        for parse_error in &parse_errors {
            let compile_error = CompileError::from(parse_error.clone());
            print_error(&compile_error, &filename, &input);
        }

        if !parse_errors.is_empty() {
            eprintln!(
                "\nWarning: {} parse error(s) occurred. Continuing with {} successfully parsed line(s).",
                parse_errors.len(),
                program.num_lines()
            );
            std::process::exit(1);
        }

        println!("{}", program.show(args.preserve_source_wording));
    } else if args.lex {
        // Just output the tokens
        for (i, token) in tokens.iter().enumerate() {
            print!("{token} ");
            if i < tokens.len() - 1 {
                print!(" ");
            }
        }
        println!();
    } else if args.stack_code {
        // Generate stack-based intermediate code
        let mut parser = Parser::new(tokens.into_iter());

        let (program, parse_errors) = parser.parse_with_error_recovery();

        // Report any parse errors but continue if we got some lines
        for parse_error in &parse_errors {
            let compile_error = CompileError::from(parse_error.clone());
            print_error(&compile_error, &filename, &input);
        }

        if !parse_errors.is_empty() {
            eprintln!(
                "\nWarning: {} parse error(s) occurred. Continuing with {} successfully parsed line(s).",
                parse_errors.len(),
                program.num_lines()
            );
            std::process::exit(1);
        }

        // Generate stack code
        let mut codegen = StackCodeGenerator::new();
        let instructions = codegen.generate(&program);

        // Convert to text
        let stack_code = codegen.to_string();

        // Write to output file
        let output_path = args.output.unwrap_or_else(|| PathBuf::from("a.p"));
        if let Err(e) = fs::write(&output_path, &stack_code) {
            eprintln!("Error writing output file {}: {}", output_path.display(), e);
            std::process::exit(1);
        }

        println!("Código intermedio de pila generado: {}", output_path.display());
        println!("\nTotal de instrucciones: {}", instructions.len());
        
        if !args.execute {
            println!("\nPrimeras 30 instrucciones:");
            for (i, instr) in instructions.iter().take(30).enumerate() {
                println!("{:4}: {}", i + 1, instr.to_string());
            }
        }
        
        // Ejecutar el código si se especifica --execute
        if args.execute {
            println!("\n{}", "=".repeat(60));
            println!("EJECUTANDO EL CÓDIGO GENERADO");
            println!("{}", "=".repeat(60));
            
            let mut interpreter = StackMachineInterpreter::new(instructions);
            interpreter.set_verbose(args.verbose);
            interpreter.ejecuta();
            
            println!("\n{}", "=".repeat(60));
            println!("Para ejecutar nuevamente use: cargo run {} --stack-code --execute", filename);
        } else {
            println!("\nPara ejecutar el código use: cargo run {} --stack-code --execute", filename);
        }
    } else if args.native_code {
        // Generate native LH5801 machine code as binary file (.lh5)
        use crate::codegen::lh5_format;
        
        let mut parser = Parser::new(tokens.into_iter());
        let (program, parse_errors) = parser.parse_with_error_recovery();

        // Report any parse errors but continue if we got some lines
        for parse_error in &parse_errors {
            let compile_error = CompileError::from(parse_error.clone());
            print_error(&compile_error, &filename, &input);
        }

        if !parse_errors.is_empty() {
            eprintln!(
                "\nWarning: {} parse error(s) occurred. Continuing with {} successfully parsed line(s).",
                parse_errors.len(),
                program.num_lines()
            );
            std::process::exit(1);
        }

        // Compilar a código máquina LH5801 en dos pasadas: la primera mide
        // el tamaño real del código, la segunda ya coloca el área de
        // variables justo después (ver `compile_native_two_pass`) — evita
        // que un programa grande corrompa su propio código con escrituras
        // de variables, como llegó a pasar con bathyscaph.bas.
        use crate::codegen::compile_native_two_pass;
        let (load_address, machine_code, _variable_addresses) =
            compile_native_two_pass(&program, 0x4100, 0x57FF);

        println!("Generados {} bytes de código máquina LH5801", machine_code.len());
        println!("Dirección de carga: 0x{:04X}", load_address);

        // Write binary LH5 file (load address + machine code)
        let output_path = args.output.unwrap_or_else(|| PathBuf::from("a.lh5"));
        
        if let Err(e) = lh5_format::write_lh5_file(&output_path, load_address, &machine_code) {
            eprintln!("Error writing output file {}: {}", output_path.display(), e);
            std::process::exit(1);
        }

        let file_size = 4 + machine_code.len(); // header (4 bytes) + code
        println!("\nArchivo binario LH5 generado:");
        println!("  Archivo: {}", output_path.display());
        println!("  Tamaño total: {} bytes", file_size);
        println!("    - Encabezado: 4 bytes");
        println!("    - Código máquina: {} bytes", machine_code.len());
        
        println!("\nEstructura del archivo:");
        println!("  Offset 0x0000-0x0001: Dirección de carga = 0x{:04X} (little-endian)", load_address);
        println!("  Offset 0x0002-0x0003: Longitud código = {} bytes (little-endian)", machine_code.len());
        println!("  Offset 0x0004-0x{:04X}: Código máquina LH5801", file_size - 1);
        
        println!("\nMemoria requerida:");
        println!("  Código: 0x{:04X}-0x{:04X} ({} bytes)", 
                 load_address, 
                 load_address as usize + machine_code.len() - 1,
                 machine_code.len());
        println!("  Pila (registro S): crece hacia abajo desde 0x57FF");
        
        // Show first bytes of machine code
        println!("\nPrimeros 32 bytes del código máquina (hex):");
        for (i, &byte) in machine_code.iter().take(32).enumerate() {
            if i % 16 == 0 && i > 0 {
                println!();
            }
            print!("{:02X} ", byte);
        }
        println!();
        
        println!("\n{}", "=".repeat(70));
        println!("INSTRUCCIONES PARA EL EMULADOR:");
        println!("{}", "=".repeat(70));
        println!("Este archivo debe cargarse con el siguiente código en el emulador:");
        println!();
        println!("fn load_lh5_file(&mut self, path: &Path) -> Result<(), Error> {{");
        println!("    let (load_address, machine_code) = lh5_format::read_lh5_file(path)?;");
        println!("    ");
        println!("    // Cargar código en memoria");
        println!("    let start = load_address as usize;");
        println!("    let end = start + machine_code.len();");
        println!("    self.memory[start..end].copy_from_slice(&machine_code);");
        println!("    ");
        println!("    // Configurar PC para ejecutar");
        println!("    self.cpu.pc = load_address;");
        println!("    ");
        println!("    Ok(())");
        println!("}}");
        println!("{}", "=".repeat(70));
    } else {
        // Default: compile the tokens into an AST and compile to bytes
        let mut parser = Parser::new(tokens.into_iter());

        let (program, parse_errors) = parser.parse_with_error_recovery();

        // Report any parse errors but continue if we got some lines
        for parse_error in &parse_errors {
            let compile_error = CompileError::from(parse_error.clone());
            print_error(&compile_error, &filename, &input);
        }

        if !parse_errors.is_empty() {
            eprintln!(
                "\nWarning: {} parse error(s) occurred. Continuing with {} successfully parsed line(s).",
                parse_errors.len(),
                program.num_lines()
            );
            std::process::exit(1);
        }
        // Generate program bytes
        let mut program_bytes = Vec::new();
        program.write_bytes(&mut program_bytes, args.preserve_source_wording);

        // Create output bytes - with or without header
        let output_bytes = if args.no_header {
            // Just the program bytes without header
            program_bytes
        } else {
            // Create header with program length and prepend it
            let header = Header::new(&program_name, program_bytes.len() as u16);
            let mut output_bytes = header.to_bytes();
            output_bytes.extend(program_bytes);
            output_bytes
        };

        // Write to output file or default a.bin
        let output_path = args.output.unwrap_or_else(|| PathBuf::from("a.bin"));
        if let Err(e) = fs::write(&output_path, &output_bytes) {
            eprintln!("Error writing output file {}: {}", output_path.display(), e);
            std::process::exit(1);
        }
        let header_info = if args.no_header {
            " (no header)"
        } else {
            " (with header)"
        };
        println!(
            "Compiled program written to {}{}",
            output_path.display(),
            header_info
        );
    }
}
