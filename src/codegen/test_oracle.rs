//! Arnés de test que ejecuta el código LH5801 generado por
//! [`crate::codegen::lh5801_backend::Lh5801Backend`] contra un emulador real
//! (`ceres-core`, del proyecto hermano `PC-1500-Emulator`, que carga y
//! ejecuta el dump real de la ROM de la Sharp PC-1500).
//!
//! Solo compila bajo `#[cfg(test)]`: sirve para verificar que el backend
//! produce código que un LH5801+ROM reales aceptarían, en vez de razonar
//! únicamente sobre la codificación de bytes esperada.
#![cfg(test)]

use std::io::Write;

use ceres_core::Pc1500;

use crate::codegen::compile_native_two_pass;
use crate::lex::{Lexer, RemarkLexOption};
use crate::parse::Parser;

/// Dirección de carga usada por los tests del oráculo — coincide con
/// `Lh5801Backend::new()`'s `start_address` por defecto, dentro del rango
/// `0x4000..=0x57FF` que acepta `Pc1500::load_lh5_file` (escribe
/// directamente en `standard_user_memory`, que empieza en `0x4000`).
pub const ORACLE_LOAD_ADDR: u16 = 0x4100;

/// Tope de pila usado por los tests del oráculo — coincide con
/// `Lh5801Backend::new()`'s `stack_top` por defecto (registro `S`).
pub const ORACLE_STACK_TOP: u16 = 0x57FF;

/// Carga `code` (empaquetado en formato `.lh5`) en una `Pc1500` nueva,
/// dejándola lista justo antes de ejecutar la primera instrucción real del
/// programa (consume el reset inicial de la CPU, ver `load_and_reset`).
pub fn load(load_addr: u16, code: &[u8]) -> Pc1500 {
    let mut bytes = Vec::with_capacity(4 + code.len());
    bytes.extend_from_slice(&load_addr.to_le_bytes());
    bytes.extend_from_slice(&(code.len() as u16).to_le_bytes());
    bytes.extend_from_slice(code);

    let mut file = tempfile::NamedTempFile::new().expect("no se pudo crear el archivo .lh5 temporal");
    file.write_all(&bytes)
        .expect("no se pudo escribir el archivo .lh5 temporal");
    file.flush().expect("no se pudo hacer flush del archivo .lh5 temporal");

    let mut pc1500 = Pc1500::new();

    // `MemoryBus::new()` precarga un fixture histórico
    // (`bathyscaph.bin`, 906 bytes en 0x40C5-0x444F) usado por el flujo
    // antiguo de BASIC tokenizado — irrelevante para nuestro código
    // máquina nativo, pero antes "invisible" porque el DATA_BASE estático
    // (0x5600) siempre caía muy por detrás de esos 906 bytes. Desde que
    // DATA_BASE se calcula dinámicamente (`compile_native_two_pass`), un
    // programa de test pequeño puede colocar sus variables justo ahí,
    // leyendo bytes de ese fixture en vez de basura-cero real. Limpiar
    // toda la RAM de usuario antes de cargar deja el oráculo determinista
    // e independiente de ese fixture, sea cual sea el tamaño del programa.
    for addr in 0x4000u32..=0x57FF {
        pc1500.write_byte(addr, 0);
    }

    // `Lh5801::new()` deja `reset_flag=true`; el primerísimo `step_cpu()`
    // no ejecuta la primera instrucción del programa cargado, sino que
    // dispara `cpu_internal_reset()`: pone S=0 y salta el PC al vector de
    // reset real de la ROM ($FFFE), pisando lo que ponga `load_lh5_file`.
    // Hay que consumir ese reset antes de cargar y saltar a nuestro código.
    pc1500.step_cpu();

    pc1500
        .load_lh5_file(file.path())
        .expect("el emulador no pudo cargar el .lh5 generado");

    pc1500
}

/// Carga `code` y ejecuta exactamente `instructions` pasos de CPU (uno por
/// instrucción, sin avanzar periféricos). Para tests que ya conocen el
/// número exacto de instrucciones a ejecutar (p.ej. una secuencia de
/// `StackInstruction` escrita a mano) — para un programa completo cuyo
/// tamaño en instrucciones reales es difícil de predecir, usar
/// [`run_lh5_until_exit`].
pub fn run_lh5(load_addr: u16, code: &[u8], instructions: usize) -> Pc1500 {
    let mut pc1500 = load(load_addr, code);
    for _ in 0..instructions {
        pc1500.step_cpu();
    }
    pc1500
}

/// Carga `code` y lo ejecuta hasta justo antes de la primera `RTN` (opcode
/// `0x9A`) que se alcanza con la pila (`S`) de vuelta en `stack_top` — es
/// decir, un `RTN` sin llamador real — sin llegar a ejecutarla, con
/// `max_instructions` como límite de seguridad.
///
/// Todo backend generado por [`crate::codegen::lh5801_backend::Lh5801Backend`]
/// termina en una `RTN` de epílogo cuyo destino real no está garantizado
/// (no venimos de un `CALL` de verdad): al ejecutarla, `S` queda en
/// `stack_top + 2` (no en `stack_top`, porque esa `RTN` hace `pop` de una
/// dirección que nadie empujó) y el PC salta a memoria arbitraria, lo que
/// puede hacer paniquear al emulador (opcode ilegal) o, en teoría,
/// escribir sobre memoria de test. Un programa con un `END`/`STOP`
/// explícito antes del final del código (p.ej. subrutinas definidas
/// después del programa principal, como en un `GOSUB` a una línea
/// posterior) también emite una `RTN` de este tipo *antes* de llegar al
/// final del código generado.
///
/// No basta con mirar cuándo el PC sale del rango de código cargado: una
/// llamada a una rutina ROM (p.ej. `CHAR_OUT` desde `PRINT`) o un `GOSUB`
/// también sacan el PC de ese rango temporalmente antes de volver, y con
/// una llamada pendiente `S` está por debajo de `stack_top` (cada
/// `SJP`/`Call` resta al menos 2) — por eso la condición combina "el
/// siguiente opcode es `RTN`" con "`S == stack_top`" (sin llamadas
/// pendientes), comprobado *antes* de ejecutar ese paso, no después.
pub fn run_lh5_until_exit(load_addr: u16, code: &[u8], stack_top: u16, max_instructions: usize) -> Pc1500 {
    let mut pc1500 = load(load_addr, code);

    for _ in 0..max_instructions {
        let pc = pc1500.cpu().p();
        let next_opcode = pc1500.read_byte(pc.into());
        if next_opcode == 0x9A && pc1500.cpu().s() == stack_top {
            break;
        }
        pc1500.step_cpu();
    }

    pc1500
}

/// Compila una fuente BASIC completa (lexer → parser → IR de pila →
/// backend LH5801) usando el pipeline real de `--native-code`, para tests
/// del oráculo que necesitan probar un programa entero en vez de una
/// secuencia de `StackInstruction` escrita a mano. Entra en pánico si hay
/// errores de lexado/parseo (los tests que la usan controlan la fuente).
pub fn compile_native(source: &str) -> Vec<u8> {
    compile_native_with_addresses(source).0
}

/// Como [`compile_native`], pero además devuelve la tabla de direcciones de
/// variable real que usó la compilación (nombre BASIC → dirección) — para
/// tests que necesitan leer el valor de una variable concreta en memoria
/// sin asumir una dirección fija. Necesario desde que `data_base` se
/// calcula dinámicamente según el tamaño del código
/// ([`compile_native_two_pass`]): la dirección de una variable ya no es la
/// misma para todos los programas, varía con cuánto código generen.
pub fn compile_native_with_addresses(source: &str) -> (Vec<u8>, std::collections::HashMap<String, usize>) {
    let lexer = Lexer::new(source, RemarkLexOption::TrimWhitespace);
    let tokens: Vec<_> = lexer
        .map(|t| t.expect("error de lexado en fuente de test"))
        .collect();

    let mut parser = Parser::new(tokens.into_iter());
    let (program, parse_errors) = parser.parse_with_error_recovery();
    assert!(parse_errors.is_empty(), "errores de parseo en fuente de test: {parse_errors:?}");

    let (_, machine_code, variable_addresses) =
        compile_native_two_pass(&program, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP);
    (machine_code, variable_addresses)
}
