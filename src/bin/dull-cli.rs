//! `dull-cli`: la CLI clásica del compilador, basada en banderas de línea de
//! comandos (`--native-code`, `-o`, `--parse`, ...) — pensada para scripts y
//! para quien ya conoce las opciones. Es lo que ejecuta `cargo run` sin
//! `--bin` durante el desarrollo (fijado como `default-run` en
//! `Cargo.toml`): más rápido para probar cambios pasando los parámetros
//! como argumentos que contestar un menú cada vez.
//!
//! Para un menú de texto interactivo (sin recordar banderas) — el
//! ejecutable pensado para repartir, `target/release/dull` — ver el binario
//! hermano `dull` (`src/main.rs`). Ambos comparten exactamente la misma
//! lógica de compilación, implementada una sola vez en `dull::actions`.

use clap::Parser as ClapParser;
use dull::actions;
use dull::lex::RemarkLexOption;
use std::path::PathBuf;

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
// `name = "dull-cli"` explícito: sin esto, clap usa el nombre del PAQUETE
// (`CARGO_PKG_NAME`, "dull") para `--version`, no el del binario — antes de
// que existiera `dull` (el menú) como binario hermano del mismo paquete
// esto era invisible (coincidían), pero ahora `dull-cli --version`
// mostraría "dull 0.1.0" en vez de "dull-cli 0.1.0".
#[command(name = "dull-cli", author, version, about, long_about = None)]
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

    /// (Solo con --native-code) Inserta una espera calibrada tras cada
    /// sentencia compilada, para acercar el ritmo de ejecución al del
    /// BASIC tokenizado interpretado en la ROM real — el código nativo,
    /// sin esto, ejecuta la misma lógica en una fracción del tiempo real
    /// (el intérprete despacha cada sentencia mediante llamadas
    /// vectorizadas y busca cada variable por nombre en tiempo de
    /// ejecución; el código nativo resuelve todo eso en compilación).
    /// Desactivado por defecto: sin esta bandera, el .lh5 generado es
    /// exactamente igual que sin este mecanismo.
    #[arg(long, requires = "native_code")]
    authentic_timing: bool,

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

    let input_path = match args.input {
        Some(path) => path,
        None => {
            eprintln!("Error: Must specify input file");
            std::process::exit(1);
        }
    };

    let remark_opt: RemarkLexOption = args.remark_mode.into();

    // Cada rama delega en `dull::actions`, que es la única implementación
    // real de cada modo — compartida con el binario `dull` (el menú
    // interactivo). El orden de las comprobaciones (analyze > parse > lex >
    // stack_code > native_code > por defecto) es el mismo que tenía este
    // `match` antes del refactor a librería.
    let result = if args.analyze {
        actions::run_analyze(&input_path, remark_opt)
    } else if args.parse {
        actions::run_parse(&input_path, remark_opt, args.preserve_source_wording)
    } else if args.lex {
        actions::run_lex(&input_path, remark_opt)
    } else if args.stack_code {
        actions::run_stack_code(&input_path, remark_opt, args.output, args.execute, args.verbose)
    } else if args.native_code {
        // `run_native_code` distingue "cuántas vueltas de espera" de "activado
        // o no" (ver su comentario) — la bandera de esta CLI sigue siendo un
        // simple interruptor, así que activada equivale al valor "auténtico"
        // por defecto (AUTHENTIC_TIMING_DELAY_ITERATIONS); quien quiera un
        // ritmo intermedio tiene el menú interactivo (`dull`) para elegirlo.
        let authentic_timing = args.authentic_timing.then_some(
            dull::codegen::lh5801_backend::AUTHENTIC_TIMING_DELAY_ITERATIONS,
        );
        actions::run_native_code(&input_path, remark_opt, args.output, authentic_timing)
    } else {
        actions::run_tokenize(
            &input_path,
            remark_opt,
            args.output,
            args.no_header,
            args.preserve_source_wording,
            args.program_name,
        )
    };

    if result.is_err() {
        std::process::exit(1);
    }
}
