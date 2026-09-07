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
/// `0x0100..=0x47FF` que acepta `Pc1500::load_lh5_file` (escribe
/// directamente en `standard_user_memory`, que con la expansión CE-161
/// empieza en `0x0100`, justo tras el área reservada de 256 bytes del
/// propio módulo — ver el comentario junto a `STANDARD_USER_MEMORY_BEGIN`
/// en `memory.rs` del proyecto hermano).
pub const ORACLE_LOAD_ADDR: u16 = 0x0100;

/// Tope de pila usado por los tests del oráculo — coincide con
/// `Lh5801Backend::new()`'s `stack_top` por defecto (registro `S`).
pub const ORACLE_STACK_TOP: u16 = 0x47FF;

/// Carga `code` (empaquetado en formato `.lh5`) en una `Pc1500` nueva,
/// dejándola lista justo antes de ejecutar la primera instrucción real del
/// programa (consume el reset inicial de la CPU, ver `load_and_reset`).
pub fn load(load_addr: u16, code: &[u8]) -> Pc1500 {
    // El campo de longitud del formato `.lh5` es de 16 bits — sin esta
    // comprobación, `code.len() as u16` desborda en silencio para
    // cualquier programa de 65536 bytes o más (`len() % 65536`),
    // escribiendo una cabecera con una longitud MENOR que el archivo
    // real. `Pc1500::load_lh5_file` confía en esa cabecera y copia solo
    // esos bytes a memoria: el resto del programa nunca se carga, y
    // cualquier salto a una dirección más allá de esa longitud trunca
    // acaba leyendo memoria nunca escrita — un "Illegal opcode" confuso
    // en vez de un fallo claro de este arnés de test. Confirmado contra
    // decathlon.bas (67310 bytes → cabecera con 1774, salto salvaje a
    // memoria no mapeada 13 instrucciones después de arrancar).
    assert!(
        code.len() <= u16::MAX as usize,
        "código demasiado grande para el formato .lh5 del arnés de test ({} bytes, el campo de longitud es de 16 bits, máximo {}) — este programa no puede cargarse con este arnés en absoluto, con o sin CodeTooLarge",
        code.len(),
        u16::MAX
    );

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
    for addr in 0x0100u32..=0x47FF {
        pc1500.write_byte(addr, 0);
    }

    // `load_lh5_file` ya consume internamente el reset pendiente
    // (`reset_flag`) como su primera acción, antes de escribir el código
    // y fijar el PC — llamar aquí a `step_cpu()` A MANO, ANTES de
    // `load_lh5_file`, hacía que ESE `step_cpu()` interno ya no
    // encontrara `reset_flag=true` (ya consumido por el de aquí), así
    // que en vez de repetir el mismo no-op, ejecutaba una instrucción
    // ROM real desde el vector de arranque ($FFFE) antes incluso de leer
    // el archivo — una instrucción extra que la GUI real (que solo llama
    // a `load_lh5_file` una vez, sin este paso previo) nunca ejecuta.
    // Corregido para que este arnés reproduzca la secuencia de arranque
    // real exactamente (no se ha confirmado que esta discrepancia fuera
    // la causa de ningún fallo observado en concreto — es una corrección
    // de fidelidad del arnés por sí misma).
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

/// Carga `code` y lo ejecuta hasta que la CPU entra en HALT (el epílogo
/// automático de todo programa generado, y cualquier `END`/`STOP`
/// explícito — ver `emit_halt` en el backend) o hasta `max_instructions`
/// como límite de seguridad.
///
/// El epílogo/`END`/`STOP` usan la instrucción HALT real del LH5801
/// (`0xFD 0xB1`), no un `RTN` — no venimos de una llamada real (`CALL`),
/// así que no hay ningún destino al que "volver": HALT simplemente para
/// la CPU sin tocar la pila (cada `step_cpu()` posterior se limita a
/// avanzar el reloj, ver `is_halted` en `ceres-core::lh5801`), así que
/// ejecutarla de verdad es seguro — a diferencia de la versión anterior
/// de este helper, que tenía que detenerse *antes* de una `RTN` sin
/// llamador real para evitar que saltara a memoria arbitraria.
pub fn run_lh5_until_exit(load_addr: u16, code: &[u8], _stack_top: u16, max_instructions: usize) -> Pc1500 {
    let mut pc1500 = load(load_addr, code);

    for _ in 0..max_instructions {
        if pc1500.cpu().is_halted() {
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

/// Como [`compile_native`], pero con el mecanismo genérico de ritmo de
/// ejecución activado (`--authentic-timing`, ver
/// `compile_native_two_pass_with_timing`) — para los tests que verifican
/// ese mecanismo específicamente.
pub fn compile_native_with_timing(source: &str) -> Vec<u8> {
    use crate::codegen::compile_native_two_pass_with_timing;

    let lexer = Lexer::new(source, RemarkLexOption::TrimWhitespace);
    let tokens: Vec<_> = lexer
        .map(|t| t.expect("error de lexado en fuente de test"))
        .collect();

    let mut parser = Parser::new(tokens.into_iter());
    let (program, parse_errors) = parser.parse_with_error_recovery();
    assert!(parse_errors.is_empty(), "errores de parseo en fuente de test: {parse_errors:?}");

    let (_, machine_code, _) =
        compile_native_two_pass_with_timing(&program, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP, true);
    machine_code
}
