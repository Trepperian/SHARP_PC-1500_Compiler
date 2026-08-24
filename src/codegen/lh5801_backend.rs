/// Backend que traduce instrucciones de la Máquina P a código máquina LH5801
///
/// El LH5801 es el procesador de 8 bits del Sharp PC-1500
/// Registros: A (acumulador), X, Y, U (índices de 16 bits), S (stack HW), P (program counter)
///
/// Estrategia de memoria (asume una PC-1500 **con expansión de RAM**,
/// CE-150/CE-158 — ver `codegen::system_memory` para las direcciones de
/// sistema verificadas contra el desensamblado real de la ROM):
/// - `STANDARD_USER_MEMORY` mapea `0x4000-0x57FF` (6144 bytes) — código,
///   variables y pila propia viven ahí, en ventanas fijas y disjuntas:
///   * `start_address` (por defecto `0x4100`): código generado.
///   * `0x5600+`: área de datos de usuario (variables, ver `DATA_BASE` en
///     `codegen::mod` — antes `0x5000`, movido tras confirmar que el
///     código de bathyscaph.bas ya lo alcanzaba y solapaba variables).
///   * `stack_top` (por defecto `0x57FF`): tope de una pila propia, ver
///     abajo.
/// - La pila usa el registro hardware real `S` (no un puntero propio en
///   memoria como antes) — `S` se inicializa en el prólogo a `stack_top`
///   con `LDI S,#imm16` (opcode `0xAA`). Esto es compatible con las
///   rutinas de ROM que llamemos (`ADDIT`, `CHAR_OUT`...), que también usan
///   `SJP`/`PSH`/`POP` sobre `S` internamente — de hecho hace falta que
///   `S` apunte a RAM válida *antes* de la primera llamada a cualquiera de
///   ellas, o la corrompen.
/// - Push/pop de un valor de A usan las instrucciones nativas `PSH A`
///   (`0xFD 0xC8`) / `POP A` (`0xFD 0x8A`), no una secuencia manual.
/// - Direcciones de 16 bits en instrucciones usan big-endian (byte alto
///   primero) — igual que las rutinas de la ROM (`readop_word`).
/// - Y se usa para direcciones de variables (ApilaInd/DesapilaInd)
/// - U se usa como registro temporal (UL para valores intermedios)

use crate::codegen::stack_instruction::StackInstruction;
use crate::codegen::rom_routines::RomRoutines;
use crate::codegen::system_memory;
use std::collections::HashMap;

/// Generador de código LH5801
pub struct Lh5801Backend {
    /// Código máquina generado (opcodes)
    code: Vec<u8>,

    /// Mapa de etiquetas a posiciones en el código
    labels: HashMap<String, usize>,

    /// Referencias a etiquetas que necesitan resolverse
    /// (posición en code, nombre de etiqueta, tipo de referencia)
    label_refs: Vec<(usize, String, RefType)>,

    /// Tabla de rutinas ROM del PC-1500
    rom_routines: RomRoutines,

    /// Dirección de inicio del código generado
    start_address: u16,

    /// Dirección a la que se inicializa el registro hardware S (tope de
    /// una pila propia dedicada, ver doc del módulo).
    stack_top: u16,

    /// Referencias a literales de cadena que se resuelven al final
    /// (literal, posición del byte alto, posición del byte bajo)
    string_refs: Vec<(String, usize, usize)>,

    /// Valores de DATA del programa (`StackInstruction::DataPool`) y mapa
    /// línea→índice (`DataLineTable`), recogidos al procesar esas dos
    /// instrucciones (siempre las primeras del programa, ver
    /// `StackCodeGenerator::generate`) y usados por `ReadData`/
    /// `RestoreData` para generar una búsqueda lineal en tiempo de
    /// compilación (evita aritmética de punteros de 16 bits en tiempo de
    /// ejecución).
    data_pool: Vec<String>,
    data_line_table: Vec<(u16, usize)>,

    /// Todos los números de línea del programa (`StackInstruction::LineTable`,
    /// también siempre al principio), usada por `IrIndirect`/`CallIndirect`
    /// (`GOTO`/`GOSUB` calculado) para generar una búsqueda lineal contra
    /// las etiquetas `LINE_n` reales, mismo patrón que `data_line_table`.
    line_table: Vec<u16>,

    /// Contador para generar nombres de etiqueta únicos internos del
    /// backend (p.ej. el "done" compartido de `ReadData`/`RestoreData`),
    /// distinto del que usa `StackCodeGenerator` a nivel de IR.
    local_label_counter: usize,
}

/// Tipo de referencia a etiqueta
///
/// Solo hay un tipo: todos los saltos a un label (condicionales incluidos,
/// vía el "trampolín" de `IrF`/`IrV`) se resuelven como direcciones
/// absolutas de 16 bits, sin límite de distancia. Antes existían también
/// `BranchBzs`/`BranchBzr` para branches cortos directos (rango ±255), que
/// hacían panic ("Branch offset too large") en programas con cuerpos de
/// bucle/IF grandes — eliminados en favor del trampolín.
#[derive(Debug, Clone, Copy)]
enum RefType {
    /// Salto absoluto de 16 bits (JMP, CALL)
    Absolute16,
}

/// Convierte un `f64` al formato de "número decimal" real de 8 bytes que
/// usa la Sharp PC-1500 en `ARX`/`ARY` (Sharp PC-1500 Technical Manual,
/// §5-3-1 "Expression of decimal number", verificado contra los ejemplos
/// del manual, p.ej. `1500` -> `03 00 15 00 00 00 00 00`):
///   byte 0: exponente (con signo, complemento a 2, sin bias)
///   byte 1: signo de la mantisa (`0x00` positivo, `0x80` negativo)
///   bytes 2-7: 12 dígitos BCD empaquetados (2 por byte, el más
///              significativo primero), mantisa normalizada como
///              `d1.d2d3...d12 x 10^exponente` (`d1` entre 1 y 9, salvo
///              que el valor sea 0, en cuyo caso todo son ceros).
fn f64_to_bcd8(f: f64) -> [u8; 8] {
    let mut bytes = [0u8; 8];
    if f == 0.0 {
        return bytes;
    }

    bytes[1] = if f < 0.0 { 0x80 } else { 0x00 };

    // "{:.11e}" da notación científica normalizada con exactamente un
    // dígito antes del punto y 11 después (12 dígitos significativos en
    // total), p.ej. 10.5 -> "1.05000000000e1", 0.5 -> "5.00000000000e-1".
    let formatted = format!("{:.11e}", f.abs());
    let (mantissa_str, exp_str) = formatted
        .split_once('e')
        .expect("format! con especificador 'e' siempre produce un separador 'e'");
    let exponent: i32 = exp_str.parse().expect("exponente de notación científica inválido");
    let digits: Vec<u8> = mantissa_str
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| b - b'0')
        .collect();
    assert_eq!(digits.len(), 12, "se esperan 12 dígitos significativos de '{{:.11e}}'");

    bytes[0] = exponent as i8 as u8;
    for i in 0..6 {
        bytes[2 + i] = (digits[i * 2] << 4) | digits[i * 2 + 1];
    }

    bytes
}

impl Lh5801Backend {
    /// Crear nuevo backend con configuración por defecto
    pub fn new() -> Self {
        Lh5801Backend {
            code: Vec::new(),
            labels: HashMap::new(),
            label_refs: Vec::new(),
            rom_routines: RomRoutines::new(),
            start_address: 0x4100, // Código: 0x4100-0x4FFF (deja hueco tras RESMEM/PRGMEM real)
            stack_top: 0x57FF,     // Pila propia: crece hacia abajo desde el tope de RAM mapeada
            string_refs: Vec::new(),
            data_pool: Vec::new(),
            data_line_table: Vec::new(),
            line_table: Vec::new(),
            local_label_counter: 0,
        }
    }

    /// Crear backend con configuración personalizada
    pub fn with_config(start_address: u16, stack_top: u16) -> Self {
        Lh5801Backend {
            code: Vec::new(),
            labels: HashMap::new(),
            label_refs: Vec::new(),
            rom_routines: RomRoutines::new(),
            start_address,
            stack_top,
            string_refs: Vec::new(),
            data_pool: Vec::new(),
            data_line_table: Vec::new(),
            line_table: Vec::new(),
            local_label_counter: 0,
        }
    }

    /// Generar un nombre de etiqueta interno único (para saltos generados
    /// por el propio backend, p.ej. el "done" compartido de una búsqueda
    /// lineal de `ReadData`/`RestoreData`).
    fn new_local_label(&mut self, prefix: &str) -> String {
        self.local_label_counter += 1;
        format!("__{prefix}_{}", self.local_label_counter)
    }
    
    /// Generar código LH5801 a partir de instrucciones de pila
    pub fn generate(&mut self, instructions: &[StackInstruction]) -> Vec<u8> {
        // Prólogo: inicializar stack pointer
        self.emit_initialization();
        
        // Primera pasada: generar código y marcar etiquetas
        for (_idx, instr) in instructions.iter().enumerate() {
            // Comentario para debugging (solo si se habilita)
            if let StackInstruction::Comment(_) = instr {
                // Ignorar en código máquina
                continue;
            }

            // Marcar etiquetas
            if let StackInstruction::Label(name) = instr {
                self.define_label(name.clone());
                continue;
            }

            // DataPool/DataLineTable no son código ejecutable: solo
            // guardan los valores de DATA del programa (recogidos por
            // StackCodeGenerator) para que ReadData/RestoreData generen su
            // búsqueda lineal contra ellos.
            if let StackInstruction::DataPool(items) = instr {
                self.data_pool = items.clone();
                continue;
            }
            if let StackInstruction::DataLineTable(table) = instr {
                self.data_line_table = table.clone();
                continue;
            }
            if let StackInstruction::LineTable(table) = instr {
                self.line_table = table.clone();
                continue;
            }

            // Generar código para la instrucción
            self.emit_instruction(instr);
        }
        
        // Epílogo: halt
        self.emit_halt();
        
        // Segunda pasada: resolver referencias a etiquetas
        self.resolve_labels();

        // Tercera pasada: resolver literales de cadena y anexar sección de datos
        self.resolve_string_literals();
        
        self.code.clone()
    }
    
    /// Emitir código de inicialización
    ///
    /// Dos cosas, ambas imprescindibles porque el código generado se carga
    /// y ejecuta directamente (sin pasar por el arranque normal de BASIC,
    /// que es quien normalmente las deja en un estado seguro):
    ///
    /// 1. Inicializar el registro hardware `S` a `stack_top`. Toda rutina
    ///    de ROM que llamemos usa `SJP`/`PSH`/`POP` internamente, así que
    ///    sin esto la primera llamada corrompe memoria no mapeada (`S`
    ///    empieza a 0 tras un reset).
    /// 2. Dejar en un estado limpio el pequeño subconjunto de variables de
    ///    sistema que `CHAR_OUT`/las rutinas de formateo de texto dan por
    ///    hecho (replica el subconjunto relevante de lo que hace
    ///    `INIT_SYS_ADDR` ($CFCC) al arrancar BASIC, sin llamarla
    ///    directamente): `CURSOR_ENA`/`CURSOR_PTR`/`KATAFLAGS` a 0, el
    ///    bloque `USING` a 0 (si no, `CHAR_OUT` puede escribir fuera de la
    ///    pantalla y el formateo de números puede aplicar un formato USING
    ///    arbitrario) y el puntero de escritura de `OUT_BUF` a `0x60`.
    /// 3. Activar la pantalla (`DON`, `0xFD 0xC1`) — un flag de hardware
    ///    real del LH5801 (`disp`, confirmado leyendo `instruction_fd` y
    ///    `Lh5801::new()`/`cpu_internal_reset()` en el emulador: arranca en
    ///    `false` tras el reset y solo un `COLD_START` real de la ROM lo
    ///    activa) del que depende `update_display_buffer()` — con `disp`
    ///    en `false` la pantalla se queda en blanco pase lo que pase en
    ///    memoria de vídeo ($7600/$7700), sin importar lo bien que
    ///    `CHAR_OUT`/`GPRINT` hayan escrito ahí. Sin esta línea ningún
    ///    programa compilado había mostrado nunca nada en pantalla.
    fn emit_initialization(&mut self) {
        // 1. S = stack_top (LDI S,#imm16)
        self.emit_byte(0xAA);
        self.emit_word(self.stack_top);

        // 2. Activar la pantalla.
        self.emit_byte(0xFD); self.emit_byte(0xC1); // DON

        // 3. Estado de sistema limpio para texto/cursor.
        self.emit_byte(0xB5); // LDI A,#0
        self.emit_byte(0x00);
        for addr in [
            system_memory::CURSOR_ENA,
            system_memory::CURSOR_PTR,
            system_memory::KATAFLAGS,
            system_memory::USING_BLOCK,
            system_memory::USING_BLOCK + 1,
            system_memory::USING_BLOCK + 2,
            system_memory::USING_BLOCK + 3,
        ] {
            self.emit_byte(0xAE); // STA addr
            self.emit_word(addr);
        }

        self.emit_byte(0xB5); // LDI A,#0x60
        self.emit_byte(0x60);
        self.emit_byte(0xAE); // STA addr
        self.emit_word(system_memory::OUT_BUF_WRITE_PTR);
    }
    
    /// Emitir instrucción RTN - retorno al llamador (BASIC vía CALL)
    fn emit_halt(&mut self) {
        // 0x9A - RTN: retorna al programa BASIC que ejecutó CALL
        self.emit_byte(0x9A);
    }
    
    /// Definir etiqueta en la posición actual
    fn define_label(&mut self, name: String) {
        let pos = self.code.len() + self.start_address as usize;
        self.labels.insert(name, pos);
    }
    
    /// Añadir referencia a etiqueta para resolver después
    fn add_label_ref(&mut self, name: String, ref_type: RefType) {
        let pos = self.code.len();
        self.label_refs.push((pos, name, ref_type));
    }
    
    /// Resolver todas las referencias a etiquetas
    fn resolve_labels(&mut self) {
        for (pos, label_name, ref_type) in &self.label_refs {
            if let Some(&target_addr) = self.labels.get(label_name) {
                let RefType::Absolute16 = ref_type;
                // Escribir dirección absoluta de 16 bits (big-endian)
                let addr = target_addr as u16;
                self.code[*pos] = (addr >> 8) as u8;
                self.code[*pos + 1] = (addr & 0xFF) as u8;
            } else {
                panic!("Undefined label: {}", label_name);
            }
        }
    }

    /// Resolver literales de cadena y anexarlos al final del binario.
    ///
    /// Estrategia:
    /// - Cada `ApilaCadena` emite dos inmediatos parcheables (high/low de puntero).
    /// - Aquí se asigna dirección a cada literal y se parchean esos bytes.
    /// - Los bytes de cadenas se anexan al final del código en formato C (terminador 0x00).
    fn resolve_string_literals(&mut self) {
        if self.string_refs.is_empty() {
            return;
        }

        let mut pool: HashMap<String, u16> = HashMap::new();
        let mut data_section: Vec<u8> = Vec::new();
        let data_base = self.start_address.wrapping_add(self.code.len() as u16);

        for (literal, high_pos, low_pos) in &self.string_refs {
            let addr = if let Some(addr) = pool.get(literal) {
                *addr
            } else {
                let addr = data_base.wrapping_add(data_section.len() as u16);
                data_section.extend_from_slice(literal.as_bytes());
                data_section.push(0x00);
                pool.insert(literal.clone(), addr);
                addr
            };

            self.code[*high_pos] = (addr >> 8) as u8;
            self.code[*low_pos] = (addr & 0xFF) as u8;
        }

        self.code.extend_from_slice(&data_section);
    }
    
    /// Emitir byte individual
    fn emit_byte(&mut self, byte: u8) {
        self.code.push(byte);
    }
    
    /// Emitir word de 16 bits en orden de instrucción LH5801 (high byte, low byte)
    /// El decodificador del LH5801 lee primero el byte alto y luego el bajo.
    fn emit_word(&mut self, word: u16) {
        self.emit_byte((word >> 8) as u8);        // High byte primero
        self.emit_byte((word & 0xFF) as u8);      // Low byte segundo
    }
    
    /// Emitir placeholder para referencia a etiqueta
    fn emit_label_placeholder(&mut self, ref_type: RefType) {
        match ref_type {
            RefType::Absolute16 => {
                self.emit_word(0x0000); // Placeholder de 2 bytes
            }
        }
    }
    
    /// Emitir llamada a rutina ROM
    /// 
    /// Implementación usando Vector Jump (VEJ) cuando es posible,
    /// o llamada manual guardando dirección de retorno.
    /// 
    /// Nota: Las rutinas ROM del PC-1500 preservan los registros según su documentación.
    /// La mayoría preservan X e Y, modifican A según su función.
    fn emit_call_rom(&mut self, address: u16) {
        // SJP (SubJumP): guarda PC en stack hardware y salta a la dirección
        // Las rutinas ROM terminan con RTN (0x9A) que restaura el PC
        // Opcode: 0xBE + dirección de 16 bits (big endian)
        self.emit_byte(0xBE);
        self.emit_word(address);
    }

    /// Llama a `CHAR_OUT` y revierte inmediatamente después el `SIE`
    /// (activar interrupciones) que esa rutina ejecuta incondicionalmente
    /// al final (ver `rom_routines.rs::CHAR_OUT`).
    ///
    /// Por qué hace falta: nuestro código arranca y se ejecuta sin pasar
    /// por el arranque en frío real de la ROM (`COLD_START`/
    /// `INIT_SYS_ADDR`), así que si las interrupciones quedaran
    /// habilitadas de verdad y alguna llegara a dispararse, saltaría al
    /// manejador real de la ROM, que asume un estado de sistema completo
    /// que nunca inicializamos — mejor no dejar la puerta abierta.
    /// (Nota: un desajuste de pila real en un `INPUT` que sondea ISKEY
    /// mucho tiempo se investigó en su día sospechando primero de esto —
    /// se llegó a instrumentar el emulador directamente para comprobar si
    /// la interrupción de temporizador interna de la CPU se disparaba, y
    /// nunca lo hizo; la causa real resultó ser un problema de la propia
    /// suite de test, no de generación de código — ver la nota de
    /// `test_oracle_input_numeric_and_string_via_simulated_keypresses_on_real_rom`.
    /// Este `RIE` no fue la solución de aquel caso, pero se mantiene
    /// igualmente como higiene razonable frente al hecho documentado y
    /// real de que `CHAR_OUT` sí activa interrupciones.)
    fn emit_call_char_out(&mut self) {
        if let Some(addr) = self.rom_routines.address("CHAR_OUT") {
            self.emit_call_rom(addr);
            self.emit_byte(0xFD); self.emit_byte(0xBE); // RIE
        } else {
            eprintln!("WARNING: Rutina ROM CHAR_OUT no encontrada");
        }
    }

    /// `GOTO`/`GOSUB <expresión>` calculado: pop número de línea (16
    /// bits, bajo primero luego alto — misma convención que
    /// `RestoreData`), y recorre `line_table` (todas las líneas reales
    /// del programa, recogidas por `StackInstruction::LineTable`)
    /// comparando contra cada una, en tiempo de compilación, generando
    /// una búsqueda lineal (mismo patrón que `ReadData`/`RestoreData`,
    /// pero con una etiqueta "siguiente" explícita por entrada en vez de
    /// contar bytes a mano, para no arriesgarse a un offset mal
    /// calculado). Al encontrar la línea, `JMP` (si `is_call=false`,
    /// `GOTO`) o `SJP` (si `is_call=true`, `GOSUB`) a la etiqueta
    /// `LINE_n` real ya definida por `StackCodeGenerator`. Si ninguna
    /// entrada coincide (línea inexistente en tiempo de ejecución):
    /// comportamiento indefinido documentado — la ejecución continúa en
    /// la instrucción siguiente sin saltar, no hay forma segura de
    /// abortar sin más infraestructura de la que dispone hoy este
    /// backend.
    fn emit_indirect_dispatch(&mut self, is_call: bool) {
        self.emit_pop_a();
        self.emit_byte(0x2A); // STA UL (línea, byte bajo)
        self.emit_pop_a();
        self.emit_byte(0x28); // STA UH (línea, byte alto)

        let done_label = self.new_local_label("INDIRECT_DONE");
        let entries = self.line_table.clone();

        for line in entries {
            let hi = (line >> 8) as u8;
            let lo = (line & 0xFF) as u8;
            let target_label = format!("LINE_{line}");
            let next_label = self.new_local_label("INDIRECT_NEXT");

            // Si UH != hi, esta entrada no aplica: saltar a la siguiente.
            self.emit_byte(0xA4); // LDA UH
            self.emit_byte(0xB7); self.emit_byte(hi); // CPI A,#hi
            self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si UH==hi/Z=1, saltar el JMP)
            self.emit_byte(0xBA); // JMP next (si UH!=hi/Z=0)
            self.add_label_ref(next_label.clone(), RefType::Absolute16);
            self.emit_label_placeholder(RefType::Absolute16);

            // Si UL != lo, esta entrada no aplica: saltar a la siguiente.
            self.emit_byte(0x24); // LDA UL
            self.emit_byte(0xB7); self.emit_byte(lo); // CPI A,#lo
            self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si UL==lo/Z=1, saltar el JMP)
            self.emit_byte(0xBA); // JMP next (si UL!=lo/Z=0)
            self.add_label_ref(next_label.clone(), RefType::Absolute16);
            self.emit_label_placeholder(RefType::Absolute16);

            // Coincide: saltar (GOTO) o llamar (GOSUB) al destino real.
            self.emit_byte(if is_call { 0xBE } else { 0xBA }); // SJP o JMP
            self.add_label_ref(target_label, RefType::Absolute16);
            self.emit_label_placeholder(RefType::Absolute16);
            // Tras un SJP (GOSUB), cuando el RETURN vuelva aquí, no
            // seguir probando entradas. Para JMP (GOTO) este JMP es
            // código inalcanzable (el salto anterior ya transfirió el
            // control permanentemente) pero inofensivo.
            self.emit_byte(0xBA); // JMP done
            self.add_label_ref(done_label.clone(), RefType::Absolute16);
            self.emit_label_placeholder(RefType::Absolute16);

            self.define_label(next_label);
        }

        self.define_label(done_label);
    }

    /// PUSH A a la pila hardware real (registro S).
    /// Entrada: A contiene el valor a pushear. Salida: A preservado.
    ///
    /// Antes esto se implementaba a mano (~40 bytes) manteniendo un puntero
    /// de pila propio en memoria (`sp_address`). El LH5801 tiene una
    /// instrucción PSH A nativa (confirmada leyendo el opcode `0xC8` bajo el
    /// prefijo `0xFD` en el propio emulador, y su uso real en la ROM, p.ej.
    /// `PSH X`/`POP X`/`PSH U`/`POP U` dentro de ADDIT/DIVISION) que hace
    /// exactamente esto en 2 bytes usando el registro S real — igual que
    /// hace la propia ROM para su pila, así que es compatible con las
    /// rutinas ROM que llamemos (que también usan PSH/POP/SJP sobre S).
    fn emit_push_a(&mut self) {
        self.emit_byte(0xFD);
        self.emit_byte(0xC8); // PSH A
    }

    /// POP de la pila hardware real (registro S) a A.
    fn emit_pop_a(&mut self) {
        self.emit_byte(0xFD);
        self.emit_byte(0x8A); // POP A
    }

    /// Copia el bloque de 8 bytes en `addr..addr+8` (formato "número
    /// decimal" real, ver `f64_to_bcd8`) al tope de la pila, byte a byte,
    /// en el mismo orden `[0..8)` en que lo deja `emit_pop_8_from_stack_to`
    /// — usado para copiar `ARX` (resultado de `ADDIT`/`SUBTR`/...) de
    /// vuelta a la pila tras una operación real.
    fn emit_push_8_from(&mut self, addr: u16) {
        for i in 0..8u16 {
            self.emit_byte(0xA5); // LDA addr
            self.emit_word(addr + i);
            self.emit_push_a();
        }
    }

    /// Extrae 8 bytes del tope de la pila y los copia a `addr..addr+8`
    /// reconstruyendo el orden original `[0..8)` (el último byte pusheado
    /// es el primero que se hace pop, así que el primer pop va a `addr+7`)
    /// — usado para copiar un operando real de la pila a `ARX`/`ARY` antes
    /// de llamar a una rutina ROM de aritmética BCD.
    fn emit_pop_8_to(&mut self, addr: u16) {
        for i in (0..8u16).rev() {
            self.emit_pop_a();
            self.emit_byte(0xAE); // STA addr
            self.emit_word(addr + i);
        }
    }

    /// `A = A*10 + UL` (`A*10` calculado como `(A<<3)+(A<<1)`, sin bucle:
    /// el multiplicando cabe siempre en un nibble/dígito decimal, 0-9, en
    /// los únicos usos de este helper). Destruye `XL`/`YL` como scratch.
    /// Usado por `CallInt` para reconstruir un entero (base 10, método de
    /// Horner) a partir de los dígitos BCD de un real.
    fn emit_a_times10_plus_ul(&mut self) {
        self.emit_byte(0x0A); // STA XL (V)
        self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x3 -> V<<3
        self.emit_byte(0x1A); // STA YL (V<<3)
        self.emit_byte(0x04); // LDA XL (V)
        self.emit_byte(0xD9); // SHL -> V<<1
        self.emit_byte(0xF9); // REC
        self.emit_byte(0x12); // ADC YL -> A = V<<3 + V<<1 = V*10
        self.emit_byte(0xF9); // REC
        self.emit_byte(0x22); // ADC UL -> A = V*10 + dígito
    }

    /// Convierte el carácter ASCII de un dígito hexadecimal ('0'-'9' o
    /// 'A'-'F') ya en `A` a su valor de nibble (0-15). Usado por
    /// `GPrintString`: el `GPRINT` real de la ROM, al recibir una
    /// cadena, la interpreta como texto de pares de dígitos hex (cada
    /// par = 1 byte = 1 columna de puntos) — no como bytes crudos, que
    /// es lo que hacía este backend antes de verificarlo visualmente
    /// contra el programa real corriendo en la GUI (`bathyscaph.bas`
    /// codifica sus paredes de cueva así: `DATA "7163470F1F0F..."`, un
    /// texto de 20 caracteres = 10 bytes reales, no 20 columnas de
    /// basura). Solo mayúsculas: es lo único que aparece en el corpus
    /// real, y es el convenio con el que las revistas de la época
    /// publicaban estos patrones.
    fn emit_hex_digit_to_nibble(&mut self) {
        self.emit_byte(0x2A); // STA UL (copia del carácter)
        self.emit_byte(0xB7); self.emit_byte(0x3A); // CPI A,#0x3A (':' = '9'+1)

        let is_letter = self.new_local_label("HEXNIB_LETTER");
        let done = self.new_local_label("HEXNIB_DONE");

        // JMP ejecuta cuando Carry=1 => BCR (0x81).
        self.emit_byte(0x81); self.emit_byte(0x03); // BCR +3 (si Carry==0 [dígito], saltar el JMP)
        self.emit_byte(0xBA); // JMP is_letter (si Carry==1, letra 'A'-'F')
        self.add_label_ref(is_letter.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Dígito '0'-'9': nibble = char - '0' (0x30).
        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xFB); // SEC
        self.emit_byte(0xB1); self.emit_byte(0x30); // SBC A,#0x30
        self.emit_byte(0xBA); // JMP done
        self.add_label_ref(done.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Letra 'A'-'F': nibble = char - 'A' (0x41) + 10 = char - 0x37.
        self.define_label(is_letter);
        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xFB); // SEC
        self.emit_byte(0xB1); self.emit_byte(0x37); // SBC A,#0x37

        self.define_label(done);
    }

    /// Extrae los dígitos centenas/decenas/unidades de un entero sin
    /// signo de 8 bits ya en `A` (0-255) por resta repetida (mismo
    /// patrón que `DivInt`) — deja centenas en `UH`, decenas en `UL`,
    /// unidades en `XL`. No toca memoria. Compartido por
    /// `emit_int_a_to_bcd_arx` (Int2Real/CallRnd) y `CallStr` (STR$).
    fn emit_extract_hundreds_tens_units(&mut self) {
        let hundreds_loop = self.new_local_label("EXTRACT_HUNDREDS_LOOP");
        let hundreds_done = self.new_local_label("EXTRACT_HUNDREDS_DONE");
        let tens_loop = self.new_local_label("EXTRACT_TENS_LOOP");
        let tens_done = self.new_local_label("EXTRACT_TENS_DONE");

        self.emit_byte(0x0A); // STA XL

        self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
        self.emit_byte(0x28); // STA UH

        self.define_label(hundreds_loop.clone());
        self.emit_byte(0x04); // LDA XL
        self.emit_byte(0xFB); // SEC
        self.emit_byte(0xB1); self.emit_byte(100); // SBC A,#100
        self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3 (si XL>=100, seguir)
        self.emit_byte(0xBA); // JMP hundreds_done (si XL<100, underflow)
        self.add_label_ref(hundreds_done.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);
        self.emit_byte(0x0A); // STA XL (confirmar XL -= 100)
        self.emit_byte(0xA4); // LDA UH
        self.emit_byte(0xDD); // INC A
        self.emit_byte(0x28); // STA UH
        self.emit_byte(0xBA); // JMP hundreds_loop
        self.add_label_ref(hundreds_loop, RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);
        self.define_label(hundreds_done);

        self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
        self.emit_byte(0x2A); // STA UL

        self.define_label(tens_loop.clone());
        self.emit_byte(0x04); // LDA XL
        self.emit_byte(0xFB); // SEC
        self.emit_byte(0xB1); self.emit_byte(10); // SBC A,#10
        self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3
        self.emit_byte(0xBA); // JMP tens_done
        self.add_label_ref(tens_done.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);
        self.emit_byte(0x0A); // STA XL
        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xDD); // INC A
        self.emit_byte(0x2A); // STA UL
        self.emit_byte(0xBA); // JMP tens_loop
        self.add_label_ref(tens_loop, RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);
        self.define_label(tens_done);
    }

    /// `A = A*10 + Mem[digit_addr]`, usando SOLO `A` y memoria como
    /// scratch (`temp1_addr`, `temp2_addr`) — a diferencia de
    /// `emit_a_times10_plus_ul`, no toca `X`/`Y`, para poder usarse
    /// dentro de un bucle que ya mantiene un puntero activo en `X` o `Y`
    /// (p.ej. `VAL`, que recorre la cadena de entrada mientras acumula
    /// el resultado). Mismo cálculo: `A*10 = (A<<3)+(A<<1)`.
    fn emit_a_times10_plus_mem(&mut self, temp1_addr: u16, temp2_addr: u16, digit_addr: u16) {
        self.emit_byte(0xAE); // STA addr (temp1 = V)
        self.emit_word(temp1_addr);
        self.emit_byte(0xD9); // SHL (A = V<<1)
        self.emit_byte(0xAE); // STA addr (temp2 = V<<1)
        self.emit_word(temp2_addr);
        self.emit_byte(0xA5); // LDA addr (recuperar V)
        self.emit_word(temp1_addr);
        self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x3 (A = V<<3)
        self.emit_byte(0xF9); // REC
        self.emit_byte(0xA3); // ADC addr (A = V<<3 + V<<1 = V*10)
        self.emit_word(temp2_addr);
        self.emit_byte(0xF9); // REC
        self.emit_byte(0xA3); // ADC addr (A = V*10 + dígito)
        self.emit_word(digit_addr);
    }

    /// Copia bytes desde `[X]` hasta `[Y]` (ambos ya posicionados por el
    /// llamador; `UL` = cuenta máxima a copiar), parando en el primer
    /// NUL encontrado (que también se copia) o al agotar `UL` — a
    /// diferencia de `DesapilaIndStringCopy`, SIEMPRE deja el resultado
    /// NUL-terminado: si el contador se agota sin encontrar un NUL en el
    /// origen, añade uno al final. Usado por `LEFT$`/`RIGHT$`/`MID$`.
    /// Destruye `A`, `X`, `Y`, `UL`.
    fn emit_copy_string_x_to_y_terminated(&mut self) {
        let loop_label = self.new_local_label("STRFN_COPY_LOOP");
        let done_label = self.new_local_label("STRFN_COPY_DONE");
        let end_label = self.new_local_label("STRFN_COPY_END");

        self.define_label(loop_label.clone());

        // Si UL==0, terminar (sin NUL copiado todavía: hace falta añadirlo).
        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
        self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UL!=0, saltar el JMP)
        self.emit_byte(0xBA); // JMP done (si UL==0)
        self.add_label_ref(done_label.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Copiar un byte; si era NUL, ya quedó copiado, terminar sin añadir otro.
        self.emit_byte(0x05); // LDA (X)
        self.emit_byte(0x1E); // STA (Y)
        self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
        self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si byte!=0, saltar el JMP)
        self.emit_byte(0xBA); // JMP end (si byte==0, NUL ya copiado)
        self.add_label_ref(end_label.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Avanzar y repetir.
        self.emit_byte(0x44); // X++
        self.emit_byte(0x54); // Y++
        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xDF); // DEC A
        self.emit_byte(0x2A); // STA UL
        self.emit_byte(0xBA); // JMP loop
        self.add_label_ref(loop_label, RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        self.define_label(done_label);
        // UL llegó a 0 sin encontrar NUL: escribir NUL final.
        self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
        self.emit_byte(0x1E); // STA (Y)

        self.define_label(end_label);
    }

    /// Convierte un entero sin signo de 8 bits ya en `A` (0-255) a su
    /// equivalente en formato "número decimal" real de 8 bytes (ver
    /// `f64_to_bcd8`), dejando el resultado directamente en `ARX` — no
    /// apila nada. Dígitos extraídos por resta repetida (mismo patrón
    /// que `DivInt`): el máximo (255) solo tiene 3 dígitos decimales, así
    /// que basta con centenas/decenas/unidades. Usado por `Int2Real`
    /// (pop + este helper + push) y por `CallRnd` (que necesita el
    /// argumento de `RND(n)` ya en `ARX` antes de llamar a `RAND_GEN`,
    /// sin pasar por la pila).
    fn emit_int_a_to_bcd_arx(&mut self) {
        let finish = self.new_local_label("I2R_FINISH");
        let case_hundreds = self.new_local_label("I2R_CASE_HUNDREDS");
        let case_tens = self.new_local_label("I2R_CASE_TENS");

        // Valor restante en XL; centenas en UH; decenas en UL.
        self.emit_extract_hundreds_tens_units();

        // ARX a cero (signo positivo, exponente 0, mantisa vacía); cada
        // caso sobreescribe solo lo que necesita.
        self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
        for offset in 0..8u16 {
            self.emit_byte(0xAE); // STA addr
            self.emit_word(system_memory::ARX + offset);
        }

        // Selección de caso según centenas/decenas (UH/UL siguen
        // intactos; ver comentario de `is_real_expr` en mod.rs).
        self.emit_byte(0xA4); // LDA UH
        self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
        self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si UH==0, saltar el JMP)
        self.emit_byte(0xBA); // JMP case_hundreds (si UH!=0)
        self.add_label_ref(case_hundreds.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
        self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si UL==0, saltar el JMP)
        self.emit_byte(0xBA); // JMP case_tens (si UL!=0)
        self.add_label_ref(case_tens.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Caso "solo unidades" (UH==0, UL==0): un único dígito
        // (posiblemente 0, ya cubierto por el cero inicial).
        self.emit_byte(0x04); // LDA XL
        self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x4 (XL << 4)
        self.emit_byte(0xAE); // STA addr (ARX+2)
        self.emit_word(system_memory::ARX + 2);
        self.emit_byte(0xBA); // JMP finish
        self.add_label_ref(finish.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Caso "centenas" (UH!=0): exponente=2, dígitos UH,UL,XL.
        self.define_label(case_hundreds);
        self.emit_byte(0xB5); self.emit_byte(0x02); // LDI A,#2
        self.emit_byte(0xAE); // STA addr (ARX+0, exponente)
        self.emit_word(system_memory::ARX);
        self.emit_byte(0xA4); // LDA UH
        self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x4
        self.emit_byte(0xF9); // REC
        self.emit_byte(0x22); // ADC UL
        self.emit_byte(0xAE); // STA addr (ARX+2)
        self.emit_word(system_memory::ARX + 2);
        self.emit_byte(0x04); // LDA XL
        self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x4
        self.emit_byte(0xAE); // STA addr (ARX+3)
        self.emit_word(system_memory::ARX + 3);
        self.emit_byte(0xBA); // JMP finish
        self.add_label_ref(finish.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Caso "decenas" (UH==0, UL!=0): exponente=1, dígitos UL,XL.
        self.define_label(case_tens);
        self.emit_byte(0xB5); self.emit_byte(0x01); // LDI A,#1
        self.emit_byte(0xAE); // STA addr (ARX+0, exponente)
        self.emit_word(system_memory::ARX);
        self.emit_byte(0x04); // LDA XL (unidades)
        self.emit_byte(0x28); // STA UH (aparcar unidades en UH libre)
        self.emit_byte(0x24); // LDA UL
        self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x4
        self.emit_byte(0xF9); // REC
        self.emit_byte(0xA2); // ADC UH
        self.emit_byte(0xAE); // STA addr (ARX+2)
        self.emit_word(system_memory::ARX + 2);
        self.emit_byte(0xBA); // JMP finish
        self.add_label_ref(finish.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        self.define_label(finish);
    }

    /// Trunca el real de 8 bytes ya en `ARX` a un entero de 8 bits
    /// (parte entera, hacia cero), dejando el resultado en `A` — no
    /// desapila nada (`ARX` debe estar ya poblado por el llamador) ni
    /// apila el resultado. Reconstruido dígito a dígito desde
    /// exponente+mantisa (método de Horner, ver
    /// `emit_a_times10_plus_ul`). Solo soporta magnitudes de hasta 3
    /// dígitos (exponente 0..=2, |valor|<1000) — el único rango que
    /// producen `Int2Real`/`RAND_GEN` en este backend (enteros de 8
    /// bits, 0-255). Exponente negativo (|valor|<1) trunca a 0;
    /// exponente>=3 no está soportado (documentado, ningún programa
    /// objetivo lo necesita): ambos casos caen en "A=0" por defecto.
    /// Usado por `CallInt` (pop 8 + este helper + push) y por `CallRnd`
    /// (tras `RAND_GEN`, que deja su resultado en `ARX` igual que
    /// `ADDIT`/`MULTIPLY`).
    fn emit_bcd_arx_to_int_a(&mut self) {
        let case_e0 = self.new_local_label("INT_CASE_E0");
        let case_e1 = self.new_local_label("INT_CASE_E1");
        let case_e2 = self.new_local_label("INT_CASE_E2");
        let done = self.new_local_label("INT_DONE");

        self.emit_byte(0xA5); // LDA addr (ARX+0, exponente)
        self.emit_word(system_memory::ARX);

        self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
        self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si A!=0, saltar el JMP)
        self.emit_byte(0xBA); // JMP case_e0 (si A==0)
        self.add_label_ref(case_e0.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        self.emit_byte(0xA5); // LDA addr (ARX+0)
        self.emit_word(system_memory::ARX);
        self.emit_byte(0xB7); self.emit_byte(0x01); // CPI A,#1
        self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3
        self.emit_byte(0xBA); // JMP case_e1
        self.add_label_ref(case_e1.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        self.emit_byte(0xA5); // LDA addr (ARX+0)
        self.emit_word(system_memory::ARX);
        self.emit_byte(0xB7); self.emit_byte(0x02); // CPI A,#2
        self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3
        self.emit_byte(0xBA); // JMP case_e2
        self.add_label_ref(case_e2.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Exponente <0 o >2: no soportado, A=0.
        self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
        self.emit_byte(0xBA); // JMP done
        self.add_label_ref(done.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // exponente=0: 1 dígito (nibble alto de ARX+2).
        self.define_label(case_e0);
        self.emit_byte(0xA5); // LDA addr (ARX+2)
        self.emit_word(system_memory::ARX + 2);
        self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); // SHR x4
        self.emit_byte(0xBA); // JMP done
        self.add_label_ref(done.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // exponente=1: 2 dígitos, d1(nibble alto)*10+d2(nibble bajo) de ARX+2.
        self.define_label(case_e1);
        self.emit_byte(0xA5); // LDA addr (ARX+2)
        self.emit_word(system_memory::ARX + 2);
        self.emit_byte(0x0A); // STA XL (byte completo)
        self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); // SHR x4 -> d1
        self.emit_byte(0x1A); // STA YL (d1 = V, valor acumulado)
        self.emit_byte(0x04); // LDA XL (byte completo)
        self.emit_byte(0xB9); self.emit_byte(0x0F); // ANI A,#0x0F -> d2
        self.emit_byte(0x2A); // STA UL (d2, dígito a sumar)
        self.emit_byte(0x14); // LDA YL (V = d1)
        self.emit_a_times10_plus_ul(); // A = d1*10+d2
        self.emit_byte(0xBA); // JMP done
        self.add_label_ref(done.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // exponente=2: 3 dígitos, d1(ARX+2 alto), d2(ARX+2 bajo), d3(ARX+3 alto).
        self.define_label(case_e2);
        self.emit_byte(0xA5); // LDA addr (ARX+2)
        self.emit_word(system_memory::ARX + 2);
        self.emit_byte(0x0A); // STA XL (byte0 completo)
        self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); // SHR x4 -> d1
        self.emit_byte(0x1A); // STA YL (d1, valor V acumulado)
        self.emit_byte(0x04); // LDA XL (byte0 completo)
        self.emit_byte(0xB9); self.emit_byte(0x0F); // ANI A,#0x0F -> d2
        self.emit_byte(0x2A); // STA UL (dígito a sumar)
        self.emit_byte(0x14); // LDA YL (V = d1)
        self.emit_a_times10_plus_ul(); // A = d1*10+d2
        self.emit_byte(0x1A); // STA YL (V = d1*10+d2)
        self.emit_byte(0xA5); // LDA addr (ARX+3)
        self.emit_word(system_memory::ARX + 3);
        self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); // SHR x4 -> d3
        self.emit_byte(0x2A); // STA UL (d3, dígito a sumar)
        self.emit_byte(0x14); // LDA YL (V)
        self.emit_a_times10_plus_ul(); // A = V*10+d3

        self.define_label(done);
    }

    /// Apilar un puntero (16 bits: high, low) a un literal de cadena. El
    /// contenido real se anexa al final del binario en
    /// `resolve_string_literals` (con deduplicación). Usado tanto por
    /// `ApilaCadena` como por `ReadData` (cada valor de DATA es, en la
    /// pila, indistinguible de un literal de cadena).
    fn emit_push_string_literal(&mut self, s: &str) {
        self.emit_byte(0xB5); // LDI A,#imm
        let high_pos = self.code.len();
        self.emit_byte(0x00);
        self.emit_push_a();

        self.emit_byte(0xB5); // LDI A,#imm
        let low_pos = self.code.len();
        self.emit_byte(0x00);
        self.emit_push_a();

        self.string_refs.push((s.to_string(), high_pos, low_pos));
    }

    /// Compara el CONTENIDO (no los punteros) de dos cadenas apiladas
    /// (Pop puntero b, Pop puntero a), byte a byte hasta que difieran o
    /// ambas lleguen a NUL a la vez. Usada por `IgualCadena`
    /// (`push_one_if_not_equal=false`) y `DistintoCadena`
    /// (`push_one_if_not_equal=true`) — comparten el mismo bucle, solo
    /// cambia qué desenlace empuja 1 y cuál empuja 0.
    fn emit_string_compare(&mut self, push_one_if_not_equal: bool) {
        let loop_label = self.new_local_label("STRCMP_LOOP");
        let not_equal_label = self.new_local_label("STRCMP_NE");
        let done_label = self.new_local_label("STRCMP_DONE");

        // Pop puntero b (16 bits) a Y
        self.emit_pop_a();
        self.emit_byte(0x1A); // STA YL
        self.emit_pop_a();
        self.emit_byte(0x18); // STA YH

        // Pop puntero a (16 bits) a X
        self.emit_pop_a();
        self.emit_byte(0x0A); // STA XL
        self.emit_pop_a();
        self.emit_byte(0x08); // STA XH

        self.define_label(loop_label.clone());

        // UL = byte en (X); A = byte en (Y); A = A - UL (SEC, ver nota en
        // RestaInt) -> Z=1 si son iguales.
        self.emit_byte(0x05); // LDA (X)
        self.emit_byte(0x2A); // STA UL
        self.emit_byte(0x15); // LDA (Y)
        self.emit_byte(0xFB); // SEC
        self.emit_byte(0x20); // SBC UL

        // Si distintos (Z=0), saltar a not_equal.
        self.emit_byte(0x8B); // BZS +3: si Z=1 (iguales), saltar el JMP
        self.emit_byte(0x03);
        self.emit_byte(0xBA); // JMP not_equal (si Z=0/distintos)
        self.add_label_ref(not_equal_label.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Iguales; si además es NUL, ambas cadenas terminan aquí -> iguales.
        self.emit_byte(0x24); // LDA UL (byte_a, == byte_b)
        self.emit_byte(0xB7); // CPI A,#0
        self.emit_byte(0x00);
        self.emit_byte(0x89); // BZR +3: si A != 0 (no NUL), saltar el JMP
        self.emit_byte(0x03);
        self.emit_byte(0xBA); // JMP done_as_equal (si A == 0/NUL)
        let equal_done_label = self.new_local_label("STRCMP_EQ");
        self.add_label_ref(equal_done_label.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // No es NUL: avanzar ambos punteros y seguir comparando.
        self.emit_byte(0x44); // X++
        self.emit_byte(0x54); // Y++
        self.emit_byte(0xBA); // JMP loop
        self.add_label_ref(loop_label, RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        // Desenlaces: cargar 0/1 según corresponda y saltar al final común.
        self.define_label(not_equal_label);
        self.emit_byte(0xB5); // LDI A,#(1 si push_one_if_not_equal, si no 0)
        self.emit_byte(u8::from(push_one_if_not_equal));
        self.emit_byte(0xBA); // JMP done
        self.add_label_ref(done_label.clone(), RefType::Absolute16);
        self.emit_label_placeholder(RefType::Absolute16);

        self.define_label(equal_done_label);
        self.emit_byte(0xB5); // LDI A,#(0 si push_one_if_not_equal, si no 1)
        self.emit_byte(u8::from(!push_one_if_not_equal));

        self.define_label(done_label);
        self.emit_push_a();
    }

    /// Emitir código para una instrucción de pila
    fn emit_instruction(&mut self, instr: &StackInstruction) {
        match instr {
            // ===== INSTRUCCIONES DE PILA BÁSICAS =====
            
            StackInstruction::ApilaInt(n) => {
                // Apilar entero
                if *n >= 0 && *n <= 255 {
                    // Entero de 8 bits
                    self.emit_byte(0xB5); // LDA #imm
                    self.emit_byte(*n as u8);
                    self.emit_push_a();
                } else {
                    // Entero de 16 bits - apilar high byte luego low byte
                    let val = *n as u16;
                    
                    // Push high byte
                    self.emit_byte(0xB5); // LDA #imm
                    self.emit_byte((val >> 8) as u8);
                    self.emit_push_a();
                    
                    // Push low byte
                    self.emit_byte(0xB5); // LDA #imm
                    self.emit_byte((val & 0xFF) as u8);
                    self.emit_push_a();
                }
            }
            
            StackInstruction::Dup => {
                // Duplicar el byte del tope de la pila (usado por ON...
                // GOTO/GOSUB y por ABS() para comparar sin consumir el
                // valor original).
                self.emit_pop_a();
                self.emit_push_a();
                self.emit_push_a();
            }

            StackInstruction::Desapila => {
                // Descartar el byte del tope de la pila sin usarlo (fin de
                // la cadena de comparaciones de ON...GOTO/GOSUB cuando
                // ninguna coincide).
                self.emit_pop_a();
            }

            StackInstruction::ApilaIntWord(n) => {
                // Como ApilaInt en su rama de 16 bits, pero siempre —
                // nunca elige la variante de 1 byte, ni siquiera para 0.
                let val = *n as u16;
                self.emit_byte(0xB5); // LDA #imm
                self.emit_byte((val >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); // LDA #imm
                self.emit_byte((val & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::ApilaReal(r) => {
                // Literal real: codificar en el formato "número decimal"
                // auténtico de 8 bytes (ver `f64_to_bcd8`) y apilarlo como
                // valor inmediato, byte a byte — el tamaño es fijo (a
                // diferencia de una cadena), así que no hace falta pool ni
                // puntero, igual que ApilaInt/ApilaIntWord.
                for byte in f64_to_bcd8(*r) {
                    self.emit_byte(0xB5); // LDI A,#imm
                    self.emit_byte(byte);
                    self.emit_push_a();
                }
            }
            
            StackInstruction::ApilaCadena(s) => {
                self.emit_push_string_literal(s);
            }
            
            StackInstruction::ApilaBool(b) => {
                // Booleano como 0/1
                self.emit_byte(0xB5); // LDA #imm
                self.emit_byte(if *b { 1 } else { 0 });
                self.emit_push_a();
            }
            
            StackInstruction::ApilaInd => {
                // Pop dirección (16 bits), Push valor en esa dirección
                // Formato: high byte, low byte en stack
                // IMPORTANTE: Usar Y para dirección porque X se usa para stack pointer
                
                // 1. Pop low byte de dirección a A
                self.emit_pop_a();
                self.emit_byte(0x1A); // YL = A
                
                // 2. Pop high byte de dirección a A
                self.emit_pop_a();
                self.emit_byte(0x18); // YH = A
                //    Ahora Y contiene la dirección
                
                // 3. Leer valor de [Y] a A
                self.emit_byte(0x15); // LDA (Y)
                
                // 4. Push A al stack
                self.emit_push_a();
            }

            StackInstruction::ApilaIndWord => {
                // Como ApilaInd, pero lee un valor de 16 bits (p.ej. el
                // puntero de una variable de cadena escalar).
                // Mem[dirección] = byte alto, Mem[dirección+1] = byte
                // bajo (misma convención que DesapilaIndWord). Se empuja
                // alto primero, luego bajo — igual que ApilaInt/
                // emit_push_string_literal, para que el resultado sea
                // indistinguible de cualquier otro puntero de cadena en
                // la pila.

                // Pop dirección (16 bits) a Y
                self.emit_pop_a();
                self.emit_byte(0x1A); // YL = A
                self.emit_pop_a();
                self.emit_byte(0x18); // YH = A

                // Leer y empujar byte alto
                self.emit_byte(0x15); // LDA (Y)
                self.emit_push_a();

                // Y++, leer y empujar byte bajo
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x15); // LDA (Y)
                self.emit_push_a();
            }

            StackInstruction::DesapilaInd => {
                // Pop valor, Pop dirección, Mem[dirección] = valor
                // Formato: high byte dir, low byte dir, valor en stack
                // IMPORTANTE: Usar Y para dirección porque X se usa para stack pointer
                
                // 1. Pop valor a guardar
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (guardar valor temporalmente)
                
                // 2. Pop low byte de dirección
                self.emit_pop_a();
                self.emit_byte(0x1A); // YL = A
                
                // 3. Pop high byte de dirección
                self.emit_pop_a();
                self.emit_byte(0x18); // YH = A
                //    Ahora Y contiene la dirección
                
                // 4. Recuperar valor de UL
                self.emit_byte(0x24); // LDA UL
                
                // 5. Almacenar en [Y]
                self.emit_byte(0x1E); // STA (Y)
            }

            StackInstruction::DesapilaIndWord => {
                // Como DesapilaInd, pero el valor es de 16 bits (p.ej. un
                // puntero a cadena). Orden en pila (de fondo a tope):
                // [addr_hi, addr_lo, value_hi, value_lo] — value se apiló
                // después de la dirección, así que se desapila primero.
                // Mem[dirección] = value_hi, Mem[dirección+1] = value_lo
                // (big-endian, igual que el resto de valores de 16 bits
                // de este backend).

                // 1-2. Pop valor (16 bits) a X
                self.emit_pop_a();
                self.emit_byte(0x0A); // STA XL (byte bajo)
                self.emit_pop_a();
                self.emit_byte(0x08); // STA XH (byte alto)

                // 3-4. Pop dirección (16 bits) a Y
                self.emit_pop_a();
                self.emit_byte(0x1A); // STA YL
                self.emit_pop_a();
                self.emit_byte(0x18); // STA YH

                // 5. Mem[Y] = byte alto del valor
                self.emit_byte(0x84); // LDA XH
                self.emit_byte(0x1E); // STA (Y)

                // 6. Y++, Mem[Y] = byte bajo del valor
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0x1E); // STA (Y)
            }

            StackInstruction::DesapilaIndStringCopy(max_len) => {
                // Pop puntero origen (X) y dirección destino (Y), copia
                // hasta max_len bytes byte a byte, parando en el primer
                // NUL (el NUL sí se copia, el resto del buffer se deja
                // como estuviera — memoria nueva empieza a 0, así que en
                // la práctica queda como relleno NUL igualmente salvo que
                // el mismo slot ya tuviera una cadena más larga antes).
                // max_len se acota a 255 (contador de 1 byte, igual que
                // el resto de este backend).
                let loop_label = self.new_local_label("STRCOPY_LOOP");
                let done_label = self.new_local_label("STRCOPY_DONE");

                // Pop puntero origen (16 bits) a X
                self.emit_pop_a();
                self.emit_byte(0x0A); // STA XL
                self.emit_pop_a();
                self.emit_byte(0x08); // STA XH

                // Pop dirección destino (16 bits) a Y
                self.emit_pop_a();
                self.emit_byte(0x1A); // STA YL
                self.emit_pop_a();
                self.emit_byte(0x18); // STA YH

                // UL = contador de bytes restantes
                self.emit_byte(0xB5); // LDI A,#max_len
                self.emit_byte((*max_len).min(255) as u8);
                self.emit_byte(0x2A); // STA UL

                self.define_label(loop_label.clone());

                // Si no quedan bytes por copiar, terminar.
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); // CPI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x89); // BZR +3: si UL != 0, saltar el JMP
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP done
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Copiar un byte; si era NUL, parar aquí (ya copiado).
                self.emit_byte(0x05); // LDA (X)
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0xB7); // CPI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x89); // BZR +3: si el byte no era NUL, saltar el JMP
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP done (byte era NUL: parar)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Avanzar punteros y decrementar contador.
                self.emit_byte(0x44); // X++
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x2A); // STA UL
                self.emit_byte(0xBA); // JMP loop
                self.add_label_ref(loop_label, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(done_label);
            }

            // ===== ARITMÉTICA REAL (BCD, vía ARX/ARY de la ROM) =====
            //
            // Un valor real vive en la pila como 8 bytes crudos (no un
            // puntero: tamaño fijo, igual que ApilaIntWord), en el mismo
            // formato que `ARX`/`ARY` (ver `f64_to_bcd8`). Las cuatro
            // operaciones siguen el mismo patrón: pop b a ARY, pop a a
            // ARX, llamar a la rutina ROM verificada (opera "ARX = ARX op
            // ARY" y dispone el resultado en ARX), push el resultado desde
            // ARX. No comprueban el código de error que devuelven
            // (Carry+UH) — igual que el resto de la aritmética entera de
            // este backend, un desbordamiento/error no está soportado.

            StackInstruction::SumaReal => {
                self.emit_pop_8_to(system_memory::ARY);
                self.emit_pop_8_to(system_memory::ARX);
                let addr = self.rom_routines.address("ADDIT").expect("ADDIT debe estar registrada en rom_routines");
                self.emit_call_rom(addr);
                self.emit_push_8_from(system_memory::ARX);
            }

            StackInstruction::SumaIntWord => {
                // Pop offset (8 bits) -> UL. Pop base (16 bits: low
                // primero -ApilaIntWord empuja high y luego low, así que
                // low queda encima-, luego high) -> XL/XH. Suma de 16
                // bits con acarreo del byte bajo al alto (LDA no toca el
                // Carry, así que sobrevive entre el ADC del byte bajo y
                // el del alto). Push resultado (high, luego low).
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (offset)

                self.emit_pop_a();
                self.emit_byte(0x0A); // STA XL (base, byte bajo)
                self.emit_pop_a();
                self.emit_byte(0x08); // STA XH (base, byte alto)

                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0x22); // ADC UL -> byte bajo del resultado, Carry actualizado
                self.emit_byte(0x0A); // STA XL

                self.emit_byte(0x84); // LDA XH (no toca Carry)
                self.emit_byte(0xB3); self.emit_byte(0x00); // ADC A,#0 -> propaga el acarreo (sin REC)
                self.emit_byte(0x08); // STA XH

                self.emit_byte(0x84); // LDA XH
                self.emit_push_a();
                self.emit_byte(0x04); // LDA XL
                self.emit_push_a();
            }

            StackInstruction::RestaReal => {
                self.emit_pop_8_to(system_memory::ARY);
                self.emit_pop_8_to(system_memory::ARX);
                let addr = self.rom_routines.address("SUBTR").expect("SUBTR debe estar registrada en rom_routines");
                self.emit_call_rom(addr);
                self.emit_push_8_from(system_memory::ARX);
            }

            StackInstruction::MulReal => {
                self.emit_pop_8_to(system_memory::ARY);
                self.emit_pop_8_to(system_memory::ARX);
                let addr = self.rom_routines.address("MULTIPLY").expect("MULTIPLY debe estar registrada en rom_routines");
                self.emit_call_rom(addr);
                self.emit_push_8_from(system_memory::ARX);
            }

            StackInstruction::DivReal => {
                self.emit_pop_8_to(system_memory::ARY);
                self.emit_pop_8_to(system_memory::ARX);
                let addr = self.rom_routines.address("DIVISION").expect("DIVISION debe estar registrada en rom_routines");
                self.emit_call_rom(addr);
                self.emit_push_8_from(system_memory::ARX);
            }

            StackInstruction::Int2Real => {
                // Pop entero sin signo de 8 bits (0-255) y construye su
                // equivalente en formato "número decimal" real de 8
                // bytes directamente en ARX (ver
                // `emit_int_a_to_bcd_arx`), para apilarlo.
                self.emit_pop_a();
                self.emit_int_a_to_bcd_arx();
                self.emit_push_8_from(system_memory::ARX);
            }

            // ===== OPERACIONES ARITMÉTICAS =====

            StackInstruction::SumaInt => {
                // Pop b, Pop a, Push (a + b)
                
                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b en UL)
                
                // 2. Pop a
                self.emit_pop_a();
                // A contiene a
                
                // 3. A = A + UL
                self.emit_byte(0xF9); // REC (Limpiar Carry)
                self.emit_byte(0x22); // ADC UL
                
                // 4. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::RestaInt => {
                // Pop b, Pop a, Push (a - b)
                
                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b en UL)
                
                // 2. Pop a
                self.emit_pop_a();
                // A contiene a
                
                // 3. A = A - UL
                // SEC (Carry=1) antes de SBC, no REC: en este backend SBC
                // calcula A + ~operando + Carry (ver sbc() en ceres-core),
                // así que una resta simple a-b sin borrow extra necesita
                // Carry=1, no Carry=0 — REC aquí computaba a-b-1, un bug
                // confirmado ejecutando contra la ROM real (ver
                // test_oracle_for_next_descending_step_on_real_rom).
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL

                // 4. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::Negativo => {
                // Negación aritmética (unario -): Pop a, Push (0 - a).
                // Necesario para literales/expresiones negativas, p.ej. el
                // STEP de un FOR descendente (STEP -1 parsea como
                // Unary(Minus, 1)). Mismo patrón que RestaInt con a=0.

                // 1. Pop valor a UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A

                // 2. A = 0 - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xB5); // LDI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL

                // 3. Push resultado
                self.emit_push_a();
            }

            StackInstruction::MulInt => {
                // Multiplicación usando bucle simple
                // Pop b, Pop a, Push (a * b)
                // Algoritmo: resultado = 0; while(b > 0) { resultado += a; b--; }
                //
                // La suma acumulada vive en XL, separada del registro A que
                // el bucle usa para decrementar UL (b) en cada iteración —
                // la versión anterior reutilizaba A para ambas cosas y el
                // "LDA UL; DEC A" de cada vuelta pisaba la suma que
                // "ADC UH" acababa de calcular, así que el valor final
                // empujado no era el producto real, sino resto del último
                // decremento de UL. Confirmado ejecutando contra la ROM
                // real (ver test_oracle_array_1d_constant_size_on_real_rom,
                // que multiplica índice*tamaño_elemento).

                // 1. Pop b a UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b en UL)

                // 2. Pop a a UH
                self.emit_pop_a();
                self.emit_byte(0x28); // UH = A (a en UH)

                // 3. Inicializar suma acumulada (XL) a 0
                self.emit_byte(0xB5); // LDI A, #0
                self.emit_byte(0x00);
                self.emit_byte(0x0A); // STA XL

                // 4. Si b == 0, saltar directamente al final (XL ya es 0)
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); // CPI A, #0
                self.emit_byte(0x00);
                self.emit_byte(0x8B); // BZS +11 (Branch Zero Set): salta el cuerpo del bucle (11 bytes)
                self.emit_byte(0x0B);

                // Bucle (11 bytes, empieza aquí):
                // XL += UH (suma acumulada += a)
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xA2); // ADC UH
                self.emit_byte(0x0A); // STA XL

                // UL-- (b--)
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x2A); // STA UL

                // if (UL != 0) goto inicio del bucle
                self.emit_byte(0xB7); // CPI A, #0
                self.emit_byte(0x00);
                self.emit_byte(0x99); // BZR -11 (Branch si Z=0, hacia atrás)
                self.emit_byte(0x0B);

                // 5. Cargar suma acumulada y empujar resultado
                self.emit_byte(0x04); // LDA XL
                self.emit_push_a();
            }
            
            StackInstruction::PowInt => {
                // a^b mediante multiplicación repetida (bucle anidado:
                // exponente veces, multiplicar resultado por la base).
                // Exponentes negativos no están soportados (documentado,
                // fuera de alcance): se tratan como 0 iteraciones, dando
                // 1, igual que el caso b=0 — bathyscaph.bas solo usa
                // `2^H` con H no negativo. Al igual que el resto de la
                // aritmética entera de este backend (ver MulInt), solo
                // caben valores de 8 bits (0-255): la base y el resultado
                // se truncan/envuelven sin aviso si se superan.
                //
                // Registros: UH = base (fijo), UL = contador del bucle
                // externo (exponente), XL = resultado acumulado entre
                // vueltas del bucle externo, YL = contador del bucle
                // interno (una multiplicación XL*UH por vuelta externa).
                // Usa el patrón de trampolín (branch corto hacia delante +
                // JMP absoluto) en todos los saltos para no depender de
                // contar a mano el tamaño de un bucle anidado.

                let outer_end = self.new_local_label("POW_END");
                let outer_loop = self.new_local_label("POW_LOOP");
                let inner_skip = self.new_local_label("POW_MUL_SKIP");

                // 1. Pop b (exponente) a UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL

                // 2. Pop a (base) a UH
                self.emit_pop_a();
                self.emit_byte(0x28); // STA UH

                // 3. resultado (XL) = 1
                self.emit_byte(0xB5); // LDI A,#1
                self.emit_byte(0x01);
                self.emit_byte(0x0A); // STA XL

                // 4. Si exponente == 0 (o negativo, truncado a 0 al hacer
                //    pop como byte), saltar directo al final (resultado=1)
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); // CPI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x89); // BZR +3 (si UL!=0, saltar el JMP)
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP outer_end (solo si UL==0)
                self.add_label_ref(outer_end.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // ===== outer_loop: resultado = resultado * base; exponente--
                self.define_label(outer_loop.clone());

                // Multiplicación interna: XL = XL * UH.
                // Copiar el resultado actual (XL) a YL: será el contador
                // del bucle interno (nº de sumas de UH a realizar).
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0x1A); // STA YL

                // Reiniciar XL a 0: será el acumulador de la multiplicación.
                self.emit_byte(0xB5); // LDI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x0A); // STA XL

                // Si YL == 0 (resultado previo era 0), saltar el bucle
                // interno entero (XL ya es 0, producto correcto).
                self.emit_byte(0x14); // LDA YL
                self.emit_byte(0xB7); // CPI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x89); // BZR +3 (si YL!=0, saltar el JMP)
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP inner_skip (solo si YL==0)
                self.add_label_ref(inner_skip.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                let inner_loop = self.new_local_label("POW_MUL_LOOP");
                self.define_label(inner_loop.clone());

                // XL += UH
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xA2); // ADC UH
                self.emit_byte(0x0A); // STA XL

                // YL--
                self.emit_byte(0x14); // LDA YL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x1A); // STA YL

                // Si YL != 0, volver a inner_loop.
                self.emit_byte(0xB7); // CPI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x8B); // BZS +3 (si YL==0, saltar el JMP)
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP inner_loop (solo si YL!=0)
                self.add_label_ref(inner_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(inner_skip);

                // exponente--; si exponente != 0, volver a outer_loop.
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x2A); // STA UL
                self.emit_byte(0xB7); // CPI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0x8B); // BZS +3 (si exponente==0, saltar el JMP)
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP outer_loop (solo si exponente!=0)
                self.add_label_ref(outer_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(outer_end);

                // 5. Push resultado
                self.emit_byte(0x04); // LDA XL
                self.emit_push_a();
            }

            StackInstruction::DivInt => {
                // División usando bucle simple
                // Pop b (divisor), Pop a (dividendo), Push (a / b)
                // Algoritmo: resultado = 0; while(a >= b) { a -= b; resultado++; }
                
                // 1. Pop b (divisor) a UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A
                
                // Verificar división por cero
                self.emit_byte(0xB7); // CPI A, #0
                self.emit_byte(0x00);
                // Si es 0, resultado = 0 y salir
                self.emit_byte(0x8B); // BZS +0x15 (Branch Zero Set)
                self.emit_byte(0x15); // Saltar ~21 bytes (ajustar si cambia el cuerpo)
                
                // 2. Pop a (dividendo) a UH
                self.emit_pop_a();
                self.emit_byte(0x28); // UH = A
                
                // 3. Inicializar cociente a 0 en YL
                self.emit_byte(0xB5); // LDI A, #0
                self.emit_byte(0x00);
                self.emit_byte(0x1A); // STA YL (0x1A)
                
                // 4. Bucle: while (UH >= UL) { UH -= UL; YL++; }
                // Comparar UH con UL (UH - UL). SEC, no REC: ver nota en
                // RestaInt.
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0xA4); // LDA UH (0xA4)
                self.emit_byte(0x20); // SBC UL (0x20)
                
                // Si resultado < 0 (Carry=0 tras SBC), terminamos
                // Usamos BCR +offset (Branch Carry Reset -> A < B)
                self.emit_byte(0x81);
                self.emit_byte(0x06); // Saltar 6 bytes (STA, LDA, INC, STA, BCH)
                
                // UH = UH - UL (ya está en A tras SBC)
                self.emit_byte(0x28); // STA UH (0x28)
                
                // YL++
                self.emit_byte(0x14); // LDA YL (0x14)
                self.emit_byte(0xDD); // INC A (0xDD)
                self.emit_byte(0x1A); // STA YL (0x1A)
                
                // Repetir loop (incondicional hacia atrás)
                // BCH -i (Branch Always Minus). Opcode 0x9E (100 1 1110)
                self.emit_byte(0x9E); 
                // Salto atrás: REC(1)+LDA(1)+SBC(1)+BCR(2)+STA(1)+LDA(1)+INC(1)+STA(1)+BCH(2) = 11 bytes
                self.emit_byte(0x0B); // Offset positivo magnitud 11
                
                // Fin: Mover YL a A (resultado)
                self.emit_byte(0x14); // LDA YL
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            // ===== CONTROL DE FLUJO =====
            
            StackInstruction::IrA(label) => {
                // JMP absoluto
                self.emit_byte(0xBA); // JMP addr
                self.add_label_ref(label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
            }
            
            StackInstruction::IrF(label) => {
                // Salto si falso (Z=1): pop condición, comparar con 0.
                //
                // Se usa un "trampolín" en vez de un branch corto directo
                // al label: BZR +3 (si la condición es verdadera, saltar
                // el JMP de abajo) seguido de un JMP absoluto de 16 bits
                // (solo se ejecuta si la condición era falsa). El BZR
                // salta una distancia fija y local (siempre 3 bytes, el
                // tamaño del JMP), nunca al label real, así que no tiene
                // el límite de rango (±255) de un branch corto — antes,
                // un branch corto directo al label podía superar ese
                // límite en programas con cuerpos de bucle/IF grandes y
                // hacía panic ("Branch offset too large"), confirmado
                // compilando bathyscaph.bas.
                self.emit_pop_a();
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                self.emit_byte(0x89); // BZR +3 (si Z=0/verdadero, saltar el JMP)
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP absoluto (solo si la condición era falsa)
                self.add_label_ref(label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
            }

            StackInstruction::IrV(label) => {
                // Salto si verdadero (Z=0): mismo trampolín que IrF, con
                // la condición corta invertida (BZS en vez de BZR).
                self.emit_pop_a();
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                self.emit_byte(0x8B); // BZS +3 (si Z=1/falso, saltar el JMP)
                self.emit_byte(0x03);
                self.emit_byte(0xBA); // JMP absoluto (solo si la condición era verdadera)
                self.add_label_ref(label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
            }
            
            StackInstruction::Call(label) => {
                // Llamada a subrutina absoluta.
                // SJP addr guarda retorno en stack HW y transfiere control a la etiqueta.
                self.emit_byte(0xBE); // SJP
                self.add_label_ref(label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
            }

            StackInstruction::IrInd => {
                // RETURN de un GOSUB. Call() ya empujó la dirección de
                // retorno en el stack hardware real (S) vía SJP, así que
                // RTN (0x9A) — la misma instrucción que usa emit_halt() —
                // la recupera y salta ahí. Antes esto caía en el catch-all
                // (NOP), así que GOSUB/RETURN no funcionaba en absoluto.
                self.emit_byte(0x9A); // RTN
            }

            StackInstruction::IrIndirect => {
                self.emit_indirect_dispatch(false);
            }

            StackInstruction::CallIndirect => {
                self.emit_indirect_dispatch(true);
            }

            StackInstruction::ExtendIntToWord => {
                // Pop entero de 8 bits, Push como entero de 16 bits
                // (byte alto = 0).
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (guardar el valor)
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_push_a(); // byte alto = 0
                self.emit_byte(0x24); // LDA UL
                self.emit_push_a(); // byte bajo = valor
            }

            // ===== ENTRADA/SALIDA =====

            StackInstruction::SystemIn => {
                // INPUT: lee una línea de teclado sondeando ISKEY/
                // KEY_2_ASCII (ambas puras, sin dependencia de RAM de
                // sistema — ver rom_routines.rs), con eco por CHAR_OUT,
                // acumulando en IN_BUF ($7BB0, el buffer real de la ROM
                // para líneas de teclado, aquí reutilizado con nuestro
                // propio bucle) hasta ENTER (CR, 0x0D) o hasta agotar
                // IN_BUF_LEN-1 bytes (deja sitio al NUL). Espera a que la
                // tecla se suelte antes de seguir sondeando (evita leer
                // el mismo carácter varias veces mientras se mantiene
                // pulsada). Limitación deliberada, documentada: sin
                // borrado con retroceso — no hay forma segura de "olvidar"
                // un carácter ya ecoado sin gestión de cursor adicional,
                // y ningún programa del corpus objetivo lo necesita para
                // ser jugable. Y/U se preservan defensivamente alrededor
                // de cada llamada a ROM (ninguna de las tres documenta
                // preservación de registros) porque este bucle SÍ depende
                // de que sobrevivan entre llamadas, a diferencia de un
                // uso aislado de CHAR_OUT. Deja en la pila un puntero
                // (16 bits) a IN_BUF, mismo convenio que CallStr/CallChr
                // — el llamador (ver gen_input) decide si eso ya es el
                // valor final (destino de cadena) o si hace falta pasarlo
                // por CallVal (destino numérico).
                self.emit_byte(0xB5); self.emit_byte((system_memory::IN_BUF >> 8) as u8); // LDI A,#hi
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xB5); self.emit_byte((system_memory::IN_BUF & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0x1A); // STA YL

                self.emit_byte(0xB5); self.emit_byte((system_memory::IN_BUF_LEN - 1) as u8); // LDI A,#presupuesto
                self.emit_byte(0x2A); // STA UL

                let poll = self.new_local_label("INPUT_POLL");
                let wait_release = self.new_local_label("INPUT_WAIT_RELEASE");
                let skip_char = self.new_local_label("INPUT_SKIP_CHAR");
                let done = self.new_local_label("INPUT_DONE");

                self.define_label(poll.clone());
                self.emit_byte(0xFD); self.emit_byte(0x98); // PSH Y
                self.emit_byte(0xFD); self.emit_byte(0xA8); // PSH U
                if let Some(addr) = self.rom_routines.address("ISKEY") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM ISKEY no encontrada");
                }
                self.emit_byte(0xFD); self.emit_byte(0x2A); // POP U
                self.emit_byte(0xFD); self.emit_byte(0x1A); // POP Y
                // Z=1 si NO hay tecla -> seguir sondeando. JMP ejecuta
                // cuando Z=1 => BZR (0x89), no BZS (ver tabla verificada
                // en instruction() del emulador: 0x89 salta cuando Z=1,
                // 0x8B salta cuando Z=0 — lo contrario de lo que
                // intuitivamente sugieren sus nombres).
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si Z==0 [hay tecla], saltar el JMP)
                self.emit_byte(0xBA); // JMP poll (si Z==1, sin tecla)
                self.add_label_ref(poll.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0xFD); self.emit_byte(0x98); // PSH Y
                self.emit_byte(0xFD); self.emit_byte(0xA8); // PSH U
                if let Some(addr) = self.rom_routines.address("KEY_2_ASCII") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM KEY_2_ASCII no encontrada");
                }
                self.emit_byte(0xFD); self.emit_byte(0x2A); // POP U
                self.emit_byte(0xFD); self.emit_byte(0x1A); // POP Y
                // A = código ASCII; Carry=1 si en realidad no había tecla
                // (condición de carrera improbable justo tras ISKEY, pero
                // se maneja igualmente volviendo a sondear).
                // JMP ejecuta cuando Carry=1 => BCR (0x81), no BCS
                // (0x81 salta cuando Carry=1, 0x83 salta cuando Carry=0).
                self.emit_byte(0x81); self.emit_byte(0x03); // BCR +3 (si Carry==0 [tecla válida], saltar el JMP)
                self.emit_byte(0xBA); // JMP poll (si Carry==1, sin tecla realmente)
                self.add_label_ref(poll.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x0A); // STA XL (carácter leído, temporal)

                // Esperar a que la tecla se suelte antes de procesar el
                // carácter, para no leerlo varias veces.
                self.define_label(wait_release.clone());
                self.emit_byte(0xFD); self.emit_byte(0x98); // PSH Y
                self.emit_byte(0xFD); self.emit_byte(0xA8); // PSH U
                if let Some(addr) = self.rom_routines.address("ISKEY") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM ISKEY no encontrada");
                }
                self.emit_byte(0xFD); self.emit_byte(0x2A); // POP U
                self.emit_byte(0xFD); self.emit_byte(0x1A); // POP Y
                // Z=1 => soltada. Si Z=0 (aún pulsada), seguir esperando.
                // JMP ejecuta cuando Z=0 => BZS (0x8B).
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si Z==1, saltar el JMP)
                self.emit_byte(0xBA); // JMP wait_release (si Z==0, aún pulsada)
                self.add_label_ref(wait_release, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // ¿CR (ENTER)?
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xB7); self.emit_byte(0x0D); // CPI A,#0x0D
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si char!=CR, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si char==CR)
                self.add_label_ref(done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // ¿Presupuesto agotado? Si es así, ignorar el carácter
                // (sin escribirlo ni ecoarlo) y seguir sondeando.
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                // JMP ejecuta cuando Z=1 (UL==0) => BZR (0x89).
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UL!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP skip_char (si UL==0)
                self.add_label_ref(skip_char.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Escribir en [Y], avanzar Y, decrementar presupuesto.
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x2A); // STA UL

                // Eco por pantalla.
                self.emit_byte(0xFD); self.emit_byte(0x98); // PSH Y
                self.emit_byte(0xFD); self.emit_byte(0xA8); // PSH U
                self.emit_byte(0x04); // LDA XL
                self.emit_call_char_out();
                self.emit_byte(0xFD); self.emit_byte(0x2A); // POP U
                self.emit_byte(0xFD); self.emit_byte(0x1A); // POP Y

                self.define_label(skip_char);
                self.emit_byte(0xBA); // JMP poll
                self.add_label_ref(poll, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(done);
                // NUL-terminar en [Y].
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x1E); // STA (Y)

                // Eco de nueva línea (CR), como el ENTER real en pantalla.
                self.emit_byte(0xFD); self.emit_byte(0x98); // PSH Y
                self.emit_byte(0xFD); self.emit_byte(0xA8); // PSH U
                self.emit_byte(0xB5); self.emit_byte(0x0D); // LDI A,#0x0D
                self.emit_call_char_out();
                self.emit_byte(0xFD); self.emit_byte(0x2A); // POP U
                self.emit_byte(0xFD); self.emit_byte(0x1A); // POP Y

                // Push puntero a IN_BUF (mismo convenio que CallStr/CallChr).
                self.emit_byte(0xB5); self.emit_byte((system_memory::IN_BUF >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); self.emit_byte((system_memory::IN_BUF & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::CallInkey(char_buf, ptr_slot) => {
                // INKEY$: sondeo NO bloqueante y SIN eco (a diferencia de
                // SystemIn) — el patrón real de un bucle de juego
                // (bathyscaph.bas: `Z$=INKEY$:IF Z$=""THEN...`), que
                // necesita leer "¿hay algo pulsado AHORA MISMO?" en cada
                // vuelta sin bloquear a esperar una tecla. Por defecto
                // char_buf[0]=NUL (cadena vacía, "sin tecla"); solo se
                // sobreescribe con el carácter real si ISKEY confirma que
                // hay tecla Y KEY_2_ASCII confirma que sigue habiendo una
                // (misma comprobación de doble carrera que SystemIn).
                let char_buf = *char_buf as u16;
                let ptr_slot = *ptr_slot as u16;

                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0xAE); // STA addr (char_buf[0] = NUL, valor por defecto)
                self.emit_word(char_buf);

                let skip_key = self.new_local_label("INKEY_SKIP");

                if let Some(addr) = self.rom_routines.address("ISKEY") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM ISKEY no encontrada");
                }
                // Z=1 si NO hay tecla -> saltar (dejar la cadena vacía).
                // JMP ejecuta cuando Z=1 => BZR (0x89).
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si Z==0 [hay tecla], saltar el JMP)
                self.emit_byte(0xBA); // JMP skip_key (si Z==1, sin tecla)
                self.add_label_ref(skip_key.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                if let Some(addr) = self.rom_routines.address("KEY_2_ASCII") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM KEY_2_ASCII no encontrada");
                }
                // A = código ASCII; Carry=1 si en realidad no había tecla
                // (condición de carrera improbable justo tras ISKEY).
                // JMP ejecuta cuando Carry=1 => BCR (0x81).
                self.emit_byte(0x81); self.emit_byte(0x03); // BCR +3 (si Carry==0 [tecla válida], saltar el JMP)
                self.emit_byte(0xBA); // JMP skip_key (si Carry==1, sin tecla realmente)
                self.add_label_ref(skip_key.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Tecla válida: char_buf[0] = código ASCII, char_buf[1] = NUL.
                self.emit_byte(0xAE); // STA addr (char_buf[0])
                self.emit_word(char_buf);
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0xAE); // STA addr (char_buf[1])
                self.emit_word(char_buf + 1);

                self.define_label(skip_key);

                // Guardar el puntero a char_buf en ptr_slot (2 bytes),
                // para que ApilaIndWord (ver gen_acc_val) lo desreferencie.
                self.emit_byte(0xB5); self.emit_byte((char_buf >> 8) as u8); // LDI A,#hi
                self.emit_byte(0xAE); // STA addr (ptr_slot alto)
                self.emit_word(ptr_slot);
                self.emit_byte(0xB5); self.emit_byte((char_buf & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0xAE); // STA addr (ptr_slot bajo)
                self.emit_word(ptr_slot + 1);
            }

            StackInstruction::SystemOutInt => {
                // PRINT de un entero de 8 bits CON SIGNO: imprime sus
                // dígitos decimales ('-' primero si es negativo), NO el
                // carácter cuyo código ASCII coincide con el valor (bug
                // real que tenía el antiguo SystemOut, encontrado con el
                // oráculo: PRINT 65 imprimía 'A' en vez de "65"). Mismo
                // truco de signo que ABS()/STEP descendente (bit7 vía
                // AND) y misma extracción de dígitos que CallStr, pero
                // imprimiendo cada dígito directamente en vez de
                // escribirlo en un buffer.
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (copia del valor original)

                self.emit_byte(0xB9); self.emit_byte(0x80); // ANI A,#0x80 (bit de signo)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                let is_negative = self.new_local_label("PRINTINT_NEG");
                let after_sign = self.new_local_label("PRINTINT_AFTER_SIGN");
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si signo==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP is_negative (si signo!=0)
                self.add_label_ref(is_negative.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Positivo: A = valor original sin modificar.
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xBA); // JMP after_sign
                self.add_label_ref(after_sign.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Negativo: imprimir '-', luego A = 0 - UL (magnitud
                // positiva, mismo patrón que Negativo).
                self.define_label(is_negative);
                self.emit_byte(0xB5); self.emit_byte(0x2D); // LDI A,#'-'
                self.emit_call_char_out();
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL

                self.define_label(after_sign);
                self.emit_extract_hundreds_tens_units(); // UH=centenas, UL=decenas, XL=unidades

                let case_hundreds = self.new_local_label("PRINTINT_HUNDREDS");
                let case_tens = self.new_local_label("PRINTINT_TENS");
                let finish = self.new_local_label("PRINTINT_FINISH");

                self.emit_byte(0xA4); // LDA UH (centenas)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si centenas==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_hundreds (si centenas!=0)
                self.add_label_ref(case_hundreds.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x24); // LDA UL (decenas)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si decenas==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_tens (si decenas!=0)
                self.add_label_ref(case_tens.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Solo unidades (incluye el caso 0).
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#'0'
                self.emit_call_char_out();
                self.emit_byte(0xBA); // JMP finish
                self.add_label_ref(finish.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // 3 dígitos.
                self.define_label(case_hundreds);
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#'0'
                self.emit_call_char_out();
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#'0'
                self.emit_call_char_out();
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#'0'
                self.emit_call_char_out();
                self.emit_byte(0xBA); // JMP finish
                self.add_label_ref(finish.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // 2 dígitos.
                self.define_label(case_tens);
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#'0'
                self.emit_call_char_out();
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#'0'
                self.emit_call_char_out();
                // (cae en finish)

                self.define_label(finish);
            }

            StackInstruction::SystemOutString => {
                // PRINT de una cadena: pop puntero (Y), recorrer hasta el
                // primer NUL, CHAR_OUT cada byte. Y/U se preservan
                // defensivamente alrededor de cada llamada a CHAR_OUT
                // (rutina cuya preservación de registros no está
                // documentada, a diferencia de BEEP/TIME_DELAY) para que
                // el puntero de recorrido sobreviva sin importar lo que
                // CHAR_OUT haga internamente.
                self.emit_pop_a();
                self.emit_byte(0x1A); // STA YL
                self.emit_pop_a();
                self.emit_byte(0x18); // STA YH

                let loop_label = self.new_local_label("PRINTSTR_LOOP");
                let done_label = self.new_local_label("PRINTSTR_DONE");

                self.define_label(loop_label.clone());
                self.emit_byte(0x15); // LDA (Y)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si char!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si char==0)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x0A); // STA XL (guardar el carácter, PSH/POP Y no toca A pero sí podría tocar XL indirectamente en CHAR_OUT)
                self.emit_byte(0xFD); self.emit_byte(0x98); // PSH Y
                self.emit_byte(0x04); // LDA XL
                self.emit_call_char_out();
                self.emit_byte(0xFD); self.emit_byte(0x1A); // POP Y

                self.emit_byte(0x54); // Y++
                self.emit_byte(0xBA); // JMP loop
                self.add_label_ref(loop_label, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(done_label);
            }

            StackInstruction::Newline => {
                // No existe una rutina ROM aislada de "nueva línea"; se usa
                // CHAR_OUT con CR (0x0D), igual que hace la propia ROM para
                // avanzar de línea en el buffer de pantalla.
                self.emit_byte(0xB5); // LDI A,#imm
                self.emit_byte(0x0D);

                self.emit_call_char_out();
            }

            // ===== GRÁFICOS / SONIDO / SISTEMA =====

            StackInstruction::Cls => {
                // CLS real: LCD_CLR (pone a 0 el buffer de pantalla) +
                // INIT_CURS (resetea el cursor de texto), mismo orden que
                // usa BCMD_CLS en la ROM.
                if let Some(addr) = self.rom_routines.address("LCD_CLR") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM LCD_CLR no encontrada");
                }
                if let Some(addr) = self.rom_routines.address("INIT_CURS") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM INIT_CURS no encontrada");
                }
            }

            StackInstruction::Clear => {
                // CLEAR real (DEL_STD_VARS) borra la tabla de variables
                // del INTÉRPRETE de la ROM ($7650-$76FF/$77xx), que nunca
                // usamos (nuestras variables viven en DATA_BASE,
                // $5000+) — no-op deliberado, ver comentario en
                // rom_routines.rs::DEL_STD_VARS.
            }

            StackInstruction::OnErrorGoto(_label) => {
                // Versión mínima documentada (ver gen_on_error_goto): no
                // hay ninguna detección automática de errores en tiempo de
                // ejecución todavía, así que ON ERROR GOTO no tiene ningún
                // efecto que generar aquí -- ejecutarlo simplemente no hace
                // nada. Deliberadamente NO se referencia `_label` (ni con
                // `add_label_ref` ni de ninguna otra forma): aunque ahora sí
                // es una etiqueta real (`LINE_n`, ver gen_on_error_goto), no
                // referenciarla evita depender de que ese error handler se
                // alcance también por otro camino normal del programa.
            }

            StackInstruction::Wait => {
                // WAIT n: TIME_DELAY real (U-Reg = nº de ciclos de
                // 15.625 ms). El propio BCMD_WAIT de la ROM en realidad
                // solo guarda el valor para que lo consuma el bucle del
                // intérprete más tarde — no reutilizable directamente —
                // así que se llama a la primitiva real que sí ejecuta la
                // espera en el momento.
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (n)
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x28); // STA UH (parte alta de U-Reg a 0)

                if let Some(addr) = self.rom_routines.address("TIME_DELAY") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM TIME_DELAY no encontrada");
                }
            }

            StackInstruction::Pause => {
                // PAUSE real: muestra el texto (ya impreso por gen_pause
                // vía SystemOut antes de esta instrucción) y continúa sola
                // tras un instante, sin esperar tecla (a diferencia de
                // INPUT). No hay ningún argumento en la pila que fije la
                // duración, así que se usa una pausa breve fija (~0.5s)
                // vía la misma TIME_DELAY real que WAIT.
                self.emit_byte(0xB5); self.emit_byte(32); // LDI A,#32 (~0.5s)
                self.emit_byte(0x2A); // STA UL
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x28); // STA UH

                if let Some(addr) = self.rom_routines.address("TIME_DELAY") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM TIME_DELAY no encontrada");
                }
            }

            StackInstruction::BeepOn | StackInstruction::BeepOff => {
                // BEEP ON/OFF en el hardware real activa/desactiva el clic
                // de teclado y los pitidos de error del propio intérprete
                // de la ROM -- ninguno de los dos existe en código nativo
                // (no pasamos por ese intérprete, y BEEP explícito ya
                // funciona siempre vía la rutina ROM real sin consultar
                // ningún flag). No-op deliberado, sin efecto observable
                // posible en este backend (mismo caso que Clear con
                // DEL_STD_VARS, ver más arriba).
            }

            StackInstruction::Poke => {
                // Pop valor, Pop dirección (16 bits), Mem[dirección] = valor.
                // Sin rutina ROM: escritura directa a memoria absoluta
                // (mismo patrón que DesapilaInd).
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (valor)

                self.emit_pop_a();
                self.emit_byte(0x1A); // YL = A (byte bajo de dirección)

                self.emit_pop_a();
                self.emit_byte(0x18); // YH = A (byte alto de dirección)

                self.emit_byte(0x24); // LDA UL (recuperar valor)
                self.emit_byte(0x1E); // STA (Y)
            }

            StackInstruction::Cursor => {
                // CURSOR n: posiciona el cursor de TEXTO. La propia ROM
                // convierte columna de carácter -> columna de punto
                // multiplicando por 6 (ancho de celda) con exactamente
                // esta secuencia (BCMD_CURSOR, $E846): A=n; UL=n; A=n*2
                // (SHL); A=n*2+n=n*3 (ADC UL); A=n*3*2=n*6 (SHL).
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (n)
                self.emit_byte(0xD9); // SHL (n*2)
                self.emit_byte(0xF9); // REC
                self.emit_byte(0x22); // ADC UL (n*2+n = n*3)
                self.emit_byte(0xD9); // SHL (n*3*2 = n*6)

                self.emit_byte(0xAE); // STA addr (CURSOR_PTR)
                self.emit_word(system_memory::CURSOR_PTR);
                self.emit_byte(0xEB); // ORI addr,#imm (CURSOR_ENA |= 0x01)
                self.emit_word(system_memory::CURSOR_ENA);
                self.emit_byte(0x01);
            }

            StackInstruction::GPrint => {
                // GPRINT de un valor numérico: 1 byte = 1 columna de
                // puntos, en la posición actual de CURSOR_PTR, avanzando
                // el cursor después (GPRINT_OUT no lo avanza por sí solo).
                self.emit_pop_a();
                if let Some(addr) = self.rom_routines.address("GPRINT_OUT") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM GPRINT_OUT no encontrada");
                }
                if let Some(addr) = self.rom_routines.address("MTRX_INC") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM MTRX_INC no encontrada");
                }
            }

            StackInstruction::GPrintString(len) => {
                // GPRINT real de la ROM sobre una cadena: el texto se
                // interpreta como PARES de dígitos hexadecimales — cada
                // par de caracteres es 1 byte real, 1 columna de puntos
                // — no 1 columna por carácter crudo (que es lo que hacía
                // este backend antes de compararlo visualmente contra el
                // programa real ejecutándose en la GUI: bathyscaph.bas
                // codifica sus paredes de cueva así, `DATA
                // "7163470F1F0F..."`, un texto de 20 caracteres = 10
                // bytes/columnas reales, no 20 columnas de basura — ver
                // `emit_hex_digit_to_nibble`). `len` es el número de
                // CARACTERES del buffer (element_size del array de
                // ancho fijo / longitud del literal); el número de
                // columnas reales es `len/2`. Pop puntero (16 bits) a Y,
                // recorrer con LIN Y (carga y auto-incrementa) dos
                // caracteres por columna.
                self.emit_pop_a();
                self.emit_byte(0x1A); // YL
                self.emit_pop_a();
                self.emit_byte(0x18); // YH

                let gprint_out = self.rom_routines.address("GPRINT_OUT");
                let mtrx_inc = self.rom_routines.address("MTRX_INC");

                for _ in 0..(*len / 2) {
                    // Carácter alto -> nibble alto (desplazado 4 bits),
                    // guardado en ARX como scratch transitorio (seguro:
                    // GPRINT nunca se ejecuta a la vez que aritmética
                    // real, que es lo único que usa ARX).
                    self.emit_byte(0x55); // LIN Y (LDA (Y); Y++)
                    self.emit_hex_digit_to_nibble();
                    self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x4
                    self.emit_byte(0xAE); // STA addr (nibble alto)
                    self.emit_word(system_memory::ARX);

                    // Carácter bajo -> nibble bajo, combinado con el alto.
                    self.emit_byte(0x55); // LIN Y (LDA (Y); Y++)
                    self.emit_hex_digit_to_nibble();
                    self.emit_byte(0xAB); // OR addr (combinar con el nibble alto)
                    self.emit_word(system_memory::ARX);

                    if let Some(addr) = gprint_out {
                        self.emit_call_rom(addr);
                    } else {
                        eprintln!("WARNING: Rutina ROM GPRINT_OUT no encontrada");
                    }
                    if let Some(addr) = mtrx_inc {
                        self.emit_call_rom(addr);
                    } else {
                        eprintln!("WARNING: Rutina ROM MTRX_INC no encontrada");
                    }
                }
            }

            StackInstruction::GCursor => {
                // GCURSOR n: posiciona el cursor GRÁFICO — en esta ROM es
                // literalmente el mismo CURSOR_PTR/CURSOR_ENA que el
                // cursor de texto (BCMD_GCURSOR cae directo en la cola de
                // BCMD_CURSOR), pero con la columna de PUNTO (n) sin
                // multiplicar por 6.
                self.emit_pop_a();
                self.emit_byte(0xAE); // STA addr (CURSOR_PTR)
                self.emit_word(system_memory::CURSOR_PTR);
                self.emit_byte(0xEB); // ORI addr,#imm (CURSOR_ENA |= 0x01)
                self.emit_word(system_memory::CURSOR_ENA);
                self.emit_byte(0x01);
            }

            StackInstruction::Beep => {
                // BEEP repeticiones, frecuencia, duración — pila (de
                // arriba abajo, según el orden de empuje en
                // gen_beep): duración, frecuencia, repeticiones.
                // Mapeo a la rutina real BEEP ($E66F, confirmada contra
                // el desensamblado): UL = frecuencia (tono), X-Reg =
                // duración. BEEP preserva Y/X/U (PSH/POP interno), así
                // que repetir la llamada `repeticiones` veces no necesita
                // recargar UL/X entre vueltas — solo el contador (YL, no
                // usado por BEEP) cambia.
                self.emit_pop_a();
                self.emit_byte(0x0A); // STA XL (duración, byte bajo)
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x08); // STA XH (duración, byte alto a 0)

                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (frecuencia/tono)

                self.emit_pop_a();
                self.emit_byte(0x1A); // STA YL (contador de repeticiones)

                let loop_label = self.new_local_label("BEEP_LOOP");
                let done_label = self.new_local_label("BEEP_DONE");

                self.define_label(loop_label.clone());
                self.emit_byte(0x14); // LDA YL
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si YL!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP done_label (si YL==0)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                if let Some(addr) = self.rom_routines.address("BEEP") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM BEEP no encontrada");
                }

                self.emit_byte(0x14); // LDA YL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x1A); // STA YL
                self.emit_byte(0xBA); // JMP loop_label
                self.add_label_ref(loop_label, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(done_label);
            }

            StackInstruction::Random(seed_addr) => {
                // RANDOM: mismo LFSR mock que RND() (ver CallRnd) —
                // resembrar de verdad exigiría una fuente de entropía real
                // (no hay ninguna verificada, ver historial de RAND_GEN en
                // rom_routines.rs), así que aquí simplemente se avanza el
                // LFSR un paso extra sobre la MISMA semilla compartida, con
                // la misma guardia de "semilla==0 -> 1" por si RANDOM se
                // llama antes que cualquier RND() (arranque en frío).
                let seed_addr = *seed_addr as u16;

                let seed_nonzero = self.new_local_label("RANDOM_SEED_NONZERO");
                self.emit_byte(0xA5); // LDA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si semilla==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP seed_nonzero (si semilla!=0)
                self.add_label_ref(seed_nonzero.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.emit_byte(0xB5); self.emit_byte(0x01); // LDI A,#1
                self.emit_byte(0xAE); // STA addr (semilla)
                self.emit_word(seed_addr);
                self.define_label(seed_nonzero);

                // Avanzar LFSR un paso (mismo polinomio/máscara 0xB8 que
                // CallRnd): UL = valor original, A = bit0 de una copia.
                self.emit_byte(0xA5); // LDA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0x2A); // STA UL (valor original)
                self.emit_byte(0xB9); self.emit_byte(0x01); // ANI A,#1 (bit0)

                let lfsr_odd = self.new_local_label("RANDOM_LFSR_ODD");
                let lfsr_done = self.new_local_label("RANDOM_LFSR_DONE");
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si bit0==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP lfsr_odd (si bit0==1)
                self.add_label_ref(lfsr_odd.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xD5); // SHR
                self.emit_byte(0xAE); // STA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0xBA); // JMP lfsr_done
                self.add_label_ref(lfsr_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(lfsr_odd);
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xD5); // SHR
                self.emit_byte(0xBD); self.emit_byte(0xB8); // EOR A,#0xB8
                self.emit_byte(0xAE); // STA addr (semilla)
                self.emit_word(seed_addr);

                self.define_label(lfsr_done);
            }

            // ===== OPERACIONES DE COMPARACIÓN =====
            
            StackInstruction::MayorInt => {
                // Pop b, Pop a, Push (a > b ? 1 : 0)
                // Implementación: a > b ⟺ a - b > 0

                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)

                // 2. Pop a
                self.emit_pop_a();
                // A contiene a

                // 3. Comparar: A - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                
                // 4. a > b requiere: carry=1 (no borrow) y resultado != 0
                // Primero descartar a < b (carry reset)
                self.emit_byte(0x81); // BCR +8 -> cargar 0
                self.emit_byte(0x08);

                // Ahora descartar igualdad
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                self.emit_byte(0x8B); // BZ +4
                self.emit_byte(0x04);
                
                // Si A > 0, cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                // Saltar 2 bytes
                self.emit_byte(0x8E); // BCH +2
                self.emit_byte(0x02);
                
                // Cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::MenorInt => {
                // Pop b, Pop a, Push (a < b ? 1 : 0)
                // Implementación: a < b ⟺ a - b < 0

                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)

                // 2. Pop a
                self.emit_pop_a();

                // 3. Comparar: A - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si resultado negativo (sin carry), push 1
                // Carry=0 significa que hubo borrow (a < b)
                self.emit_byte(0x83); // BCS +4
                self.emit_byte(0x04);
                
                // Sin carry (a < b), cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                // Saltar 2 bytes
                self.emit_byte(0x8E); // BCH +2
                self.emit_byte(0x02);
                
                // Con carry (a >= b), cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::IgualInt => {
                // Pop b, Pop a, Push (a == b ? 1 : 0)

                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)

                // 2. Pop a
                self.emit_pop_a();

                // 3. Comparar: A - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si Z=1, push 1; sino push 0
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                
                self.emit_byte(0x8B); // BZ +4
                self.emit_byte(0x04);
                
                // No igual, cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                // Saltar 2 bytes
                self.emit_byte(0x8E); // BCH +2
                self.emit_byte(0x02);
                
                // Igual, cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::DistintoInt => {
                // Pop b, Pop a, Push (a != b ? 1 : 0)

                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)

                // 2. Pop a
                self.emit_pop_a();

                // 3. Comparar: A - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si Z=0, push 1; sino push 0
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                
                self.emit_byte(0x89); // BNZ +4
                self.emit_byte(0x04);
                
                // Igual, cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                // Saltar 2 bytes
                self.emit_byte(0x8E); // BCH +2
                self.emit_byte(0x02);
                
                // No igual, cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::IgualCadena => {
                self.emit_string_compare(false);
            }

            StackInstruction::DistintoCadena => {
                self.emit_string_compare(true);
            }

            StackInstruction::CallAsc => {
                // ASC(s): código ASCII del primer carácter. Pop puntero
                // (16 bits) a Y, empujar el byte en [Y].
                self.emit_pop_a();
                self.emit_byte(0x1A); // YL
                self.emit_pop_a();
                self.emit_byte(0x18); // YH
                self.emit_byte(0x15); // LDA (Y)
                self.emit_push_a();
            }

            StackInstruction::CallChr(buf) => {
                // CHR$(n): un único carácter (código ASCII n) + NUL, en un
                // buffer dedicado de 2 bytes. Mismo patrón que CallStr:
                // guardar n en un registro escondido de la escritura del
                // puntero de destino (que también pasa por A) antes de
                // tocar Y.
                let buf = *buf as u16;

                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL (n)

                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8); // LDI A,#hi
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0x1A); // STA YL

                self.emit_byte(0x24); // LDA UL (n)
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x1E); // STA (Y) -- NUL final

                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::CallLen(max_len) => {
                // LEN(s): pop puntero (Y), contar bytes hasta el primer
                // NUL o hasta max_len (lo que llegue antes) — sin
                // necesitar memoria scratch (Y=puntero, UL=presupuesto).
                let max_len_u8 = (*max_len).min(255) as u8;

                self.emit_pop_a();
                self.emit_byte(0x1A); // YL
                self.emit_pop_a();
                self.emit_byte(0x18); // YH

                self.emit_byte(0xB5); self.emit_byte(max_len_u8); // LDI A,#max_len
                self.emit_byte(0x2A); // STA UL

                let loop_label = self.new_local_label("LEN_LOOP");
                let done_label = self.new_local_label("LEN_DONE");
                self.define_label(loop_label.clone());

                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UL!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si UL==0: presupuesto agotado)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x15); // LDA (Y)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si byte!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si byte==0: NUL encontrado)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x54); // Y++
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x2A); // STA UL
                self.emit_byte(0xBA); // JMP loop
                self.add_label_ref(loop_label, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(done_label);
                // longitud = max_len - UL restante
                self.emit_byte(0xB5); self.emit_byte(max_len_u8); // LDI A,#max_len
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                self.emit_push_a();
            }

            StackInstruction::CallLeft(max_len, buf) => {
                // LEFT$(s, n): copia los primeros min(n, max_len)
                // caracteres de s al buffer de resultado dedicado.
                let max_len_u8 = (*max_len).min(255) as u8;
                let buf = *buf as u16;
                let no_clamp = self.new_local_label("LEFT_NO_CLAMP");

                // Pop n -> UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL

                // Clamp: si n >= max_len, UL = max_len.
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0xB1); self.emit_byte(max_len_u8); // SBC A,#max_len
                self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3 (si n>=max_len/carry=1, seguir con el clamp)
                self.emit_byte(0xBA); // JMP no_clamp (si n<max_len)
                self.add_label_ref(no_clamp.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.emit_byte(0xB5); self.emit_byte(max_len_u8); // LDI A,#max_len
                self.emit_byte(0x2A); // STA UL
                self.define_label(no_clamp);

                // Pop puntero origen -> X
                self.emit_pop_a();
                self.emit_byte(0x0A); // XL
                self.emit_pop_a();
                self.emit_byte(0x08); // XH

                // Y = buf (destino)
                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8); // LDI A,#hi
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0x1A); // STA YL

                self.emit_copy_string_x_to_y_terminated();

                // Push puntero al resultado (high, luego low).
                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::CallRight(max_len, buf) => {
                // RIGHT$(s, n): busca el final de s (NUL o max_len),
                // retrocede hasta n caracteres sin pasar del principio,
                // y copia desde ahí hasta el final al buffer de resultado.
                let max_len_u8 = (*max_len).min(255) as u8;
                let buf = *buf as u16;

                // Pop n -> UL (se mantiene intacto para el copiado final)
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL

                // Pop puntero origen -> X (extremo izquierdo, referencia fija)
                self.emit_pop_a();
                self.emit_byte(0x0A); // XL
                self.emit_pop_a();
                self.emit_byte(0x08); // XH

                // Y = X (copia para recorrer buscando el final de la cadena)
                self.emit_byte(0xFD); self.emit_byte(0x5A); // Y = X

                // UH = presupuesto restante para el escaneo (max_len)
                self.emit_byte(0xB5); self.emit_byte(max_len_u8); // LDI A,#max_len
                self.emit_byte(0x28); // STA UH

                let scan_loop = self.new_local_label("RIGHT_SCAN_LOOP");
                let scan_done = self.new_local_label("RIGHT_SCAN_DONE");
                self.define_label(scan_loop.clone());

                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UH!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP scan_done (si UH==0: presupuesto agotado)
                self.add_label_ref(scan_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x15); // LDA (Y)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si char!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP scan_done (si char==0: NUL encontrado)
                self.add_label_ref(scan_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x54); // Y++
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x28); // STA UH
                self.emit_byte(0xBA); // JMP scan_loop
                self.add_label_ref(scan_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(scan_done);
                // Y apunta ahora al NUL (o a X+max_len si no había NUL cerca).
                // UH todavía tiene el presupuesto SOBRANTE del escaneo;
                // convertirlo en la longitud REAL encontrada (UH = max_len
                // - UH) — necesario para acotar tanto el retroceso como
                // la copia final: sin esto, pedir más caracteres de los
                // que hay (p.ej. RIGHT$ con n=100 sobre una cadena de 5)
                // retrocedía correctamente solo 5 posiciones pero LUEGO
                // copiaba 100 bytes igualmente (UL sin acotar), leyendo
                // basura más allá del buffer.
                self.emit_byte(0xB5); self.emit_byte(max_len_u8); // LDI A,#max_len
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0xA0); // SBC UH -> A = max_len - restante = longitud real
                self.emit_byte(0x28); // STA UH (longitud real)

                // Acotar UL (n) a la longitud real si la excede.
                let right_no_clamp = self.new_local_label("RIGHT_NO_CLAMP");
                self.emit_byte(0x24); // LDA UL (n)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0xA0); // SBC UH -> A = n - longitud real
                self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3 (si n>=longitud real/carry=1, seguir con el clamp)
                self.emit_byte(0xBA); // JMP right_no_clamp (si n<longitud real)
                self.add_label_ref(right_no_clamp.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.emit_byte(0xA4); // LDA UH (longitud real)
                self.emit_byte(0x2A); // STA UL (clamp: n = longitud real)
                self.define_label(right_no_clamp);

                // Retroceder Y hasta n (ya acotado) pasos, sin pasar de
                // X. UH = copia de n (UL sigue intacto para el copiado
                // final).
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0x28); // STA UH

                let back_loop = self.new_local_label("RIGHT_BACK_LOOP");
                let back_done = self.new_local_label("RIGHT_BACK_DONE");
                let back_continue = self.new_local_label("RIGHT_BACK_CONTINUE");
                self.define_label(back_loop.clone());

                // Si UH==0 (ya retrocedimos n veces), terminar.
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UH!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP back_done
                self.add_label_ref(back_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Si Y == X (llegamos al principio), terminar.
                self.emit_byte(0x14); // LDA YL
                self.emit_byte(0x06); // CPA XL
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si YL==XL/Z=1, saltar el JMP)
                self.emit_byte(0xBA); // JMP back_continue (si YL!=XL/Z=0: distintos)
                self.add_label_ref(back_continue.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x94); // LDA YH
                self.emit_byte(0x86); // CPA XH
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si YH==XH/Z=1, saltar el JMP)
                self.emit_byte(0xBA); // JMP back_continue (si YH!=XH/Z=0: distintos)
                self.add_label_ref(back_continue.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // YL==XL y YH==XH: Y==X, terminar el retroceso.
                self.emit_byte(0xBA); // JMP back_done
                self.add_label_ref(back_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(back_continue);
                self.emit_byte(0x56); // Y--
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x28); // STA UH
                self.emit_byte(0xBA); // JMP back_loop
                self.add_label_ref(back_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(back_done);
                // Y apunta al inicio de los últimos n caracteres (o a X
                // si se agotó antes). Preparar la copia: X=Y (origen),
                // Y=buf (destino), UL sigue siendo n.
                self.emit_byte(0xFD); self.emit_byte(0x18); // X = Y

                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8); // LDI A,#hi
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0x1A); // STA YL

                self.emit_copy_string_x_to_y_terminated();

                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::CallMid(max_len, buf) => {
                // MID$(s, start, length): avanza X (puntero a s) start-1
                // posiciones (1-indexado, se asume start>=1) y copia
                // hasta `length` caracteres desde ahí al buffer de
                // resultado dedicado.
                let buf = *buf as u16;
                let _ = max_len; // el límite real de recorrido lo da el NUL del origen o `length`

                // Pop length -> UL (se mantiene intacto para el copiado final)
                self.emit_pop_a();
                self.emit_byte(0x2A); // STA UL

                // Pop start -> UH (contador para avanzar X)
                self.emit_pop_a();
                self.emit_byte(0x28); // STA UH
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xDF); // DEC A (start - 1: nº de avances)
                self.emit_byte(0x28); // STA UH

                // Pop puntero origen -> X
                self.emit_pop_a();
                self.emit_byte(0x0A); // XL
                self.emit_pop_a();
                self.emit_byte(0x08); // XH

                let adv_loop = self.new_local_label("MID_ADV_LOOP");
                let adv_done = self.new_local_label("MID_ADV_DONE");
                self.define_label(adv_loop.clone());

                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UH!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP adv_done (si UH==0: ya avanzamos start-1)
                self.add_label_ref(adv_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x44); // X++
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x28); // STA UH
                self.emit_byte(0xBA); // JMP adv_loop
                self.add_label_ref(adv_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(adv_done);
                // X apunta ahora a la posición inicial. Copiar `length`
                // (UL) caracteres al buffer de resultado.
                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8); // LDI A,#hi
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0x1A); // STA YL

                self.emit_copy_string_x_to_y_terminated();

                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::CallStr(buf) => {
                // STR$(n): convierte un entero de 8 bits (0-255) a su
                // representación ASCII decimal (sin ceros a la
                // izquierda), en el buffer de resultado dedicado.
                let buf = *buf as u16;

                self.emit_pop_a();
                self.emit_extract_hundreds_tens_units(); // UH=centenas, UL=decenas, XL=unidades

                // Y = buf (destino, se va escribiendo secuencialmente)
                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8); // LDI A,#hi
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8); // LDI A,#lo
                self.emit_byte(0x1A); // STA YL

                let case_hundreds = self.new_local_label("STR_CASE_HUNDREDS");
                let case_tens = self.new_local_label("STR_CASE_TENS");
                let finish = self.new_local_label("STR_FINISH");

                // Si centenas != 0 -> case_hundreds (escribe 3 dígitos).
                self.emit_byte(0xA4); // LDA UH (centenas)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si centenas==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_hundreds (si centenas!=0)
                self.add_label_ref(case_hundreds.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Si decenas != 0 -> case_tens (escribe 2 dígitos).
                self.emit_byte(0x24); // LDA UL (decenas)
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si decenas==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_tens (si decenas!=0)
                self.add_label_ref(case_tens.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Caso "solo unidades" (ni centenas ni decenas): 1 dígito
                // (incluye el caso 0).
                self.emit_byte(0x04); // LDA XL (unidades)
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#0x30 ('0')
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0xBA); // JMP finish
                self.add_label_ref(finish.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // 3 dígitos: centenas, decenas, unidades.
                self.define_label(case_hundreds);
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#0x30
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#0x30
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#0x30
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0xBA); // JMP finish
                self.add_label_ref(finish.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // 2 dígitos: decenas, unidades.
                self.define_label(case_tens);
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#0x30
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xB3); self.emit_byte(0x30); // ADC A,#0x30
                self.emit_byte(0x1E); // STA (Y)
                self.emit_byte(0x54); // Y++
                // (cae en finish)

                self.define_label(finish);
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x1E); // STA (Y) -- NUL final

                self.emit_byte(0xB5); self.emit_byte((buf >> 8) as u8);
                self.emit_push_a();
                self.emit_byte(0xB5); self.emit_byte((buf & 0xFF) as u8);
                self.emit_push_a();
            }

            StackInstruction::CallVal(max_len, scratch) => {
                let max_len_u8 = (*max_len).min(255) as u8;
                let scratch = *scratch as u16;
                let result_addr = scratch;
                let temp1_addr = scratch + 1;
                let temp2_addr = scratch + 2;
                let digit_addr = scratch + 3;

                // Pop puntero -> Y
                self.emit_pop_a();
                self.emit_byte(0x1A); // YL
                self.emit_pop_a();
                self.emit_byte(0x18); // YH

                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0xAE); self.emit_word(result_addr); // STA result (= 0)

                self.emit_byte(0xB5); self.emit_byte(max_len_u8); // LDI A,#max_len
                self.emit_byte(0x2A); // STA UL (presupuesto)

                let loop_label = self.new_local_label("VAL_LOOP");
                let done_label = self.new_local_label("VAL_DONE");
                self.define_label(loop_label.clone());

                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si UL!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si UL==0: presupuesto agotado)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0x15); // LDA (Y)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0xB1); self.emit_byte(0x30); // SBC A,#0x30 -> A = char - '0'
                self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3 (si no hubo borrow/carry=1, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si hubo borrow: char < '0', no es dígito)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.emit_byte(0xB7); self.emit_byte(0x0A); // CPI A,#10
                self.emit_byte(0x81); self.emit_byte(0x03); // BCR +3 (si A<10, saltar el JMP)
                self.emit_byte(0xBA); // JMP done (si A>=10: char > '9', no es dígito)
                self.add_label_ref(done_label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // A = dígito válido (0-9).
                self.emit_byte(0xAE); self.emit_word(digit_addr); // STA digit

                self.emit_byte(0xA5); self.emit_word(result_addr); // LDA result (valor previo)
                self.emit_a_times10_plus_mem(temp1_addr, temp2_addr, digit_addr);
                self.emit_byte(0xAE); self.emit_word(result_addr); // STA result (nuevo valor)

                self.emit_byte(0x54); // Y++
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xDF); // DEC A
                self.emit_byte(0x2A); // STA UL
                self.emit_byte(0xBA); // JMP loop
                self.add_label_ref(loop_label, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(done_label);
                self.emit_byte(0xA5); self.emit_word(result_addr); // LDA result
                self.emit_push_a();
            }

            StackInstruction::CallRnd(seed_addr) => {
                // RND(n): sustituto deliberadamente NO auténtico — $F5EB
                // ("RAND_GEN") resultó NO ser el punto de entrada general
                // de la ROM para n>0 (ver historial: llamarlo directo con
                // ARX poblado, mismo patrón que ADDIT/MULTIPLY, produjo
                // escrituras a memoria no mapeada; el camino real con
                // escalado pasa por varias subrutinas sin documentar
                // — $F707, $F715, $F6B4, $F661, $F88F — pendientes de
                // investigar). En su lugar: un LFSR de Galois de 8 bits
                // autocontenido (polinomio x^8+x^6+x^5+x^4+1, máscara
                // 0xB8, longitud máxima 255 antes de repetirse),
                // documentado explícitamente como no auténtico.
                //
                // `seed_addr` guarda el estado entre llamadas (asignado
                // una vez por StackCodeGenerator, igual que el scratch de
                // AND/OR). Cada llamada: (1) si la semilla es 0
                // (arranque en frío), la fija a 1 — un LFSR con semilla 0
                // se queda atascado en 0 para siempre; (2) avanza el LFSR
                // un paso; (3) reduce el resultado módulo n (resta
                // repetida, mismo patrón que DivInt) para devolver un
                // valor en [0, n); n=0 empuja 0 directamente (caso sin
                // rango, evita un bucle infinito restando 0).
                let seed_addr = *seed_addr as u16;

                let seed_nonzero = self.new_local_label("RND_SEED_NONZERO");
                self.emit_byte(0xA5); // LDA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si semilla==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP seed_nonzero (si semilla!=0: ya vale, saltar la inicialización)
                self.add_label_ref(seed_nonzero.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.emit_byte(0xB5); self.emit_byte(0x01); // LDI A,#1 (semilla==0: fijar a 1)
                self.emit_byte(0xAE); // STA addr (semilla)
                self.emit_word(seed_addr);
                self.define_label(seed_nonzero);

                // Avanzar LFSR: UL = valor original (para el shift),
                // A = bit0 de una copia (test sin destruir UL).
                self.emit_byte(0xA5); // LDA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0x2A); // STA UL (valor original)
                self.emit_byte(0xB9); self.emit_byte(0x01); // ANI A,#1 (bit0)

                let lfsr_odd = self.new_local_label("RND_LFSR_ODD");
                let lfsr_done = self.new_local_label("RND_LFSR_DONE");
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si bit0==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP lfsr_odd (si bit0==1)
                self.add_label_ref(lfsr_odd.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // bit0==0: semilla >>= 1, sin XOR.
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xD5); // SHR
                self.emit_byte(0xAE); // STA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0xBA); // JMP lfsr_done
                self.add_label_ref(lfsr_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // bit0==1: semilla = (semilla >> 1) XOR 0xB8.
                self.define_label(lfsr_odd);
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xD5); // SHR
                self.emit_byte(0xBD); self.emit_byte(0xB8); // EOR A,#0xB8
                self.emit_byte(0xAE); // STA addr (semilla)
                self.emit_word(seed_addr);

                self.define_label(lfsr_done);

                // Pop n (16 bits: alto primero al desapilar, mismo
                // convenio que ApilaIntWord/DesapilaIndWord — ver el
                // ajuste en FunctionInner::Rnd, mod.rs). SIEMPRE 16 bits
                // ahora, nunca 8: bug real encontrado jugando
                // bathyscaph.bas de verdad (no en ningún test aislado) —
                // `RND 256-1` pasa n=256 (no cabe en 1 byte, así que se
                // apilaba como 16 bits), pero aquí solo se hacía pop de 1
                // byte; cada llamada dejaba 1 byte suelto en la pila, y
                // tras las 31 vueltas del bucle de la subrutina "CRASH"
                // eso desincronizaba lo bastante como para que el
                // siguiente POKE# leyera basura como dirección.
                self.emit_pop_a();
                self.emit_byte(0x0A); // STA XL (n, byte bajo)
                self.emit_pop_a();
                self.emit_byte(0x08); // STA XH (n, byte alto)

                let mod_zero = self.new_local_label("RND_MOD_ZERO");
                let mod_loop = self.new_local_label("RND_MOD_LOOP");
                let mod_done = self.new_local_label("RND_MOD_DONE");
                let no_reduction = self.new_local_label("RND_NO_REDUCTION");

                // n>=256 (byte alto != 0): "módulo n" sobre una semilla
                // que ya es de 8 bits (0-255) es un no-op (0-255 < 256
                // siempre) — empujar la semilla ya avanzada tal cual, sin
                // pasar por la reducción de abajo (que asume n de 8 bits
                // en UL). Cubre exactamente `RND 256`.
                self.emit_byte(0x84); // LDA XH
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si XH==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP no_reduction (si XH!=0, n>=256)
                self.add_label_ref(no_reduction.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // XH==0: n cabe en 8 bits, copiar a UL y seguir con la
                // lógica de reducción existente sin cambios.
                self.emit_byte(0x04); // LDA XL (n, byte bajo)
                self.emit_byte(0x2A); // STA UL (n)

                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0 (A todavía es n, recién copiado)
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si n!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP mod_zero (si n==0)
                self.add_label_ref(mod_zero.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // n != 0: reducir la semilla (ya avanzada) módulo n.
                self.emit_byte(0xA5); // LDA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_byte(0x0A); // STA XL (valor a reducir)

                self.define_label(mod_loop.clone());
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL (A = XL - n)
                self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3 (si XL>=n, seguir)
                self.emit_byte(0xBA); // JMP mod_done (si XL<n, underflow: XL ya es el resultado)
                self.add_label_ref(mod_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.emit_byte(0x0A); // STA XL (confirmar XL -= n)
                self.emit_byte(0xBA); // JMP mod_loop
                self.add_label_ref(mod_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(mod_done);
                self.emit_byte(0x04); // LDA XL
                self.emit_push_a();
                self.emit_byte(0xBA); // JMP fin (saltar el caso n==0)
                let rnd_end = self.new_local_label("RND_END");
                self.add_label_ref(rnd_end.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(mod_zero);
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_push_a();
                self.emit_byte(0xBA); // JMP rnd_end (saltar el caso n>=256)
                self.add_label_ref(rnd_end.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // n>=256: empujar la semilla (ya avanzada) sin reducir.
                self.define_label(no_reduction);
                self.emit_byte(0xA5); // LDA addr (semilla)
                self.emit_word(seed_addr);
                self.emit_push_a();

                self.define_label(rnd_end);
            }

            StackInstruction::CallSgn => {
                // SGN(x) sobre un real de 8 bytes: pop a ARX. En este
                // formato es trivial sin ninguna rutina ROM — el signo
                // vive en un byte dedicado (ARX+1) y el valor es 0 si y
                // solo si TODOS los bytes de mantisa (ARX+2..ARX+8) son
                // 0. Push 1 byte: 0xFF(-1)/0x00(0)/0x01(+1), mismo
                // convenio de complemento a 2 que el resto de la
                // aritmética entera de este backend (ver Negativo).
                self.emit_pop_8_to(system_memory::ARX);

                let case_zero = self.new_local_label("SGN_CASE_ZERO");
                let case_negative = self.new_local_label("SGN_CASE_NEGATIVE");
                let done = self.new_local_label("SGN_DONE");

                // A = OR de los 6 bytes de mantisa (0 <=> valor 0).
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                for offset in 2..8u16 {
                    self.emit_byte(0xAB); // OR addr
                    self.emit_word(system_memory::ARX + offset);
                }
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x89); self.emit_byte(0x03); // BZR +3 (si A!=0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_zero (si A==0)
                self.add_label_ref(case_zero.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Mantisa no vacía: mirar el byte de signo.
                self.emit_byte(0xA5); // LDA addr (ARX+1)
                self.emit_word(system_memory::ARX + 1);
                self.emit_byte(0xB7); self.emit_byte(0x00); // CPI A,#0
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si signo==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_negative (si signo!=0)
                self.add_label_ref(case_negative.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // Positivo: push 1.
                self.emit_byte(0xB5); self.emit_byte(0x01); // LDI A,#1
                self.emit_push_a();
                self.emit_byte(0xBA); // JMP done
                self.add_label_ref(done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(case_negative);
                self.emit_byte(0xB5); self.emit_byte(0xFF); // LDI A,#-1 (0xFF)
                self.emit_push_a();
                self.emit_byte(0xBA); // JMP done
                self.add_label_ref(done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(case_zero);
                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_push_a();

                self.define_label(done);
            }

            StackInstruction::CallInt => {
                // INT(x) sobre un real de 8 bytes: pop a ARX y trunca a
                // un entero de 8 bits (parte entera, hacia cero)
                // reconstruido dígito a dígito desde exponente+mantisa
                // (método de Horner, ver `emit_a_times10_plus_ul`). Solo
                // soporta magnitudes de hasta 3 dígitos (exponente
                // 0..=2, |valor|<1000) — el único rango que produce el
                // resto de este backend (enteros de 8 bits, 0-255,
                // promocionados vía Int2Real). Exponente negativo
                // (|valor|<1) trunca a 0; exponente>=3 no está soportado
                // (documentado, ningún programa objetivo lo necesita):
                // ambos casos caen en el mismo "push 0" por defecto.
                self.emit_pop_8_to(system_memory::ARX);
                self.emit_bcd_arx_to_int_a();
                self.emit_push_a();
            }

            StackInstruction::CallPoint => {
                // POINT(x): pop columna x (0-155) y lee el punto del
                // buffer de pantalla — reimplementado directamente contra
                // el formato de buffer verificado en
                // ceres-core::display.rs (`update_display_buffer`), en
                // vez de la rutina ROM (que necesitaría más aritmética de
                // páginas de memoria para poco beneficio). El buffer
                // empaqueta 2 columnas por cada par de bytes: sea
                // half=x/39 (0-3) y k=x%39 — dirección=base(half)+2k,
                // donde base es $7600 si half es par, $7700 si es impar,
                // y el nibble a usar es el BAJO si half<2, el ALTO si
                // half>=2. Resultado = nibble(byte0) | nibble(byte1)<<4.
                self.emit_pop_a();
                self.emit_byte(0x0A); // STA XL (x, valor restante de la división)

                self.emit_byte(0xB5); self.emit_byte(0x00); // LDI A,#0
                self.emit_byte(0x28); // STA UH (half = 0)

                let div_loop = self.new_local_label("POINT_DIV_LOOP");
                let div_done = self.new_local_label("POINT_DIV_DONE");
                self.define_label(div_loop.clone());
                self.emit_byte(0x04); // LDA XL
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0xB1); self.emit_byte(39); // SBC A,#39
                self.emit_byte(0x83); self.emit_byte(0x03); // BCS +3 (si x>=39, seguir)
                self.emit_byte(0xBA); // JMP div_done (si x<39, underflow: x ya es k)
                self.add_label_ref(div_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.emit_byte(0x0A); // STA XL (confirmar x -= 39)
                self.emit_byte(0xA4); // LDA UH
                self.emit_byte(0xDD); // INC A
                self.emit_byte(0x28); // STA UH (half++)
                self.emit_byte(0xBA); // JMP div_loop
                self.add_label_ref(div_loop, RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
                self.define_label(div_done);
                // XL = k, UH = half

                self.emit_byte(0x04); // LDA XL (k)
                self.emit_byte(0xD9); // SHL (2k)
                self.emit_byte(0x0A); // STA XL (2k)
                self.emit_byte(0x1A); // STA YL (2k, byte bajo de la dirección)

                let case_odd = self.new_local_label("POINT_BASE_ODD");
                let after_base = self.new_local_label("POINT_AFTER_BASE");
                self.emit_byte(0xA4); // LDA UH (half)
                self.emit_byte(0xB9); self.emit_byte(0x01); // ANI A,#1
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si half par/A==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_odd (si half impar)
                self.add_label_ref(case_odd.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // half par: base = 0x76
                self.emit_byte(0xB5); self.emit_byte(0x76); // LDI A,#0x76
                self.emit_byte(0x18); // STA YH
                self.emit_byte(0xBA); // JMP after_base
                self.add_label_ref(after_base.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                self.define_label(case_odd);
                self.emit_byte(0xB5); self.emit_byte(0x77); // LDI A,#0x77
                self.emit_byte(0x18); // STA YH

                self.define_label(after_base);
                // Y = dirección del primer byte del par.
                self.emit_byte(0x15); // LDA (Y) -> byte0
                self.emit_byte(0x2A); // STA UL (byte0)
                self.emit_byte(0x54); // Y++
                self.emit_byte(0x15); // LDA (Y) -> byte1
                self.emit_byte(0x0A); // STA XL (byte1)

                let case_high = self.new_local_label("POINT_NIBBLE_HIGH");
                let point_done = self.new_local_label("POINT_DONE");
                self.emit_byte(0xA4); // LDA UH (half)
                self.emit_byte(0xB9); self.emit_byte(0x02); // ANI A,#2
                self.emit_byte(0x8B); self.emit_byte(0x03); // BZS +3 (si half<2/A==0, saltar el JMP)
                self.emit_byte(0xBA); // JMP case_high (si half>=2/A!=0)
                self.add_label_ref(case_high.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // half<2: nibble bajo de cada byte.
                self.emit_byte(0x24); // LDA UL (byte0)
                self.emit_byte(0xB9); self.emit_byte(0x0F); // ANI A,#0x0F -> nib0
                self.emit_byte(0x28); // STA UH (nib0, half ya no hace falta)
                self.emit_byte(0x04); // LDA XL (byte1)
                self.emit_byte(0xB9); self.emit_byte(0x0F); // ANI A,#0x0F -> nib1 (bits bajos)
                self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); self.emit_byte(0xD9); // SHL x4 -> nib1<<4
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xA2); // ADC UH -> resultado = nib0 | (nib1<<4)
                self.emit_push_a();
                self.emit_byte(0xBA); // JMP point_done
                self.add_label_ref(point_done.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);

                // half>=2: nibble alto de cada byte.
                self.define_label(case_high);
                self.emit_byte(0x24); // LDA UL (byte0)
                self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); self.emit_byte(0xD5); // SHR x4 -> nib0
                self.emit_byte(0x28); // STA UH (nib0)
                self.emit_byte(0x04); // LDA XL (byte1)
                self.emit_byte(0xB9); self.emit_byte(0xF0); // ANI A,#0xF0 -> nib1 ya en la posición alta
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xA2); // ADC UH -> resultado = nib0 | (byte1&0xF0)
                self.emit_push_a();

                self.define_label(point_done);
            }

            StackInstruction::MayorIgualInt => {
                // Pop b, Pop a, Push (a >= b ? 1 : 0)
                // Implementación: a >= b ⟺ a - b >= 0

                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)

                // 2. Pop a
                self.emit_pop_a();

                // 3. Comparar: A - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si carry set (sin borrow), a >= b
                self.emit_byte(0x83); // BCS +4
                self.emit_byte(0x04);
                
                // Sin carry (a < b), cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                // Saltar 2 bytes
                self.emit_byte(0x8E); // BCH +2
                self.emit_byte(0x02);
                
                // Con carry (a >= b), cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::MenorIgualInt => {
                // Pop b, Pop a, Push (a <= b ? 1 : 0)
                // Implementación: a <= b ⟺ !(a > b)

                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)

                // 2. Pop a
                self.emit_pop_a();

                // 3. Comparar: A - UL (SEC, no REC: ver nota en RestaInt)
                self.emit_byte(0xFB); // SEC
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si A <= 0 (Z=1 o negativo), push 1
                // Primero chequear a < b (carry reset tras SBC)
                self.emit_byte(0x81); // BCR +8 -> cargar 1
                self.emit_byte(0x08);

                // Si no, chequear igualdad
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                self.emit_byte(0x8B); // BZS +4 -> cargar 1
                self.emit_byte(0x04);
                
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);

                // Saltar sobre el caso verdadero
                self.emit_byte(0x8E); // BCH +2
                self.emit_byte(0x02);

                // Caso verdadero (a < b o a == b)
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                
                // 5. Push resultado
                self.emit_push_a();
            }

            // ===== OPERACIONES LÓGICAS (AND / OR bit a bit) =====
            //
            // El LH5801 no tiene AND/OR registro-a-registro con un operando
            // en UL/UH (a diferencia de ADC/SBC, que sí); solo `AND addr` /
            // `OR addr` con una dirección absoluta de 16 bits (ver
            // AndInt/OrInt en stack_instruction.rs). Se usa `scratch` como
            // sitio temporal en memoria para `b` mientras `a` se mantiene
            // en A.

            StackInstruction::AndInt(scratch) => {
                let scratch = *scratch as u16;

                // 1. Pop b, guardar en scratch
                self.emit_pop_a();
                self.emit_byte(0xAE); // STA addr
                self.emit_word(scratch);

                // 2. Pop a (queda en A)
                self.emit_pop_a();

                // 3. A = A AND [scratch]
                self.emit_byte(0xA9); // AND addr
                self.emit_word(scratch);

                // 4. Push resultado
                self.emit_push_a();
            }

            StackInstruction::OrInt(scratch) => {
                let scratch = *scratch as u16;

                // 1. Pop b, guardar en scratch
                self.emit_pop_a();
                self.emit_byte(0xAE); // STA addr
                self.emit_word(scratch);

                // 2. Pop a (queda en A)
                self.emit_pop_a();

                // 3. A = A OR [scratch]
                self.emit_byte(0xAB); // OR addr
                self.emit_word(scratch);

                // 4. Push resultado
                self.emit_push_a();
            }

            // ===== DATA / READ / RESTORE =====
            //
            // Ambas se implementan como una búsqueda lineal generada en
            // tiempo de compilación (una comparación + salto por cada
            // valor de DATA / línea con DATA del programa), reutilizando
            // el mismo patrón de trampolín (branch corto invertido + JMP
            // absoluto) que IrF/IrV — evita tener que hacer aritmética de
            // punteros de 16 bits (dirección_tabla + 2*índice) en tiempo
            // de ejecución. Es lineal en el número de DATA del programa,
            // no en el tamaño del código, así que es aceptable para
            // programas reales (p.ej. bathyscaph.bas tiene 16 DATA).

            StackInstruction::ReadData(addr) => {
                let done_label = self.new_local_label("READ_DONE");
                let addr = *addr as u16;

                self.emit_byte(0xA5); // LDA addr (índice actual)
                self.emit_word(addr);

                for (i, value) in self.data_pool.clone().iter().enumerate() {
                    self.emit_byte(0xB7); // CPI A,#i
                    self.emit_byte(i as u8);
                    // Bloque a saltar si A != i: emit_push_string_literal
                    // (8 bytes: 2x (LDI A,#imm[2] + PSH A[2])) + JMP (3 bytes).
                    self.emit_byte(0x89); // BZR +11
                    self.emit_byte(11);
                    self.emit_push_string_literal(value);
                    self.emit_byte(0xBA); // JMP done (solo si A == i)
                    self.add_label_ref(done_label.clone(), RefType::Absolute16);
                    self.emit_label_placeholder(RefType::Absolute16);
                }

                // No debería alcanzarse en un programa correcto (READ sin
                // más DATA disponible) — apilar un puntero nulo para no
                // desequilibrar la pila.
                self.emit_byte(0xB5);
                self.emit_byte(0x00);
                self.emit_push_a();
                self.emit_byte(0xB5);
                self.emit_byte(0x00);
                self.emit_push_a();

                self.define_label(done_label);

                // Avanzar el índice para la siguiente lectura.
                self.emit_byte(0xA5); // LDA addr
                self.emit_word(addr);
                self.emit_byte(0xDD); // INC A
                self.emit_byte(0xAE); // STA addr
                self.emit_word(addr);
            }

            StackInstruction::RestoreData(addr) => {
                let done_label = self.new_local_label("RESTORE_DONE");
                let addr = *addr as u16;

                // Pop número de línea (16 bits: bajo primero, luego alto —
                // ApilaInt empuja alto y luego bajo, así que el bajo queda
                // encima). 0 = sentinela de "sin argumento" (RESTORE sin
                // línea), ningún programa BASIC real usa la línea 0.
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (byte bajo)
                self.emit_pop_a();
                self.emit_byte(0x28); // UH = A (byte alto)

                // (0, 0) -> índice 0 antepuesto para tratar el sentinela
                // con el mismo mecanismo que una línea real de la tabla.
                let mut entries: Vec<(u16, usize)> = vec![(0, 0)];
                entries.extend(self.data_line_table.clone());

                for (line, index) in entries {
                    let hi = (line >> 8) as u8;
                    let lo = (line & 0xFF) as u8;

                    self.emit_byte(0xA4); // LDA UH
                    self.emit_byte(0xB7); // CPI A,#hi
                    self.emit_byte(hi);
                    // Si el byte alto no coincide, saltar el resto de esta
                    // entrada entera: bloque de byte bajo (5) + bloque de
                    // coincidencia (8) = 13 bytes.
                    self.emit_byte(0x89); // BZR +13
                    self.emit_byte(13);

                    self.emit_byte(0x24); // LDA UL
                    self.emit_byte(0xB7); // CPI A,#lo
                    self.emit_byte(lo);
                    // Si el byte bajo no coincide (con el alto ya
                    // coincidiendo), saltar solo el bloque de coincidencia
                    // (8 bytes: LDI[2]+STA addr[3]+JMP[3]).
                    self.emit_byte(0x89); // BZR +8
                    self.emit_byte(8);

                    self.emit_byte(0xB5); // LDI A,#index
                    self.emit_byte(index as u8);
                    self.emit_byte(0xAE); // STA addr
                    self.emit_word(addr);
                    self.emit_byte(0xBA); // JMP done
                    self.add_label_ref(done_label.clone(), RefType::Absolute16);
                    self.emit_label_placeholder(RefType::Absolute16);
                }

                // Línea no encontrada en la tabla: quedarse al principio
                // en vez de dejar el índice sin definir.
                self.emit_byte(0xB5); // LDI A,#0
                self.emit_byte(0x00);
                self.emit_byte(0xAE); // STA addr
                self.emit_word(addr);

                self.define_label(done_label);
            }

            // ===== CONTROL =====

            StackInstruction::Stop => {
                // Terminar programa - RTN devuelve control a BASIC
                self.emit_halt();
            }
            
            // ===== NO OPERACIÓN =====
            
            StackInstruction::Nop => {
                // NOP - no hay instrucción NOP en LH5801
                // Usar instrucción inocua como AND A, #0xFF
                self.emit_byte(0xB9); // AND #imm
                self.emit_byte(0xFF);
            }
            
            // ===== INSTRUCCIONES NO IMPLEMENTADAS =====
            
            _ => {
                // Por ahora, emitir comentario de depuración
                eprintln!("WARNING: Instrucción no implementada en backend LH5801: {:?}", instr);
                self.emit_byte(0xB9); // AND #0xFF (NOP)
                self.emit_byte(0xFF);
            }
        }
    }
    
    /// Obtener código generado como bytes
    pub fn get_code(&self) -> &[u8] {
        &self.code
    }
    
    /// Obtener dirección de inicio
    pub fn get_start_address(&self) -> u16 {
        self.start_address
    }
}

impl Default for Lh5801Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        if needle.is_empty() {
            return true;
        }
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn generated_code_for(instr: StackInstruction) -> Vec<u8> {
        let mut backend = Lh5801Backend::new();
        backend.generate(&[instr])
    }

    /// Test de humo del oráculo (Fase 0): ejecuta el prólogo de
    /// inicialización real generado por el backend contra el emulador
    /// (`ceres-core`, ROM real) y comprueba que `LDI S,#imm`/`LDI A,#imm`/
    /// `STA addr` (0xAA/0xB5/0xAE) se comportan como asume el resto del
    /// backend, en vez de solo confiar en la codificación de bytes esperada.
    #[test]
    fn test_oracle_initialization_prologue_runs_on_real_rom() {
        use crate::codegen::test_oracle::{run_lh5, ORACLE_LOAD_ADDR};

        let mut backend = Lh5801Backend::with_config(ORACLE_LOAD_ADDR, 0x57FF);
        let code = backend.generate(&[]);

        // El prólogo de inicialización son exactamente 12 instrucciones
        // (LDI S,#imm; DON [activar pantalla]; LDI A,#0; 7x STA addr;
        // LDI A,#0x60; STA addr) antes de tocar nada más (incluida la RTN
        // final, cuyo destino no está garantizado aquí).
        let pc1500 = run_lh5(ORACLE_LOAD_ADDR, &code, 12);

        assert_eq!(pc1500.cpu().s(), 0x57FF, "S debe inicializarse a stack_top");
        assert!(pc1500.cpu().display_enabled(), "la pantalla debe quedar activada (DON) — si no, update_display_buffer() nunca pinta nada");
        assert_eq!(pc1500.read_byte(0x7874), 0x00, "CURSOR_ENA debe quedar a 0");
        assert_eq!(pc1500.read_byte(0x7875), 0x00, "CURSOR_PTR debe quedar a 0");
        assert_eq!(pc1500.read_byte(0x785D), 0x00, "KATAFLAGS debe quedar a 0");
        assert_eq!(pc1500.read_byte(0x7895), 0x00, "bloque USING debe quedar a 0");
        assert_eq!(pc1500.read_byte(0x7898), 0x00, "bloque USING debe quedar a 0");
        assert_eq!(pc1500.read_byte(0x788F), 0x60, "puntero de OUT_BUF debe quedar en 0x60");
    }

    /// Fase 1: verifica contra la ROM real que GOSUB/RETURN (`Call`/`IrInd`)
    /// realmente vuelve al punto de llamada tras el arreglo — antes `IrInd`
    /// caía en el catch-all (NOP) y esto nunca se ejecutaba.
    ///
    /// Programa: salta a MAIN, que llama a SUB (GOSUB); SUB escribe 0xAA en
    /// 0x5000 y hace RETURN; si el RETURN funciona, la ejecución continúa
    /// justo después del Call y escribe 0xBB en 0x5001. Si RETURN fuera un
    /// NOP, la ejecución seguiría dentro de SUB (que no tiene más código
    /// que consumir aparte del propio NOP) y 0x5001 nunca se escribiría.
    #[test]
    fn test_oracle_gosub_return_actually_returns_on_real_rom() {
        use crate::codegen::test_oracle::{run_lh5, ORACLE_LOAD_ADDR};

        let mut backend = Lh5801Backend::with_config(ORACLE_LOAD_ADDR, 0x57FF);
        let instructions = vec![
            StackInstruction::IrA("MAIN".to_string()),
            StackInstruction::Label("SUB".to_string()),
            StackInstruction::ApilaInt(0x5000),
            StackInstruction::ApilaInt(0xAA),
            StackInstruction::DesapilaInd,
            StackInstruction::IrInd, // RETURN
            StackInstruction::Label("MAIN".to_string()),
            StackInstruction::Call("SUB".to_string()), // GOSUB
            StackInstruction::ApilaInt(0x5001),
            StackInstruction::ApilaInt(0xBB),
            StackInstruction::DesapilaInd,
        ];
        let code = backend.generate(&instructions);

        // Prólogo (12) + IrA (1) + ApilaInt(0x5000) (4, >255) +
        // ApilaInt(0xAA) (2) + DesapilaInd (8) + IrInd/RTN (1) + Call/SJP
        // (1) + ApilaInt(0x5001) (4) + ApilaInt(0xBB) (2) + DesapilaInd (8)
        // = 43 instrucciones para completar ambas escrituras, justo antes
        // de la RTN del epílogo (cuyo destino no está garantizado aquí).
        let pc1500 = run_lh5(ORACLE_LOAD_ADDR, &code, 43);

        assert_eq!(pc1500.read_byte(0x5000), 0xAA, "SUB debe ejecutarse");
        assert_eq!(
            pc1500.read_byte(0x5001), 0xBB,
            "tras el RETURN la ejecución debe continuar después del Call, no quedarse en SUB"
        );
    }

    /// Fase 1: verifica contra la ROM real un `FOR...STEP` descendente
    /// (`STEP -1`) a través del pipeline completo (fuente BASIC → lexer →
    /// parser → IR de pila → backend). Antes el backend ignoraba el signo
    /// del STEP y siempre comparaba con `MayorInt` (ascendente), lo que
    /// habría hecho que este bucle nunca terminara (la condición de salida
    /// "variable > límite" nunca se cumple bajando de 3 a 1).
    #[test]
    fn test_oracle_for_next_descending_step_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        // @(21504)=0x5400 cuenta cuántas veces se ejecuta el cuerpo del
        // bucle; @(21505)=0x5401 guarda el valor final de J. Se usan
        // direcciones fijas (@) en zona libre (0x5400+) en vez de
        // variables normales, para no depender de qué dirección les
        // asigne internamente el compilador — pero deben quedar fuera del
        // área de variables auto-asignadas (DATA_BASE=0x5000+), o
        // colisionan con el scratch del STEP/J y se corrompen entre sí.
        let source = "10 @(21504)=0\n20 FOR J=3 TO 1 STEP -1\n30 @(21504)=@(21504)+1\n40 NEXT J\n50 @(21505)=J\n60 END\n";
        let code = compile_native(source);

        // El número exacto de instrucciones reales de un programa con
        // bucle es difícil de predecir a mano; se ejecuta hasta que el PC
        // sale de la región de código (justo tras la RTN final).
        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        assert_eq!(
            pc1500.read_byte(0x5400), 3,
            "el cuerpo debe ejecutarse 3 veces (J=3,2,1) con STEP -1"
        );
        assert_eq!(pc1500.read_byte(0x5401), 0, "J debe terminar en 0 tras salir del bucle descendente");
    }

    /// Complementa el test descendente: comprueba que el caso ascendente
    /// (sin cláusula STEP, por defecto 1) sigue funcionando tras el
    /// arreglo del signo del STEP y el bug de SEC/REC.
    #[test]
    fn test_oracle_for_next_ascending_default_step_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 @(21504)=0\n20 FOR J=1 TO 3\n30 @(21504)=@(21504)+1\n40 NEXT J\n50 @(21505)=J\n60 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        assert_eq!(
            pc1500.read_byte(0x5400), 3,
            "el cuerpo debe ejecutarse 3 veces (J=1,2,3) con STEP por defecto"
        );
        assert_eq!(pc1500.read_byte(0x5401), 4, "J debe terminar en 4 tras salir del bucle ascendente");
    }

    /// Test end-to-end del "núcleo reducido": un programa que combina en
    /// un solo pipeline real (fuente → lexer → parser → IR → backend →
    /// ejecución en la ROM real) las construcciones básicas ya
    /// implementadas y verificadas por separado — asignación, aritmética,
    /// comparación justo en el límite de igualdad (el caso que rompía el
    /// bug de SEC/REC), IF-THEN, FOR/NEXT, GOSUB/RETURN y PRINT de un
    /// código de carácter vía la rutina real CHAR_OUT — para comprobar que
    /// el núcleo funciona como conjunto, no solo característica a
    /// característica de forma aislada.
    ///
    /// Nota sobre el alcance real de PRINT: `PRINT expr` imprime la
    /// representación decimal en texto de `expr` (vía `SystemOutInt` +
    /// `CHAR_OUT`, con dígitos reales) — antes de que existiera ese
    /// formateo, este mismo test imprimía el BYTE de `expr` directamente
    /// como código de carácter (67='C', 88='X'); ahora imprime los
    /// dígitos "67"/"88" como texto, 2 caracteres cada uno.
    #[test]
    fn test_oracle_core_subset_end_to_end_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 A=65\n20 B=2\n30 C=A+B\n40 IF C=67 THEN PRINT C;\n50 FOR I=1 TO 2\n60 GOSUB 100\n70 NEXT I\n80 @(21600)=99\n90 END\n100 PRINT 88;\n110 RETURN\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // @(21600) solo se escribe tras salir del FOR y volver de las dos
        // GOSUB — confirma que todo el programa se ejecutó hasta el final,
        // no que se colgó o saltó a mitad.
        assert_eq!(pc1500.read_byte(0x5460), 99, "el programa debe llegar hasta el final (línea 80)");

        // CHAR_OUT avanza CURSOR_PTR ($7875) en 6 columnas por carácter
        // impreso (ancho de glifo). Se imprimen 6 caracteres en total:
        // "67" (línea 40, A+B=67=igualdad exacta) y "88" dos veces (una
        // por iteración GOSUB) = 2+2+2 dígitos.
        assert_eq!(
            pc1500.read_byte(0x7875), 36,
            "CURSOR_PTR debe avanzar 6*6=36 tras imprimir \"67\"+\"88\"+\"88\" (6 dígitos) vía CHAR_OUT"
        );

        // Comprueba que CHAR_OUT realmente dibujó píxeles (no solo movió
        // el cursor): al menos un byte del buffer de pantalla debe haber
        // cambiado de su valor inicial (0x00).
        let display_touched = (0x7600..0x7600 + 36).any(|addr| pc1500.read_byte(addr) != 0x00);
        assert!(display_touched, "CHAR_OUT debe haber escrito píxeles reales en el buffer de pantalla");
    }

    /// Fase 2 (núcleo reducido de arrays): `DIM A(5)` con tamaño constante
    /// debe reservar 6 elementos reales (índices 0..=5) de 1 byte cada uno
    /// y `A(i)` debe direccionar exactamente `base+i`, no el tamaño de
    /// elemento hardcodeado (5 bytes) que usaba el código antes de esta
    /// fase. Verificado contra la ROM real: escribe en tres índices
    /// distintos (incluido el último, `A(5)`, el caso límite) y los relee
    /// por direcciones fijas (`@`) independientes de la asignación interna
    /// de direcciones.
    #[test]
    fn test_oracle_array_1d_constant_size_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 DIM A(5)\n20 A(0)=11\n30 A(1)=22\n40 A(5)=99\n50 @(21600)=A(0)\n60 @(21601)=A(1)\n70 @(21602)=A(5)\n80 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        assert_eq!(pc1500.read_byte(0x5460), 11, "A(0)");
        assert_eq!(pc1500.read_byte(0x5461), 22, "A(1)");
        assert_eq!(pc1500.read_byte(0x5462), 99, "A(5), el índice límite de DIM A(5)");
    }

    /// Fase 2: `DIM B(2,2)` (3x3 elementos, índices 0..=2 en cada
    /// dimensión) debe usar el número de columnas real (3) en vez del
    /// hardcodeado (10) para calcular `fila*columnas+col`. Verificado
    /// contra la ROM real en tres celdas distintas, incluida la esquina
    /// opuesta `B(2,2)`.
    #[test]
    fn test_oracle_array_2d_constant_size_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 DIM B(2,2)\n20 B(0,0)=1\n30 B(1,1)=2\n40 B(2,2)=3\n50 @(21610)=B(0,0)\n60 @(21611)=B(1,1)\n70 @(21612)=B(2,2)\n80 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 21610/21611/21612 = 0x546A/0x546B/0x546C
        assert_eq!(pc1500.read_byte(0x546A), 1, "B(0,0)");
        assert_eq!(pc1500.read_byte(0x546B), 2, "B(1,1)");
        assert_eq!(pc1500.read_byte(0x546C), 3, "B(2,2), la esquina opuesta");
    }

    /// Un paso más ambicioso que los tests anteriores, combinando en un
    /// solo programa dos bucles `FOR` independientes, un array rellenado
    /// con valores calculados (no literales), y una subrutina `GOSUB`
    /// invocada 10 veces dentro de un bucle (no solo una vez, a diferencia
    /// de `test_oracle_core_subset_end_to_end_on_real_rom`) — más cerca de
    /// la forma de un programa real, siempre dentro del núcleo ya
    /// verificado (sin DATA/READ, cadenas, USING ni reales, que todavía no
    /// están soportados). Guarda los dígitos 0-9 en un array y los
    /// imprime todos vía CHAR_OUT real (con formato decimal de verdad,
    /// ver nota de `test_oracle_core_subset_end_to_end_on_real_rom`),
    /// dando "0123456789" en pantalla — antes de que PRINT formateara
    /// números, este mismo test guardaba códigos ASCII (48-57) en vez de
    /// los dígitos 0-9 para conseguir el mismo resultado por la vía del
    /// PRINT-de-carácter-crudo de entonces; ya no hace falta ese rodeo.
    #[test]
    fn test_oracle_digits_program_end_to_end_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native_with_addresses, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 DIM D(9)\n20 FOR I=0 TO 9\n30 D(I)=I\n40 NEXT I\n50 FOR I=0 TO 9\n60 GOSUB 100\n70 NEXT I\n80 END\n100 PRINT D(I);\n110 RETURN\n";
        let (code, addrs) = compile_native_with_addresses(source);
        let d_addr = *addrs.get("D").expect("dirección de D no encontrada") as u32;

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // D(0)..D(9) deben valer 0..=9 — confirma que el primer bucle
        // calculó y guardó los valores correctamente en el array.
        for i in 0..10u32 {
            assert_eq!(
                pc1500.read_byte(d_addr + i), i as u8,
                "D({i}) debe ser {i}"
            );
        }

        // CHAR_OUT avanza CURSOR_PTR en 6 columnas por carácter; 10
        // dígitos de un solo carácter cada uno impresos (D(I) va de 0 a
        // 9, uno por invocación de GOSUB 100 dentro del segundo bucle)
        // -> 60.
        assert_eq!(pc1500.read_byte(0x7875), 60, "CURSOR_PTR tras imprimir los 10 dígitos");

        let display_touched = (0x7600..0x7600 + 60).any(|addr| pc1500.read_byte(addr) != 0x00);
        assert!(display_touched, "CHAR_OUT debe haber escrito píxeles reales en el buffer de pantalla");
    }

    /// Fase "DATA/READ/RESTORE" (paso hacia bathyscaph.bas, que usa
    /// exactamente este patrón: varias líneas DATA con cadenas, RESTORE a
    /// una línea concreta, READ de una cadena). Cubre: RESTORE sin
    /// argumento (vuelve al principio), lectura secuencial de dos DATA sin
    /// RESTORE entre medias (avanza el índice), y RESTORE a una línea
    /// concreta que salta directamente a esa posición. Verificado contra
    /// la ROM real: cada variable de cadena debe apuntar al contenido
    /// real correspondiente (no solo a "algún puntero").
    #[test]
    fn test_oracle_data_read_restore_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native_with_addresses, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 RESTORE \n20 READ A$\n30 READ B$\n40 RESTORE 1002\n50 READ C$\n60 END\n1000 DATA \"AAAA\"\n1001 DATA \"BBBB\"\n1002 DATA \"CCCC\"\n";
        let (code, addrs) = compile_native_with_addresses(source);
        let a_addr = *addrs.get("A$").expect("dirección de A$ no encontrada") as u32;
        let b_addr = *addrs.get("B$").expect("dirección de B$ no encontrada") as u32;
        let c_addr = *addrs.get("C$").expect("dirección de C$ no encontrada") as u32;

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // Lee un puntero de cadena (2 bytes, alto luego bajo) de `addr` y
        // devuelve el contenido de la cadena a la que apunta.
        let read_string_var = |var_addr: u32| -> String {
            let hi = pc1500.read_byte(var_addr) as u16;
            let lo = pc1500.read_byte(var_addr + 1) as u16;
            let str_addr = ((hi << 8) | lo) as u32;
            let mut s = String::new();
            let mut a = str_addr;
            loop {
                let b = pc1500.read_byte(a);
                if b == 0 {
                    break;
                }
                s.push(b as char);
                a += 1;
                assert!(a - str_addr < 64, "cadena sin terminador NUL cerca de {str_addr:#06x}");
            }
            s
        };

        assert_eq!(read_string_var(a_addr), "AAAA", "A$: RESTORE sin argumento -> primer DATA");
        assert_eq!(read_string_var(b_addr), "BBBB", "B$: READ secuencial sin RESTORE -> siguiente DATA");
        assert_eq!(read_string_var(c_addr), "CCCC", "C$: RESTORE 1002 -> DATA de esa línea concreta");
    }

    /// El patrón exacto de bathyscaph.bas: `DIM A$(0)*20` (un array de
    /// cadena de un elemento, ancho fijo 20) seguido de `READ A$(0)`. A
    /// diferencia de una variable de cadena escalar (que solo guarda un
    /// puntero), esto debe COPIAR los 20 caracteres reales al buffer
    /// reservado — verificado leyendo esos 20 bytes directamente de
    /// memoria, no siguiendo un puntero.
    #[test]
    fn test_oracle_read_into_fixed_width_string_array_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native_with_addresses, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 DIM A$(0)*20\n20 READ A$(0)\n30 END\n1000 DATA \"7163470F1F0F47637160\"\n";
        let (code, addrs) = compile_native_with_addresses(source);
        let a_addr = *addrs.get("A$").expect("dirección de A$ no encontrada") as u32;

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        let expected = "7163470F1F0F47637160";
        let actual: String = (0..20).map(|i| pc1500.read_byte(a_addr + i) as char).collect();
        assert_eq!(actual, expected, "A$(0) debe contener los 20 caracteres copiados, no un puntero");
    }

    /// Comparación de cadenas (`=`/`<>`), incluida `IF Z$=""` — el patrón
    /// exacto de bathyscaph.bas (`IF Z$=""THEN 85`). Debe comparar el
    /// CONTENIDO, no los punteros: A$ y B$ tienen el mismo texto ("HI")
    /// pero son literales/variables distintas (punteros distintos).
    ///
    /// Nota: `IF cond THEN <asignación>` (p.ej. `THEN X=1`) resultó ser un
    /// bug de parser preexistente (se interpreta como un GOTO implícito a
    /// una expresión sin sentido, no como una sentencia) — no lo usa
    /// bathyscaph (solo usa `THEN <número de línea>`), así que este test
    /// usa `THEN <línea>` + `GOTO` en vez de asignar directamente tras
    /// THEN, para no mezclar ese bug ajeno con lo que se está probando
    /// aquí.
    #[test]
    fn test_oracle_string_equality_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 A$=\"HI\"\n20 B$=\"HI\"\n30 C$=\"BYE\"\n40 D$=\"\"\n50 @(21700)=0\n60 @(21701)=0\n70 @(21702)=0\n80 @(21703)=0\n90 IF A$=B$ THEN 130\n100 IF A$=C$ THEN 140\n110 IF D$=\"\" THEN 150\n120 IF A$<>C$ THEN 160\n125 GOTO 170\n130 @(21700)=1\n135 GOTO 100\n140 @(21701)=1\n145 GOTO 110\n150 @(21702)=1\n155 GOTO 120\n160 @(21703)=1\n165 GOTO 125\n170 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        assert_eq!(pc1500.read_byte(0x54C4), 1, "\"HI\"=\"HI\" (contenido igual, punteros distintos) debe ser cierto");
        assert_eq!(pc1500.read_byte(0x54C5), 0, "\"HI\"=\"BYE\" debe ser falso");
        assert_eq!(pc1500.read_byte(0x54C6), 1, "\"\"=\"\" (ambas vacías) debe ser cierto");
        assert_eq!(pc1500.read_byte(0x54C7), 1, "\"HI\"<>\"BYE\" debe ser cierto");
    }

    /// `ASC Z$` — necesario para `H=H-SGN(ASC Z$-10.5)` en bathyscaph.bas.
    #[test]
    fn test_oracle_asc_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 Z$=\"A\"\n20 @(21800)=ASC Z$\n30 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 21800 = 0x5528
        assert_eq!(pc1500.read_byte(0x5528), b'A', "ASC(\"A\") debe ser 65");
    }

    /// `AND`/`OR` bit a bit — el patrón exacto de bathyscaph.bas
    /// (`IF (QAND G)>0`, flags combinados como potencias de 2, no
    /// booleanos lógicos). `5 AND 3 = 1`, `5 OR 2 = 7`, `12 AND 10 = 8`
    /// solo tienen sentido como resultado si la operación es bit a bit.
    #[test]
    fn test_oracle_and_or_int_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 @(21900)=5 AND 3\n20 @(21901)=5 OR 2\n30 @(21902)=12 AND 10\n40 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 21900 = 0x558C, 21901 = 0x558D, 21902 = 0x558E
        assert_eq!(pc1500.read_byte(0x558C), 1, "5 AND 3 debe ser 1 (bit a bit, no booleano)");
        assert_eq!(pc1500.read_byte(0x558D), 7, "5 OR 2 debe ser 7 (bit a bit, no booleano)");
        assert_eq!(pc1500.read_byte(0x558E), 8, "12 AND 10 debe ser 8 (bit a bit, no booleano)");
    }

    /// `^` (exponenciación entera) — necesario para `2^H` en
    /// bathyscaph.bas (`G=INT (2^H+.5)`, aquí probado solo la parte
    /// entera de la potencia; la suma real `+.5` y el `INT` son Fase 4).
    /// Incluye el caso borde exponente=0 (debe dar 1, no 0).
    #[test]
    fn test_oracle_pow_int_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 @(21950)=2^5\n20 @(21951)=3^3\n30 @(21952)=7^0\n40 @(21953)=2^1\n50 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 21950 = 0x55BE, 21951 = 0x55BF, 21952 = 0x55C0, 21953 = 0x55C1
        assert_eq!(pc1500.read_byte(0x55BE), 32, "2^5 debe ser 32");
        assert_eq!(pc1500.read_byte(0x55BF), 27, "3^3 debe ser 27");
        assert_eq!(pc1500.read_byte(0x55C0), 1, "7^0 debe ser 1 (caso borde exponente=0)");
        assert_eq!(pc1500.read_byte(0x55C1), 2, "2^1 debe ser 2");
    }

    /// Ejemplos textuales del Sharp PC-1500 Technical Manual, §5-3-1
    /// ("Expression of decimal number"): confirman byte a byte el formato
    /// antes de usarlo para nada más.
    #[test]
    fn test_f64_to_bcd8_matches_technical_manual_examples() {
        assert_eq!(f64_to_bcd8(1500.0), [0x03, 0x00, 0x15, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(f64_to_bcd8(1.23456), [0x00, 0x00, 0x12, 0x34, 0x56, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_f64_to_bcd8_zero_is_all_zero_bytes() {
        assert_eq!(f64_to_bcd8(0.0), [0x00; 8]);
    }

    #[test]
    fn test_f64_to_bcd8_matches_bathyscaph_literals() {
        // 0.5 = 5.00000000000 x 10^-1
        assert_eq!(f64_to_bcd8(0.5), [0xFF, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00]);
        // 10.5 = 1.05000000000 x 10^1
        assert_eq!(f64_to_bcd8(10.5), [0x01, 0x00, 0x10, 0x50, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_f64_to_bcd8_negative_sets_sign_byte() {
        assert_eq!(f64_to_bcd8(-2.0), [0x00, 0x80, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    /// Aritmética real (BCD, vía `ADDIT`/`SUBTR` reales de la ROM) mezclada
    /// con enteros — el patrón exacto de bathyscaph.bas:
    /// `H=H-SGN (ASC Z$-10.5)` y `G=INT (2^H+.5)`. Cubre las 3 formas de
    /// `CallInt` (1/2/3 dígitos) y ambos signos de `CallSgn`.
    #[test]
    fn test_oracle_real_arithmetic_sgn_int_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 @(21960)=SGN (5-10.5)\n20 @(21961)=SGN (15-10.5)\n30 @(21962)=INT (2^3+.5)\n40 @(21963)=INT (2^5+.5)\n50 @(21964)=INT (2^7+.5)\n60 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // 21960 = 0x55C8 .. 21964 = 0x55CC
        assert_eq!(pc1500.read_byte(0x55C8), 0xFF, "SGN(5-10.5)=SGN(-5.5) debe ser -1 (0xFF)");
        assert_eq!(pc1500.read_byte(0x55C9), 1, "SGN(15-10.5)=SGN(4.5) debe ser 1");
        assert_eq!(pc1500.read_byte(0x55CA), 8, "INT(2^3+.5)=INT(8.5) debe ser 8 (1 dígito)");
        assert_eq!(pc1500.read_byte(0x55CB), 32, "INT(2^5+.5)=INT(32.5) debe ser 32 (2 dígitos)");
        assert_eq!(pc1500.read_byte(0x55CC), 128, "INT(2^7+.5)=INT(128.5) debe ser 128 (3 dígitos)");
    }

    /// Verificación combinada de TODO lo añadido en esta sesión en un
    /// único programa, para descartar interacciones entre piezas que los
    /// tests individuales (por diseño, aislados) no pueden ver — arrays +
    /// FOR/NEXT + MulInt, DATA/READ de una cadena, aritmética real
    /// mezclada con enteros (`INT`/`SGN`), `AND` bit a bit, `^`,
    /// comparación de cadenas, y `GOSUB`/`RETURN`, todo en la misma
    /// pasada, con los resultados volcados a memoria para poder
    /// comprobarlos de una vez tras la ejecución. Sigue el patrón
    /// "`IF cond THEN <línea>` + `GOTO`" en vez de `THEN <asignación>`
    /// (bug de parser preexistente y ajeno, documentado en
    /// `test_oracle_string_equality_on_real_rom`).
    #[test]
    fn test_oracle_full_session_feature_combination_end_to_end_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 DIM D(3)\n\
20 FOR I=0TO 3\n\
30 D(I)=I*I\n\
40 NEXT I\n\
50 DATA \"OK\"\n\
60 READ Z$\n\
70 H=3\n\
80 G=INT (2^H+.5)\n\
90 Q=8\n\
100 IF (QAND G)>0THEN 120\n\
110 GOTO 130\n\
120 R=1\n\
130 S=SGN (5-10.5)\n\
140 T=SGN (15-10.5)\n\
150 U=2^5\n\
160 IF Z$=\"OK\"THEN 180\n\
170 GOTO 190\n\
180 V=1\n\
190 GOSUB 300\n\
200 @(21500)=D(0)\n\
210 @(21501)=D(1)\n\
220 @(21502)=D(2)\n\
230 @(21503)=D(3)\n\
240 @(21504)=G\n\
250 @(21505)=R\n\
260 @(21506)=S\n\
270 @(21507)=T\n\
280 @(21508)=U\n\
290 @(21509)=V\n\
291 @(21510)=W\n\
292 END\n\
300 W=99\n\
310 RETURN\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 30000);

        // 21500 = 0x53FC .. 21510 = 0x5406
        assert_eq!(pc1500.read_byte(0x53FC), 0, "D(0) = 0*0");
        assert_eq!(pc1500.read_byte(0x53FD), 1, "D(1) = 1*1");
        assert_eq!(pc1500.read_byte(0x53FE), 4, "D(2) = 2*2");
        assert_eq!(pc1500.read_byte(0x53FF), 9, "D(3) = 3*3");
        assert_eq!(pc1500.read_byte(0x5400), 8, "G = INT(2^3+.5) = 8");
        assert_eq!(pc1500.read_byte(0x5401), 1, "R = 1 ((8 AND 8)>0 es cierto)");
        assert_eq!(pc1500.read_byte(0x5402), 0xFF, "S = SGN(5-10.5) = -1");
        assert_eq!(pc1500.read_byte(0x5403), 1, "T = SGN(15-10.5) = 1");
        assert_eq!(pc1500.read_byte(0x5404), 32, "U = 2^5 = 32");
        assert_eq!(pc1500.read_byte(0x5405), 1, "V = 1 (Z$=\"OK\" tras READ es cierto)");
        assert_eq!(pc1500.read_byte(0x5406), 99, "W = 99 (asignado en la subrutina de GOSUB 300)");
    }

    /// `CLS`, `CURSOR`, `GCURSOR`, `POKE#` y `CLEAR` (no-op) — el patrón
    /// exacto de bathyscaph.bas: `CLS :WAIT 0:CLEAR :... :GCURSOR 10`.
    #[test]
    fn test_oracle_cls_cursor_gcursor_poke_clear_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        // Ensuciar el buffer de pantalla y el cursor antes de CLS, para
        // comprobar que de verdad los limpia (no solo que "no crashea").
        let source = "\
10 POKE# 30208,255\n\
20 CURSOR 20\n\
30 CLS\n\
40 CURSOR 5\n\
50 GCURSOR 42\n\
60 POKE# 22000,123\n\
70 CLEAR \n\
80 END\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 30208 = 0x7600 (primer byte del buffer de pantalla).
        assert_eq!(pc1500.read_byte(0x7600), 0, "CLS debe poner a 0 el buffer de pantalla");
        // Tras CURSOR 5, CURSOR_PTR = 5*6 = 30; tras GCURSOR 42, CURSOR_PTR = 42.
        assert_eq!(pc1500.read_byte(0x7875), 42, "GCURSOR 42 debe dejar CURSOR_PTR = 42 (sin *6)");
        assert_eq!(pc1500.read_byte(0x7874) & 0x01, 1, "CURSOR_ENA bit0 debe quedar activado");
        // 22000 = 0x55F0
        assert_eq!(pc1500.read_byte(0x55F0), 123, "POKE# escribe directamente en memoria absoluta");
    }

    /// `WAIT n` llama a `TIME_DELAY` real, que sondea el bit de la señal
    /// cuadrada del reloj (`PC1500_PRT_B` bit5) hasta ver una transición —
    /// pero el arnés de test (`run_lh5*`, que solo llama a `step_cpu()`)
    /// no avanza los periféricos (`Pc1500::step()`, que sí actualiza
    /// `pd1990ac`/`lh5810`, es privado y solo se ejecuta desde
    /// `Pc1500::run()`/`step_frame()`), así que ese bit nunca cambia en
    /// este arnés y `TIME_DELAY` nunca completaría un `run_lh5_until_exit`
    /// (bucle infinito genuino DEL ARNÉS, no del código generado). Lo que
    /// SÍ se puede verificar aquí sin colgar el test: que la llamada no
    /// se cae a memoria inválida — tras varios miles de pasos, el PC debe
    /// seguir dentro del cuerpo de `TIME_DELAY` ($E88C-$E8C0), sondeando
    /// tranquilamente, no en una dirección arbitraria (opcode ilegal).
    #[test]
    fn test_oracle_wait_calls_time_delay_and_polls_without_crashing_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5, ORACLE_LOAD_ADDR};

        let source = "10 WAIT 5\n20 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5(ORACLE_LOAD_ADDR, &code, 2000);

        // TIME_DELAY ($E88C-$E8C0) llama, dentro de su propio bucle de
        // sondeo, a una subrutina auxiliar vía VMJ que aterriza en
        // $E451-$E456 (confirmado paso a paso) — ambos rangos son
        // ejecución legítima DENTRO de TIME_DELAY, no un salto a memoria
        // arbitraria.
        let pc = pc1500.cpu().p();
        assert!(
            (0xE88C..=0xE8C0).contains(&pc) || (0xE451..=0xE456).contains(&pc),
            "tras 2000 pasos el PC debería seguir dentro de TIME_DELAY o su subrutina auxiliar (sondeando), no en {pc:#06X}"
        );
    }

    /// `PRINT` (modo texto, vía `SystemOutInt`/`SystemOutString` +
    /// `CHAR_OUT`) para números (positivo, negativo, cero) y una cadena
    /// literal. Encontrado con este mismo oráculo: el antiguo `SystemOut`
    /// (una sola instrucción para todo) hacía `pop_a` de 1 byte siempre,
    /// así que para una cadena (puntero de 16 bits) descuadraba la pila
    /// hardware por un byte y acababa en un opcode ilegal en cuanto el
    /// siguiente `CHAR_OUT`/`RTN` intentaba devolver el control — y para
    /// un número imprimía el CARÁCTER cuyo código coincide con el valor
    /// (p.ej. 65 -> 'A'), no el texto "65". La extracción de dígitos en
    /// sí ya está probada por separado (`CallStr`/`STR$`, que escribe en
    /// un buffer plano verificable byte a byte); aquí se verifica que
    /// `SystemOutInt` la enruta al número correcto de `CHAR_OUT` — cada
    /// carácter de texto avanza `CURSOR_PTR` en 6 (misma multiplicación
    /// que la sentencia `CURSOR`), así que el avance total delata
    /// exactamente cuántos caracteres (dígitos + signo) se imprimieron.
    #[test]
    fn test_oracle_print_int_and_string_cursor_advance_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        // Cada PRINT añade un salto de línea (CR, 1 carácter más).
        let cases: &[(&str, u8)] = &[
            ("10 PRINT 65\n20 END\n", (2 + 1) * 6),   // "65" + CR
            ("10 PRINT 5\n20 END\n", (1 + 1) * 6),    // "5" + CR
            ("10 PRINT 100\n20 END\n", (3 + 1) * 6),  // "100" + CR
            ("10 PRINT 0\n20 END\n", (1 + 1) * 6),    // "0" + CR
            ("10 PRINT -5\n20 END\n", (2 + 1) * 6),   // "-5" + CR
            ("10 PRINT \"HI\"\n20 END\n", (2 + 1) * 6), // "HI" + CR
        ];

        for (source, expected_cursor) in cases {
            let code = compile_native(source);
            let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);
            let cursor_ptr = pc1500.read_byte(0x7875);
            assert_eq!(
                cursor_ptr, *expected_cursor,
                "CURSOR_PTR tras `{}` (esperado {} caracteres * 6)",
                source.lines().next().unwrap(), expected_cursor / 6
            );
        }
    }

    /// `GPRINT` de un valor numérico (1 byte = 1 columna) y de una cadena
    /// (cada byte = una columna, avanzando el cursor) — el patrón exacto
    /// de bathyscaph.bas (`GPRINT A$(0);` / `GPRINT "141C"` / `GPRINT
    /// S;...`). Verifica el byte reconstruido en el buffer de pantalla
    /// con la MISMA fórmula que usa `ceres-core::display.rs` para
    /// renderizar (`low(adr) | low(adr+1)<<4`), derivada independientemente
    /// del código del backend.
    #[test]
    fn test_oracle_gprint_numeric_and_string_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        // "3C" en vez de "XY": el GPRINT real de la ROM interpreta una
        // cadena como texto de pares de dígitos HEXADECIMALES (ver
        // `emit_hex_digit_to_nibble`), así que el contenido de prueba
        // tiene que ser hex válido para probar el camino real, no
        // caracteres arbitrarios.
        let source = "10 DIM A$(0)*2\n20 A$(0)=\"3C\"\n30 GCURSOR 0\n40 GPRINT 5\n50 GPRINT 3\n60 GCURSOR 39\n70 GPRINT \"AB\"\n80 GCURSOR 78\n90 GPRINT A$(0)\n100 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        let recon = |adr: u32| -> u8 {
            (pc1500.read_byte(adr) & 0x0F) | ((pc1500.read_byte(adr + 1) & 0x0F) << 4)
        };
        // Columna 78 usa el NIBBLE ALTO del mismo par $7600+.
        let recon_high = |adr: u32| -> u8 {
            ((pc1500.read_byte(adr) >> 4) & 0x0F) | (((pc1500.read_byte(adr + 1) >> 4) & 0x0F) << 4)
        };

        assert_eq!(recon(0x7600), 5, "GPRINT 5 en columna 0");
        assert_eq!(recon(0x7602), 3, "GPRINT 3 en columna 1 (cursor auto-avanzado)");
        // "AB" son 2 CARACTERES = 1 par hex = 1 sola columna (0xAB=171),
        // no 2 columnas de los bytes crudos 'A'/'B' como antes de este
        // arreglo — confirma que ya no se dibuja el doble de columnas
        // de las que corresponden.
        assert_eq!(recon(0x7700), 0xAB, "GPRINT \"AB\" columna 39: 1 columna, valor decodificado 0xAB");
        // 0xFF, no 0x00: este test nunca llama a CLS, así que la memoria
        // de vídeo está en su valor por defecto sin inicializar (ver
        // `INITIAL_VALUE` en memory.rs), no a cero.
        assert_eq!(recon(0x7702), 0xFF, "columna 40 no debe tocarse: \"AB\" es 1 sola columna, no 2");
        // A$(0)="3C" (array de ancho fijo, 2 caracteres = 1 par hex):
        // igual, 1 sola columna con el valor decodificado 0x3C.
        assert_eq!(recon_high(0x7600), 0x3C, "GPRINT A$(0)=\"3C\" columna 78: 1 columna, valor decodificado 0x3C");
    }

    /// `POINT(x)` debe leer exactamente lo que `GPRINT` escribió — el
    /// patrón exacto de bathyscaph.bas (`Q=POINT P`, tras dibujar el
    /// mapa con `GPRINT A$(0)`). Cubre las 4 combinaciones base/nibble
    /// (columnas 0, 39, 78, 155: par/impar x bajo/alto). Valores <128
    /// (bit7=0) para no depender de si GPRINT_OUT preserva ese bit, que
    /// el propio display de 7 filas ignora al renderizar.
    #[test]
    fn test_oracle_point_reads_back_what_gprint_wrote_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 GCURSOR 0\n\
20 GPRINT 42\n\
30 GCURSOR 39\n\
40 GPRINT 85\n\
50 GCURSOR 78\n\
60 GPRINT 100\n\
70 GCURSOR 155\n\
80 GPRINT 15\n\
90 @(22100)=POINT 0\n\
100 @(22101)=POINT 39\n\
110 @(22102)=POINT 78\n\
120 @(22103)=POINT 155\n\
130 END\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 8000);

        // 22100 = 0x5654 .. 22103 = 0x5657
        assert_eq!(pc1500.read_byte(0x5654), 42, "POINT(0): base par, nibble bajo");
        assert_eq!(pc1500.read_byte(0x5655), 85, "POINT(39): base impar, nibble bajo");
        assert_eq!(pc1500.read_byte(0x5656), 100, "POINT(78): base par, nibble alto");
        assert_eq!(pc1500.read_byte(0x5657), 15, "POINT(155): base impar, nibble alto");
    }

    /// `BEEP a,b,c` — el patrón exacto de bathyscaph.bas (`BEEP 1,0,1`).
    /// El sonido en sí no es verificable por estado de memoria, pero
    /// confirma lo que sí importa para la corrección del programa: que
    /// la(s) llamada(s) a la rutina ROM real `BEEP` terminan limpiamente
    /// y el programa sigue ejecutándose después (a diferencia de `WAIT`,
    /// `BEEP` cuenta su temporización con un bucle software, no un
    /// periférico externo, así que sí completa en este arnés).
    #[test]
    fn test_oracle_beep_returns_cleanly_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 BEEP 2,1,1\n20 @(22200)=1\n30 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 500_000);

        // 22200 = 0x56B8
        assert_eq!(pc1500.read_byte(0x56B8), 1, "el programa debe seguir tras BEEP (llama a BEEP 2 veces y vuelve limpiamente)");
    }

    /// Multi-asignación (`H=3,G=8`) e identificador built-in `TIME` — el
    /// patrón exacto de bathyscaph.bas (`TIME =0,H=3,G=8`). `TIME` se
    /// trata como una variable normal (no un reloj en tiempo real: no hay
    /// mecanismo de interrupciones/temporizador implementado en este
    /// backend) — bathyscaph solo lo usa para mostrarlo al final de la
    /// partida (`PRINT "TIME :";TIME *100`), no para lógica de juego, así
    /// que basta con que se pueda asignar y leer como cualquier otra
    /// variable.
    #[test]
    fn test_oracle_multi_assignment_and_time_identifier_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 TIME =0,H=3,G=8\n20 @(22400)=H\n30 @(22401)=G\n40 @(22402)=TIME \n50 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 22400 = 0x5780, 22401 = 0x5781, 22402 = 0x5782
        assert_eq!(pc1500.read_byte(0x5780), 3, "H=3 (segunda asignación de la línea multi-asignación)");
        assert_eq!(pc1500.read_byte(0x5781), 8, "G=8 (tercera asignación)");
        assert_eq!(pc1500.read_byte(0x5782), 0, "TIME=0 (primera asignación, TIME tratado como variable normal)");
    }

    /// Exploratorio (no de regresión estricta): compila el bathyscaph.bas
    /// REAL del corpus y lo ejecuta muchos pasos contra la ROM real, para
    /// confirmar que el binario completo no colisiona con memoria propia
    /// (arreglo de `DATA_BASE`) ni cae en un opcode ilegal — es un juego
    /// interactivo con bucle infinito esperando teclado (`INKEY$` sin
    /// tecla pulsada = cadena vacía en este arnés), así que nunca
    /// "termina" solo; el objetivo es solo confirmar que corre de forma
    /// sostenida sin corromperse.
    ///
    /// Usa `step_frame()`, no `step_cpu()` en bucle (ver el test
    /// equivalente de rasemottes.bas para por qué esto importa: un
    /// `step_cpu()` puro puede quedarse "atascado" de forma segura en un
    /// `WAIT` con argumento no trivial, dando una falsa sensación de
    /// corrección sin haber ejercitado el camino de ejecución real).
    #[test]
    fn test_oracle_bathyscaph_runs_sustained_without_crashing_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, load, ORACLE_LOAD_ADDR};

        let source = std::fs::read_to_string("test/basic/bathyscaph.bas")
            .expect("no se pudo leer test/basic/bathyscaph.bas");
        let code = compile_native(&source);
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);

        for _ in 0..300 {
            pc1500.step_frame();
        }

        let pc = pc1500.cpu().p();
        assert!(
            (0x4000..=0x57FF).contains(&pc) || (0xC000..=0xFFFF).contains(&pc),
            "tras 300 frames el PC debería seguir en memoria de usuario o ROM, no en {pc:#06X} (posible corrupción/salto a memoria inválida)"
        );
    }

    /// Segundo programa clásico real (más allá de bathyscaph.bas) usado
    /// como prueba de consistencia: "Rase-mottes" (E. Beaurepaire, no
    /// publicado), un juego de avioneta con mecánica muy similar a
    /// bathyscaph (RESTORE+READ+GPRINT de un perfil de terreno,
    /// POINT+AND para colisión, INKEY$ para control), pero de un autor y
    /// estructura distintos — sirve para confirmar que el backend no es
    /// un "one-hit wonder" ajustado solo a bathyscaph. Usa además
    /// `RANDOM` y `PRINT USING`/`USING` (esta última no implementada en
    /// el backend — cae en el catch-all sin argumentos en la pila, así
    /// que es un no-op limpio: no corrompe la pila, solo hace que los
    /// números salgan sin el formato "*####" con el que se imprimirían
    /// en la ROM real).
    ///
    /// Documenta un límite real y actual, no lo esconde: al arreglar el
    /// bug de fuga de pila en `SGN`/`INT` (ver
    /// `test_oracle_int_and_sgn_promote_non_real_argument_on_real_rom`),
    /// el código nativo generado para rasemottes.bas creció de 5780 a
    /// 6140 bytes — y la RAM de usuario disponible es 6144 bytes
    /// (`0x4000-0x57FF`) para código + variables + pila software
    /// COMBINADOS (ver el comentario histórico junto a `ArrayMeta` en
    /// `codegen/mod.rs`). El programa ya NO CABE: solo quedan 4 bytes
    /// para variables+pila tras el código. Antes de este fix cabía de
    /// milagro (5780 bytes) precisamente porque el bug de fuga de pila
    /// significaba que el backend generaba MENOS código del que un
    /// `INT`/`SGN` correcto necesita (sin la promoción a real que ahora
    /// añadimos). Un código máquina correcto pesa más que uno con un
    /// bug de omisión — no hay contradicción, pero sí confirma
    /// concretamente, con un segundo programa real del corpus (no solo
    /// una advertencia teórica), la limitación de tamaño ya encontrada
    /// al probar el corpus completo: sin optimización de tamaño de
    /// código, muchos programas clásicos reales no caben en la RAM
    /// disponible. No es un bug de este test ni de rasemottes.bas —
    /// documentarlo aquí para que, si en el futuro se añade una fase de
    /// reducción de tamaño de código, este test empiece a fallar (señal
    /// de que hay que actualizarlo, no de que algo se ha roto).
    #[test]
    fn test_oracle_rasemottes_currently_exceeds_available_ram_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, ORACLE_LOAD_ADDR};

        let source = std::fs::read_to_string("test/basic/rasemottes.bas")
            .expect("no se pudo leer test/basic/rasemottes.bas");
        let code = compile_native(&source);

        // Mismo límite que valida `load_lh5_file` en la carga real: el
        // código (sin contar siquiera variables/pila) debe terminar
        // dentro de `standard_user_memory` (hasta 0x57FF inclusive).
        let code_end = ORACLE_LOAD_ADDR as usize + code.len();
        assert!(
            code_end > 0x5800,
            "rasemottes.bas ahora cabe en la RAM de usuario (código termina en {code_end:#06X}, \
             límite 0x5800) — actualizar/eliminar este test y recuperar la prueba de ejecución \
             sostenida contra la GUI real"
        );
    }

    /// Un `IF` sin bloque explícito (sin `THEN`, o incluso con `THEN`)
    /// cuyo consecuente son VARIAS sentencias separadas por `:` — p.ej.
    /// `IF cond stmt1:stmt2` — debe hacer TODAS las sentencias
    /// condicionales, no solo la primera. Encontrado compilando
    /// rasemottes.bas (segundo programa clásico real probado, más allá
    /// de bathyscaph.bas): su línea `40 IF INKEY$ <>"" CLS :GOTO 70`
    /// compilaba con el `GOTO 70` FUERA del bloque condicional (se
    /// ejecutaba siempre, no solo cuando se pulsaba una tecla), lo que
    /// sacaba al programa de su bucle de espera inmediatamente y
    /// terminaba corrompiendo la pila de software más adelante (`Illegal
    /// opcode 0xff at PC 0x0000` en la GUI real). El propio parser
    /// (`parse_if_stmt`) solo capturaba una única sentencia como
    /// consecuente; el bucle de `parse_code_line_with_recovery` que lo
    /// llama trataba cualquier sentencia posterior separada por ':' como
    /// independiente. Arreglado haciendo que `parse_if_stmt` absorba el
    /// resto de la línea (agrupándolas en `StatementInner::Multi` cuando
    /// hay más de una) — el comportamiento estándar de BASIC clásico
    /// (incluido este dialecto Sharp PC-1500), donde todo lo que sigue a
    /// un IF sin bloque, hasta el final de la línea, es condicional.
    #[test]
    fn test_oracle_if_with_multiple_colon_separated_statements_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native_with_addresses, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        // Condición FALSA: NINGUNA de las dos sentencias debe ejecutarse.
        // Si el bug estuviera presente (solo la primera sentencia queda
        // guardada por el IF), B pasaría a 1 igualmente.
        let false_source = "10 A=0:B=0\n20 IF 1=0 A=1:B=1\n30 END\n";
        let (false_code, false_addrs) = compile_native_with_addresses(false_source);
        let a_addr = *false_addrs.get("A").expect("dirección de A no encontrada") as u32;
        let b_addr = *false_addrs.get("B").expect("dirección de B no encontrada") as u32;
        let pc1500_false = run_lh5_until_exit(ORACLE_LOAD_ADDR, &false_code, ORACLE_STACK_TOP, 5000);
        assert_eq!(pc1500_false.read_byte(a_addr), 0, "condición falsa: A no debe tocarse");
        assert_eq!(
            pc1500_false.read_byte(b_addr), 0,
            "condición falsa: B (segunda sentencia tras ':') tampoco debe tocarse — si se ejecutara, confirmaría el bug de que solo la primera sentencia quedaba guardada por el IF"
        );

        // Condición VERDADERA: AMBAS sentencias deben ejecutarse.
        let true_source = "10 A=0:B=0\n20 IF 1=1 A=7:B=8\n30 END\n";
        let (true_code, true_addrs) = compile_native_with_addresses(true_source);
        let a_addr2 = *true_addrs.get("A").expect("dirección de A no encontrada") as u32;
        let b_addr2 = *true_addrs.get("B").expect("dirección de B no encontrada") as u32;
        let pc1500_true = run_lh5_until_exit(ORACLE_LOAD_ADDR, &true_code, ORACLE_STACK_TOP, 5000);
        assert_eq!(pc1500_true.read_byte(a_addr2), 7, "condición verdadera: A=7 (primera sentencia) debe ejecutarse");
        assert_eq!(pc1500_true.read_byte(b_addr2), 8, "condición verdadera: B=8 (segunda sentencia) también debe ejecutarse");
    }

    /// `INT(x)`/`SGN(x)` esperan su argumento ya empujado como real de 8
    /// bytes (`emit_pop_8_to`) — si `x` no contiene ningún literal
    /// decimal (p.ej. `INT(A/4)`, con A variable y 4 literal entero),
    /// `is_real_expr` decide correctamente `DivInt` para la propia
    /// división (produce 1 byte), pero antes de este fix nada
    /// promocionaba ese resultado a real antes de `CallInt`/`CallSgn`:
    /// fuga de 7 bytes en la pila software por cada llamada. Encontrado
    /// compilando rasemottes.bas (segundo programa clásico real
    /// probado): su línea `130 GPRINT R;I OR A:BEEP 1,0,1:M=M+INT
    /// (A/4):...` desbordaba `S` casi al instante al mantener pulsada la
    /// tecla de control, con `Illegal opcode 0xff at PC 0x0000` en la GUI
    /// real. bathyscaph.bas nunca lo expuso porque sus únicos usos de
    /// `SGN`/`INT` ya eran reales por construcción (contenían un literal
    /// decimal: `SGN(ASC Z$-10.5)`, `INT(2^H+.5)`).
    #[test]
    fn test_oracle_int_and_sgn_promote_non_real_argument_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, load, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        // `INT(A/4)`/`SGN(A-4)` repetidos muchas veces en un bucle
        // ajustado — con la fuga de 7 bytes/llamada, S se pasaría de
        // `stack_top` en un puñado de vueltas.
        let source = "\
10 A=1:M=0\n\
20 M=M+INT (A/4)+SGN (A-4)\n\
30 A=A*2:IF A>64 LET A=1\n\
40 GOTO 20\n\
";
        let code = compile_native(source);
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);

        for frame in 0..50 {
            pc1500.step_frame();
            let s = pc1500.cpu().s();
            assert!(
                s <= ORACLE_STACK_TOP,
                "frame {frame}: S se pasó de stack_top ({ORACLE_STACK_TOP:#06X}): {s:#06X} \
                 (fuga de pila en INT/SGN sobre un argumento no-real)"
            );
        }
    }

    /// Regresión directa del bug de la barra negra en las primeras
    /// columnas del display: `DATA_BASE` era una constante fija (`0x5600`)
    /// elegida "a ojo" para caber por delante del código generado en su
    /// momento. Cada vez que el compilador ganaba una feature nueva (p.ej.
    /// `INKEY$`, `RND` de 16 bits, el `GPRINT` de cadenas con decodificado
    /// hexadecimal) el código de bathyscaph.bas creció un poco más, hasta
    /// que dos veces distintas ese crecimiento superó la constante: las
    /// variables (p.ej. `S`/`R`/`Q` en bathyscaph) empezaban a vivir
    /// DENTRO del propio código todavía sin ejecutar, así que su primera
    /// lectura (antes de la primera asignación real) devolvía bytes de
    /// código/pool de datos en vez de basura-cero, y cada escritura
    /// posterior corrompía esos mismos bytes de código — visible en el
    /// display real como una barra sólida en las columnas 0-9 (el primer
    /// `GPRINT S` de bathyscaph, que solo se ejecuta una vez, quedaba con
    /// ese valor "leído de código" en vez de 0). `compile_native_two_pass`
    /// sustituyó la constante por un cálculo dinámico
    /// (`data_base = dirección_de_carga + tamaño_real_del_código`); este
    /// test verifica esa invariante directamente para el bathyscaph.bas
    /// real del corpus, no solo que "no crashea" (ver el test de arriba,
    /// que ya perseguía este mismo objetivo pero con una aserción
    /// demasiado débil para detectar corrupción silenciosa sin salto a
    /// memoria inválida).
    #[test]
    fn test_oracle_bathyscaph_variables_never_overlap_code_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native_with_addresses, ORACLE_LOAD_ADDR};

        let source = std::fs::read_to_string("test/basic/bathyscaph.bas")
            .expect("no se pudo leer test/basic/bathyscaph.bas");
        let (code, addrs) = compile_native_with_addresses(&source);

        let code_end = ORACLE_LOAD_ADDR as usize + code.len();
        let min_var_addr = addrs
            .values()
            .copied()
            .min()
            .expect("el programa debe declarar al menos una variable");

        assert!(
            min_var_addr >= code_end,
            "una variable empieza en {min_var_addr:#06X}, dentro del código real (termina en {code_end:#06X}) — \
             las variables corromperían/leerían código todavía no ejecutado"
        );
    }

    /// `RND(n)` — el patrón exacto de bathyscaph.bas (`RND 16`, `RND
    /// 256-1`). Este backend NO implementa el algoritmo real de la ROM
    /// (ver comentario de `StackInstruction::CallRnd` en el backend);
    /// usa un LFSR de Galois de 8 bits autocontenido en su lugar. Se
    /// verifica lo que sí importa para la corrección de un generador
    /// pseudoaleatorio sustituto: el resultado siempre cabe en `[0, n)`,
    /// dos llamadas consecutivas dan valores distintos (el estado
    /// avanza), y `RND(0)` no cuelga el programa (caso borde explícito).
    #[test]
    fn test_oracle_rnd_lfsr_stays_in_range_and_advances_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 @(22500)=RND (16)\n\
20 @(22501)=RND (16)\n\
30 @(22502)=RND (16)\n\
40 @(22503)=RND (0)\n\
50 END\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 10000);

        // 22500 = 0x57E4 .. 22503 = 0x57E7
        let a = pc1500.read_byte(0x57E4);
        let b = pc1500.read_byte(0x57E5);
        let c = pc1500.read_byte(0x57E6);
        let zero_case = pc1500.read_byte(0x57E7);

        assert!(a < 16, "RND(16) debe dar un valor < 16, dio {a}");
        assert!(b < 16, "RND(16) debe dar un valor < 16, dio {b}");
        assert!(c < 16, "RND(16) debe dar un valor < 16, dio {c}");
        assert!(
            a != b || b != c,
            "llamadas consecutivas a RND no deberían dar SIEMPRE el mismo valor (el LFSR debe avanzar): {a}, {b}, {c}"
        );
        assert_eq!(zero_case, 0, "RND(0) debe dar 0 (caso sin rango), no colgarse");
    }

    /// `IF cond THEN <asignación sin LET>` — bug de parser preexistente
    /// (ajeno a la generación de código: `IF...THEN X=5` se malinterpretaba
    /// como un GOTO implícito a la expresión `X=5`) corregido igualando
    /// `is_let_mandatory` en la cláusula THEN al resto de la gramática
    /// (`LET` siempre opcional). No lo necesita bathyscaph.bas, pero es
    /// un patrón común en BASIC en general.
    #[test]
    fn test_oracle_if_then_assignment_without_let_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 X=1\n20 IF 1=1THEN X=5\n30 IF 1=2THEN X=9\n40 @(21600)=X\n50 END\n";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 5000);

        // 21600 = 0x5460 (dentro de 0x4000-0x57FF; 22600/0x5848 usado en un
        // intento anterior de este test EXCEDE 0x57FF, el tope real de
        // standard_user_memory — la escritura ahí no persiste, lo cual no
        // es un bug del compilador sino una dirección de prueba inválida).
        assert_eq!(
            pc1500.read_byte(0x5460), 5,
            "IF 1=1 THEN X=5 (condición cierta) debe asignar 5; IF 1=2 THEN X=9 (falsa) no debe ejecutarse"
        );
    }

    /// Funciones de cadena (`LEN`, `LEFT$`, `RIGHT$`, `MID$`, `STR$`,
    /// `VAL`) sobre una variable de cadena escalar (puntero
    /// NUL-terminado) — el caso más simple, antes de probar arrays de
    /// ancho fijo (que el corpus usa mucho más, en el siguiente test).
    #[test]
    fn test_oracle_string_functions_on_scalar_var_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 A$=\"HELLO WORLD\"\n\
20 @(21700)=LEN A$\n\
30 @(21701)=ASC LEFT$ (A$,5)\n\
40 @(21702)=ASC RIGHT$ (A$,5)\n\
50 @(21703)=ASC MID$ (A$,7,5)\n\
60 @(21704)=ASC STR$ (42)\n\
70 @(21705)=VAL (\"123\")\n\
80 @(21706)=LEN LEFT$ (A$,5)\n\
90 END\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // 21700 = 0x54C4 .. 21706 = 0x54CA
        assert_eq!(pc1500.read_byte(0x54C4), 11, "LEN(\"HELLO WORLD\") = 11");
        assert_eq!(pc1500.read_byte(0x54C5), b'H', "LEFT$(A$,5) = \"HELLO\", primer carácter 'H'");
        assert_eq!(pc1500.read_byte(0x54C6), b'W', "RIGHT$(A$,5) = \"WORLD\", primer carácter 'W'");
        assert_eq!(pc1500.read_byte(0x54C7), b'W', "MID$(A$,7,5) = \"WORLD\", primer carácter 'W'");
        assert_eq!(pc1500.read_byte(0x54C8), b'4', "STR$(42) = \"42\", primer carácter '4'");
        assert_eq!(pc1500.read_byte(0x54C9), 123, "VAL(\"123\") = 123");
        assert_eq!(pc1500.read_byte(0x54CA), 5, "LEN(LEFT$(A$,5)) = 5 (anidamiento de funciones distintas)");
    }

    /// Funciones de cadena sobre un elemento de array de cadena de ancho
    /// fijo (`DIM S$(0)*5`, NO NUL-terminado) — el patrón dominante en el
    /// corpus real (`LEFT$(S$(0),n)`, `MID$(O$(I)`, etc.). También cubre
    /// el "clamp": pedir más caracteres de los que hay disponibles no
    /// debe leer basura más allá del ancho declarado.
    #[test]
    fn test_oracle_string_functions_on_fixed_width_array_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 DIM S$(0)*5\n\
20 S$(0)=\"ABCDE\"\n\
30 @(21710)=LEN S$(0)\n\
40 @(21711)=ASC LEFT$ (S$(0),2)\n\
50 @(21712)=ASC RIGHT$ (S$(0),2)\n\
60 @(21713)=ASC MID$ (S$(0),3,2)\n\
70 @(21714)=LEN LEFT$ (S$(0),100)\n\
80 @(21715)=LEN RIGHT$ (S$(0),100)\n\
90 END\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // 21710 = 0x54CE .. 21715 = 0x54D3
        assert_eq!(pc1500.read_byte(0x54CE), 5, "LEN(S$(0)) = 5 (ancho fijo declarado, sin NUL)");
        assert_eq!(pc1500.read_byte(0x54CF), b'A', "LEFT$(S$(0),2) = \"AB\", primer carácter 'A'");
        assert_eq!(pc1500.read_byte(0x54D0), b'D', "RIGHT$(S$(0),2) = \"DE\", primer carácter 'D'");
        assert_eq!(pc1500.read_byte(0x54D1), b'C', "MID$(S$(0),3,2) = \"CD\", primer carácter 'C'");
        assert_eq!(pc1500.read_byte(0x54D2), 5, "LEFT$(S$(0),100) debe recortarse (clamp) a los 5 disponibles");
        assert_eq!(pc1500.read_byte(0x54D3), 5, "RIGHT$(S$(0),100) debe recortarse (clamp) a los 5 disponibles");
    }

    /// `RESTORE <constante> + <expresión>` — el patrón exacto de
    /// bathyscaph.bas (`RESTORE 999+RND 16`), usando una variable en vez
    /// de `RND` para que el resultado sea determinista y comprobable.
    /// Antes de esta sesión caía siempre en "reiniciar desde el
    /// principio de DATA"; ahora usa aritmética de 16 bits real
    /// (`SumaIntWord`) para calcular la línea de destino.
    #[test]
    fn test_oracle_restore_dynamic_constant_plus_expr_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 DATA \"AAAA\"\n\
11 DATA \"BBBB\"\n\
12 DATA \"CCCC\"\n\
20 X=1\n\
30 RESTORE 1000+X\n\
40 READ A$\n\
50 @(21600)=ASC A$\n\
60 END\n\
1000 DATA \"DDDD\"\n\
1001 DATA \"EEEE\"\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // 21600 = 0x5460
        assert_eq!(
            pc1500.read_byte(0x5460), b'E',
            "RESTORE 1000+X (X=1) -> línea 1001 -> READ A$ debe dar \"EEEE\" (primer carácter 'E')"
        );
    }

    /// `GOTO`/`GOSUB` calculado — antes causaba un panic en tiempo de
    /// compilación ("Undefined label") para cualquier destino que no
    /// fuera un número de línea literal o una etiqueta de cadena. Cubre
    /// el caso común (variable pequeña, ≤255, extendida a 16 bits) y el
    /// patrón `<constante>+<expresión>` (misma mecánica que `RESTORE
    /// 999+RND 16`, aquí determinista con una variable en vez de `RND`).
    #[test]
    fn test_oracle_computed_goto_gosub_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 D=30\n\
20 GOTO D\n\
25 @(21600)=1\n\
26 GOTO 50\n\
30 @(21600)=2\n\
40 E=100\n\
41 GOSUB E\n\
42 @(21601)=X\n\
43 F=190\n\
44 GOSUB F+10\n\
45 @(21602)=Y\n\
50 END\n\
100 X=42\n\
110 RETURN\n\
200 Y=77\n\
210 RETURN\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 30000);

        // 21600 = 0x5460, 21601 = 0x5461, 21602 = 0x5462
        assert_eq!(pc1500.read_byte(0x5460), 2, "GOTO D (D=30) debe saltar a la línea 30, no ejecutar la 25");
        assert_eq!(pc1500.read_byte(0x5461), 42, "GOSUB E (E=100) debe llamar a la línea 100 (X=42) y volver");
        assert_eq!(pc1500.read_byte(0x5462), 77, "GOSUB F+10 (F=190 -> línea 200) debe llamar a la línea 200 (Y=77) y volver");
    }

    /// Un mismo `FOR` con DOS `NEXT` textuales en el código fuente,
    /// alcanzados por caminos de control distintos (vía `GOTO`) según una
    /// condición — el patrón exacto que hacía panic con "NEXT sin FOR
    /// correspondiente" en decathlon.bas (`FOR J=1TO 3:...NEXT J:...GOTO
    /// 290` / `...NEXT J` mucho más abajo, alcanzado por otro camino).
    /// Verifica que AMBOS `NEXT J` hacen avanzar el MISMO bucle
    /// correctamente, no que solo uno "gane".
    #[test]
    fn test_oracle_for_next_multiple_next_statements_same_loop_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 FOR J=1TO 3\n\
20 IF J=2THEN 100\n\
30 A=A+1\n\
40 GOTO 200\n\
100 B=B+1\n\
110 NEXT J\n\
120 GOTO 500\n\
200 NEXT J\n\
500 @(21610)=A\n\
510 @(21611)=B\n\
520 @(21612)=J\n\
530 END\n\
";
        let code = compile_native(source);

        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        // 21610 = 0x546A, 21611 = 0x546B, 21612 = 0x546C
        assert_eq!(pc1500.read_byte(0x546A), 2, "A debe incrementarse en J=1 y J=3 (el NEXT J de la línea 200)");
        assert_eq!(pc1500.read_byte(0x546B), 1, "B debe incrementarse solo en J=2 (el NEXT J de la línea 110)");
        assert_eq!(pc1500.read_byte(0x546C), 4, "J debe terminar en 4 (bucle 1..3 completo, ambos NEXT avanzan el mismo bucle)");
    }

    #[test]
    fn test_basic_generation() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::ApilaInt(42),
            StackInstruction::Label("loop".to_string()),
            StackInstruction::IrA("loop".to_string()),
        ];
        
        let code = backend.generate(&instructions);
        assert!(!code.is_empty());
    }
    
    #[test]
    fn test_initialization() {
        let backend = Lh5801Backend::new();
        assert_eq!(backend.start_address, 0x4100);
        assert_eq!(backend.stack_top, 0x57FF);
    }

    #[test]
    fn test_emit_word_order_is_high_low() {
        let mut backend = Lh5801Backend::new();
        backend.emit_word(0x1234);
        assert_eq!(backend.code, vec![0x12, 0x34]);
    }

    #[test]
    fn test_ira_encodes_jmp_with_high_low_address() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::IrA("target".to_string()),
            StackInstruction::Label("target".to_string()),
        ];

        let code = backend.generate(&instructions);

        // IrA debe emitir 0xBA seguido de la dirección (high, low) de la
        // etiqueta "target", que cae justo después de esta instrucción de
        // 3 bytes. Se busca la posición en vez de asumir un tamaño de
        // prólogo fijo, ya que este puede cambiar.
        let pos = code.iter().position(|&b| b == 0xBA).expect("No se encontró JMP (0xBA)");
        let target = backend.get_start_address() + pos as u16 + 3;
        assert_eq!(code[pos + 1], (target >> 8) as u8);
        assert_eq!(code[pos + 2], (target & 0xFF) as u8);
    }

    /// IrF ahora usa un trampolín (BZR +3 corto e invertido, seguido de un
    /// JMP absoluto de 16 bits) en vez de un branch corto directo al
    /// label — así que el offset del branch es siempre +3, tanto si el
    /// label está delante como detrás; lo que sí varía con la dirección
    /// es la dirección absoluta que codifica el JMP.
    #[test]
    fn test_irf_uses_bzr_trampoline_then_absolute_jmp() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::IrF("end".to_string()),
            StackInstruction::Label("end".to_string()),
        ];

        let code = backend.generate(&instructions);
        let window = code
            .windows(6)
            .find(|w| w[0] == 0xB7 && w[1] == 0x00)
            .expect("No se encontró secuencia CPI #0 del ir-f");

        assert_eq!(window[2], 0x89); // BZR +3 (invertido: si es verdadero, saltar el JMP)
        assert_eq!(window[3], 0x03);
        assert_eq!(window[4], 0xBA); // JMP absoluto
    }

    #[test]
    fn test_irf_trampoline_jmp_resolves_backward_label() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::Label("loop".to_string()),
            StackInstruction::IrF("loop".to_string()),
        ];

        let code = backend.generate(&instructions);
        let window = code
            .windows(6)
            .find(|w| w[0] == 0xB7 && w[1] == 0x00)
            .expect("No se encontró secuencia CPI #0 del ir-f");

        assert_eq!(window[2], 0x89);
        assert_eq!(window[3], 0x03);
        assert_eq!(window[4], 0xBA);

        // El JMP debe apuntar a una dirección real dentro del código
        // generado (la etiqueta "loop", justo tras el prólogo), no al
        // placeholder sin resolver (0x0000).
        let jmp_pos = code.iter().position(|&b| b == 0xBA).expect("No se encontró JMP (0xBA)");
        let target = ((code[jmp_pos + 1] as u16) << 8) | (code[jmp_pos + 2] as u16);
        let start = backend.get_start_address();
        assert!(
            target >= start && target < start + code.len() as u16,
            "el JMP debe apuntar dentro del código generado, no a 0x0000"
        );
    }

    #[test]
    fn test_irv_uses_bzs_trampoline_then_absolute_jmp() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::Label("loop".to_string()),
            StackInstruction::IrV("loop".to_string()),
        ];

        let code = backend.generate(&instructions);
        let window = code
            .windows(6)
            .find(|w| w[0] == 0xB7 && w[1] == 0x00)
            .expect("No se encontró secuencia CPI #0 del ir-v");

        assert_eq!(window[2], 0x8B); // BZS +3 (invertido: si es falso, saltar el JMP)
        assert_eq!(window[3], 0x03);
        assert_eq!(window[4], 0xBA); // JMP absoluto
    }

    /// Reproduce el caso real que hacía panic ("Branch offset too large")
    /// al compilar bathyscaph.bas: un `IF` cuyo cuerpo (THEN) genera más
    /// de 255 bytes de código, muy por encima del rango de un branch
    /// corto directo. Con el trampolín, el salto real es un JMP absoluto
    /// de 16 bits sin límite de distancia.
    #[test]
    fn test_irf_trampoline_supports_targets_beyond_short_branch_range() {
        let mut backend = Lh5801Backend::new();
        let mut instructions = vec![StackInstruction::IrF("far".to_string())];
        // ApilaInt(300) (valor >255) emite 4 bytes; 100 repeticiones dan
        // ~400 bytes, muy por encima de los 255 que un branch corto directo
        // al label podría alcanzar (no hace falta equilibrar la pila: este
        // código nunca se ejecuta, solo se comprueba que generate() no
        // hace panic al resolver el salto).
        for _ in 0..100 {
            instructions.push(StackInstruction::ApilaInt(300));
        }
        instructions.push(StackInstruction::Label("far".to_string()));

        let code = backend.generate(&instructions);
        assert!(!code.is_empty());
    }

    #[test]
    fn test_divint_loop_exit_uses_bcr_after_subtract_compare() {
        let code = generated_code_for(StackInstruction::DivInt);
        // SEC; LDA UH; SBC UL; BCR +6
        assert!(has_subsequence(&code, &[0xFB, 0xA4, 0x20, 0x81, 0x06]));
    }

    #[test]
    fn test_mayorint_translation_matches_expected_flow() {
        let code = generated_code_for(StackInstruction::MayorInt);
        // SEC; SBC UL; BCR +8 (a<b => false); CPI #0; BZS +4 (a==b => false)
        assert!(has_subsequence(&code, &[0xFB, 0x20, 0x81, 0x08, 0xB7, 0x00, 0x8B, 0x04]));
    }

    #[test]
    fn test_menorint_translation_matches_expected_flow() {
        let code = generated_code_for(StackInstruction::MenorInt);
        // SEC; SBC UL; BCS +4 (a>=b => false path)
        assert!(has_subsequence(&code, &[0xFB, 0x20, 0x83, 0x04]));
    }

    #[test]
    fn test_menorigual_translation_matches_expected_flow() {
        let code = generated_code_for(StackInstruction::MenorIgualInt);
        // SEC; SBC UL; BCR +8 (a<b => true); CPI #0; BZS +4 (a==b => true)
        assert!(has_subsequence(&code, &[0xFB, 0x20, 0x81, 0x08, 0xB7, 0x00, 0x8B, 0x04]));
    }

    /// SBC en este backend calcula `A + ~operando + Carry` (ver `sbc()` en
    /// `ceres-core`), así que una resta simple `a-b` sin borrow extra
    /// necesita Carry=1 (SEC) antes de SBC, no Carry=0 (REC) — usar REC
    /// aquí computaba `a-b-1`, un bug confirmado ejecutando contra la ROM
    /// real (ver `test_oracle_for_next_descending_step_on_real_rom`).
    #[test]
    fn test_all_integer_comparisons_set_carry_before_sbc() {
        for instr in [
            StackInstruction::MayorInt,
            StackInstruction::MenorInt,
            StackInstruction::IgualInt,
            StackInstruction::DistintoInt,
            StackInstruction::MayorIgualInt,
            StackInstruction::MenorIgualInt,
        ] {
            let code = generated_code_for(instr);
            assert!(has_subsequence(&code, &[0xFB, 0x20]));
        }
    }

    #[test]
    fn test_call_encodes_sjp_with_absolute_label_address() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::Call("sub".to_string()),
            StackInstruction::Label("sub".to_string()),
        ];

        let code = backend.generate(&instructions);

        // CALL debe emitir 0xBE seguido de la dirección (high, low) de la
        // etiqueta "sub", justo después de esta instrucción de 3 bytes. Se
        // busca la posición en vez de asumir un tamaño de prólogo fijo.
        let pos = code.iter().position(|&b| b == 0xBE).expect("No se encontró SJP (0xBE)");
        let target = backend.get_start_address() + pos as u16 + 3;
        assert_eq!(code[pos + 1], (target >> 8) as u8);
        assert_eq!(code[pos + 2], (target & 0xFF) as u8);
    }

    #[test]
    fn test_apila_cadena_appends_literal_data_section() {
        let mut backend = Lh5801Backend::new();
        let code = backend.generate(&[StackInstruction::ApilaCadena("HI".to_string())]);

        // Sección de datos anexada al final: "HI\0"
        assert!(code.ends_with(&[b'H', b'I', 0x00]));
        // Debe haber RTN antes de la sección de datos
        assert!(has_subsequence(&code, &[0x9A, b'H', b'I', 0x00]));
    }

    /// `CHR$`/`ABS` sobre el corpus de "lagunas a cubrir": `CHR$(65)` debe
    /// producir la cadena de 1 carácter "A" (verificado con `ASC` sobre el
    /// resultado, mismo patrón que el resto de tests de funciones de
    /// cadena) y `ABS` debe funcionar sobre el caso real del corpus
    /// (resta entre dos enteros, nunca un real — ver comentario de
    /// `FunctionInner::Abs` en `mod.rs`), cubriendo negativo/positivo/cero.
    #[test]
    fn test_oracle_chr_and_abs_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "\
10 @(21700)=ASC CHR$ (65)\n\
20 A=3:B=8\n\
30 @(21701)=ABS (A-B)\n\
40 @(21702)=ABS (B-A)\n\
50 @(21703)=ABS (A-A)\n\
60 END\n\
";
        let code = compile_native(source);
        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        assert_eq!(pc1500.read_byte(0x54C4), b'A', "CHR$(65) = \"A\"");
        assert_eq!(pc1500.read_byte(0x54C5), 5, "ABS(3-8) = 5");
        assert_eq!(pc1500.read_byte(0x54C6), 5, "ABS(8-3) = 5");
        assert_eq!(pc1500.read_byte(0x54C7), 0, "ABS(3-3) = 0");
    }

    /// `RANDOM` debe perturbar de verdad el estado del LFSR mock
    /// compartido con `RND()` (ver comentario de `StackInstruction::Random`
    /// en el backend) — no basta con que deje de hacer panic: dos
    /// programas idénticos salvo por un `RANDOM` de más antes del primer
    /// `RND()` deben producir resultados distintos, porque `RANDOM` avanza
    /// el LFSR un paso extra.
    #[test]
    fn test_oracle_random_perturbs_shared_rnd_seed_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let without_random = compile_native("10 @(21700)=RND (200)\n20 END\n");
        let with_random = compile_native("10 RANDOM\n20 @(21700)=RND (200)\n30 END\n");

        let pc_without = run_lh5_until_exit(ORACLE_LOAD_ADDR, &without_random, ORACLE_STACK_TOP, 20000);
        let pc_with = run_lh5_until_exit(ORACLE_LOAD_ADDR, &with_random, ORACLE_STACK_TOP, 20000);

        let value_without = pc_without.read_byte(0x54C4);
        let value_with = pc_with.read_byte(0x54C4);
        assert_ne!(
            value_without, value_with,
            "RANDOM debería cambiar el resultado del siguiente RND() (sin: {value_without}, con: {value_with})"
        );
    }

    /// `PAUSE`: debe imprimir su argumento (como `PRINT`, con salto de
    /// línea automático) y luego iniciar una pausa real — verificado
    /// igual que `WAIT` (`test_oracle_wait_...`): tras un número modesto
    /// de pasos el PC debe seguir dentro de `TIME_DELAY` o de la
    /// subrutina auxiliar de sondeo que llama internamente (ver el
    /// comentario de ese mismo rango en el test de `WAIT`).
    #[test]
    fn test_oracle_pause_prints_then_delays_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5, ORACLE_LOAD_ADDR};

        let code = compile_native("10 PAUSE \"HI\"\n20 END\n");
        let pc1500 = run_lh5(ORACLE_LOAD_ADDR, &code, 2000);

        assert_eq!(pc1500.read_byte(0x7875), (2 + 1) * 6, "CURSOR_PTR tras PAUSE \"HI\" (\"HI\" + salto de línea)");
        let pc = pc1500.cpu().p();
        assert!(
            (0xE88C..=0xE8C0).contains(&pc) || (0xE451..=0xE456).contains(&pc),
            "tras 2000 pasos el PC debería seguir dentro de TIME_DELAY o su subrutina auxiliar (sondeando), no en {pc:#06X}"
        );
    }

    /// `ON ERROR GOTO`: antes de esta corrección, el target evaluado en
    /// tiempo de ejecución se descartaba sin usar (fuga de pila) y se
    /// emitía una etiqueta fija ("ERROR_HANDLER") que nunca se definía en
    /// ningún sitio — `resolve_labels` hacía panic con "Undefined label"
    /// en cuanto el programa la usaba (el caso real: `invader-v2.bas`,
    /// `pacman.bas`). Verifica que compila sin panic Y que el programa
    /// sigue funcionando con normalidad tras la declaración (la línea 20
    /// se ejecuta y dos, no salta a la 999 porque no hay detección de
    /// errores real todavía, solo el registro mínimo del handler).
    #[test]
    fn test_oracle_on_error_goto_compiles_and_continues_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5_until_exit, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 ON ERROR GOTO 999\n20 @(21700)=42\n30 END\n999 @(21700)=99\n1000 END\n";
        let code = compile_native(source);
        let pc1500 = run_lh5_until_exit(ORACLE_LOAD_ADDR, &code, ORACLE_STACK_TOP, 20000);

        assert_eq!(pc1500.read_byte(0x54C4), 42, "sin detección de errores real, el flujo normal (línea 20) debe ejecutarse, no el handler (línea 999)");
    }

    /// `INPUT`: simula pulsaciones reales de teclado (`Pc1500::press`/
    /// `release`, la misma API que usaría la GUI) para un destino
    /// numérico y uno de cadena en el mismo programa — cubre el bucle de
    /// sondeo ISKEY/KEY_2_ASCII, el eco por CHAR_OUT, esperar a que la
    /// tecla se suelte antes de continuar, el terminador CR, y las dos
    /// rutas de almacenamiento (`CallVal` para numérico, puntero directo
    /// para cadena) de `gen_input`.
    ///
    /// Notas de infraestructura para cualquier test futuro que necesite
    /// simular teclado:
    /// - `step_cpu()` a solas NO basta: el "strobe" de columna del
    ///   teclado (`ks`) se deriva del registro DDA del LH5810 dentro de
    ///   `Pc1500::run()` (privado), que solo se invoca desde
    ///   `step_frame()`; con solo `step_cpu()` `ks` se queda congelado en
    ///   0 para siempre y ninguna tecla se detecta nunca (la GUI real,
    ///   `ceres-egui`, también usa `step_frame()` para esto).
    /// - El programa de prueba NUNCA debe terminar en un `END`/`RTN`
    ///   alcanzable dentro del margen de los `step_frame()` usados para
    ///   sondear teclado: cada `step_frame()` son 15000 ticks (una
    ///   "rebanada" gruesa e ininterrumpible), así que un programa corto
    ///   puede terminar de sobra DENTRO de una sola llamada — y ejecutar
    ///   ese `RTN` sin llamador real dispara un opcode ilegal (mismo caso
    ///   que documenta `run_lh5_until_exit`). Perseguir esto por la vía
    ///   equivocada (sospechando primero de un desajuste de pila en
    ///   `SystemIn`, después de una interrupción de temporizador interna
    ///   de la CPU — se llegó a instrumentar el emulador directamente
    ///   para confirmar que ninguna de las dos ocurre) costó mucho tiempo
    ///   de depuración; la causa real solo se confirmó al ver que un
    ///   programa CON un bucle infinito (`A=A+1:GOTO 10`, sin `INPUT`)
    ///   corría decenas de `step_frame()` sin problema, mientras que
    ///   cualquier programa corto con un final alcanzable fallaba
    ///   incluso con muy pocos. La forma robusta de evitarlo, usada aquí,
    ///   es terminar el programa de prueba con un bucle infinito sobre sí
    ///   mismo (`GOTO <su propia línea>`) en vez de `END` — así no hay
    ///   ningún `RTN` sin llamador que temer y sobra hacer stepping fino
    ///   con guardas de salida.
    #[test]
    fn test_oracle_input_numeric_and_string_via_simulated_keypresses_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, load, ORACLE_LOAD_ADDR};
        use ceres_core::Key;

        let source = "10 INPUT A\n20 INPUT B$\n30 @(21700)=A\n40 @(21701)=ASC B$\n50 GOTO 50\n";
        let code = compile_native(source);
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);

        let type_key_and_release = |pc1500: &mut ceres_core::Pc1500, key: Key| {
            pc1500.press(key);
            for _ in 0..3 {
                pc1500.step_frame();
            }
            pc1500.release(key);
            for _ in 0..3 {
                pc1500.step_frame();
            }
        };

        // INPUT A: teclear "5" y ENTER -> A debe quedar en 5.
        type_key_and_release(&mut pc1500, Key::Five);
        type_key_and_release(&mut pc1500, Key::Enter);

        // INPUT B$: teclear "O" y ENTER -> B$ debe quedar en "O".
        type_key_and_release(&mut pc1500, Key::O);
        type_key_and_release(&mut pc1500, Key::Enter);

        // Dejar correr las asignaciones finales (línea 50 es un bucle
        // infinito sobre sí misma, a propósito — ver nota de arriba).
        pc1500.step_frame();

        assert_eq!(pc1500.read_byte(0x54C4), 5, "INPUT A tras teclear \"5\"+ENTER");
        assert_eq!(pc1500.read_byte(0x54C5), b'O', "INPUT B$ tras teclear \"O\"+ENTER");
    }

    /// `INKEY$`: a diferencia de `INPUT`, es un sondeo NO bloqueante y
    /// SIN eco — el patrón real de un bucle de juego (`bathyscaph.bas`:
    /// `Z$=INKEY$:IF Z$=""THEN...`). Verifica, con el mismo bucle
    /// sondeando `INKEY$` en cada vuelta: (1) sin ninguna tecla pulsada,
    /// `LEN(INKEY$)=0` (cadena vacía); (2) con una tecla pulsada,
    /// `ASC(INKEY$)` es su código ASCII real; y (3) `CURSOR_PTR` nunca se
    /// mueve — confirma que, a diferencia de `SystemIn`, no hay ningún
    /// eco por `CHAR_OUT`.
    #[test]
    fn test_oracle_inkey_polls_without_blocking_or_echo_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, load, ORACLE_LOAD_ADDR};
        use ceres_core::Key;

        let source = "10 @(21700)=LEN (INKEY$)\n20 GOTO 10\n";
        let code = compile_native(source);
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);

        // Sin tecla pulsada: varias vueltas del bucle deben dar longitud 0.
        for _ in 0..3 {
            pc1500.step_frame();
        }
        assert_eq!(pc1500.read_byte(0x54C4), 0, "INKEY$ sin tecla pulsada debe ser \"\" (longitud 0)");
        assert_eq!(pc1500.read_byte(0x7875), 0, "CURSOR_PTR no debe moverse: INKEY$ no hace eco");

        // Con una tecla mantenida pulsada: longitud 1, y sigue sin eco.
        pc1500.press(Key::Five);
        for _ in 0..3 {
            pc1500.step_frame();
        }
        assert_eq!(pc1500.read_byte(0x54C4), 1, "INKEY$ con una tecla pulsada debe tener longitud 1");
        assert_eq!(pc1500.read_byte(0x7875), 0, "CURSOR_PTR sigue sin moverse con una tecla pulsada: INKEY$ no hace eco");
        pc1500.release(Key::Five);
    }

    /// Mismo caso que el test anterior pero comprobando el código ASCII
    /// real vía `ASC(INKEY$)`, no solo la longitud.
    #[test]
    fn test_oracle_inkey_returns_correct_ascii_code_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, load, ORACLE_LOAD_ADDR};
        use ceres_core::Key;

        let source = "10 @(21700)=ASC (INKEY$)\n20 GOTO 10\n";
        let code = compile_native(source);
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);

        pc1500.press(Key::Five);
        for _ in 0..3 {
            pc1500.step_frame();
        }
        assert_eq!(pc1500.read_byte(0x54C4), b'5', "ASC(INKEY$) con \"5\" pulsada debe ser el código ASCII de '5'");
        pc1500.release(Key::Five);
    }

    /// Prueba end-to-end sobre `bathyscaph.bas` **sin modificar**, el
    /// programa real de referencia de todo este proyecto — no un
    /// fragmento sintético. Cubre, en un solo pipeline real (fuente →
    /// lexer → parser → IR → backend → ejecución en la ROM real):
    ///
    /// - Fase de dibujo de la cueva (líneas 8-40): `CLS`, `WAIT 0`,
    ///   `CLEAR`, `DIM A$(0)*20`, un bucle `FOR` de 14 vueltas con
    ///   `RESTORE 999+RND 16` (expresión dinámica, no literal),
    ///   `READ A$(0)` y `GPRINT A$(0);` — confirmado por que el buffer
    ///   de pantalla real queda tocado en decenas de bytes.
    /// - Inicialización (línea 41): `TIME =0,H=3,G=8` — confirmado leyendo
    ///   `H` en su dirección real.
    /// - El bucle de juego (línea 50 en adelante) alcanza `P=2` y arranca.
    /// - **Control real del jugador vía `INKEY$`** (líneas 60-80): con
    ///   una tecla de verdad simulada (`Pc1500::press`), `H` cambia en la
    ///   dirección correcta en cada vuelta que la tecla sigue pulsada —
    ///   `Up` (código ASCII real 11) la baja, `Down` (código real 10) la
    ///   sube, exactamente como `H=H-SGN(ASC Z$-10.5)` dicta. Esto es lo
    ///   que de verdad hace jugable el programa — antes de implementar
    ///   `INKEY$` hoy, `H` nunca habría cambiado.
    ///
    /// Las comprobaciones se hacen mientras `P<10` (columnas 2-9): la
    /// cueva se dibuja desde `GCURSOR 10` en adelante (línea 17), así que
    /// esas columnas están garantizado en blanco y no puede haber una
    /// colisión real que resetee el estado (`GOSUB "CRASH"`) — evita que
    /// el timing exacto de la prueba dependa de la disposición de la
    /// cueva (determinista por nuestro LFSR mock, pero frágil de fijar a
    /// mano). El tramo de frames usado se calibró empíricamente contra
    /// este mismo oráculo: a los 13 `step_frame()` la fase de dibujo ya
    /// ha terminado (`H` todavía en su valor sin inicializar, 0) pero el
    /// bucle de juego aún no ha arrancado; 1-2 frames más bastan para
    /// verlo en marcha con `H` respondiendo ya a la tecla.
    #[test]
    fn test_oracle_bathyscaph_end_to_end_gameplay_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native_with_addresses, load, ORACLE_LOAD_ADDR};
        use ceres_core::Key;

        let source = std::fs::read_to_string("test/basic/bathyscaph.bas")
            .expect("no se pudo leer test/basic/bathyscaph.bas");
        let (code, addrs) = compile_native_with_addresses(&source);
        let h_addr = *addrs.get("H").expect("dirección de H no encontrada") as u32;
        let p_addr = *addrs.get("P").expect("dirección de P no encontrada") as u32;

        // --- Fase de dibujo de la cueva, sin ninguna tecla pulsada ---
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);
        for _ in 0..11 {
            pc1500.step_frame();
        }

        let touched_bytes = (0x7600..0x7600 + 160).filter(|&a| pc1500.read_byte(a) != 0).count();
        assert!(
            touched_bytes > 100,
            "la cueva (14 segmentos DATA vía RESTORE+READ+GPRINT) debe haber tocado bien más de 100 bytes del buffer de pantalla, tocó {touched_bytes}"
        );

        // Un frame más: el bucle de juego arranca (P=2) y H se inicializa a 3.
        pc1500.step_frame();
        assert_eq!(pc1500.read_byte(p_addr), 2, "el bucle de juego debe haber arrancado en P=2");
        assert_eq!(pc1500.read_byte(h_addr), 3, "H debe estar inicializada a 3 (línea 41: TIME=0,H=3,G=8)");

        // --- Control real del jugador: Up debe BAJAR H ---
        pc1500.press(Key::Up);
        pc1500.step_frame();
        pc1500.step_frame();
        assert!(pc1500.read_byte(p_addr) < 10, "todavía dentro de la zona segura sin cueva dibujada (P<10)");
        assert!(
            pc1500.read_byte(h_addr) < 3,
            "con Up pulsada, H debe haber bajado de su valor inicial (3), quedó en {}",
            pc1500.read_byte(h_addr)
        );
        pc1500.release(Key::Up);

        // --- Control real del jugador: Down debe SUBIR H (partida limpia) ---
        let mut pc1500 = load(ORACLE_LOAD_ADDR, &code);
        for _ in 0..12 {
            pc1500.step_frame();
        }
        pc1500.press(Key::Down);
        pc1500.step_frame();
        pc1500.step_frame();
        assert!(pc1500.read_byte(p_addr) < 10, "todavía dentro de la zona segura sin cueva dibujada (P<10)");
        assert!(
            pc1500.read_byte(h_addr) > 3,
            "con Down pulsada, H debe haber subido de su valor inicial (3), quedó en {}",
            pc1500.read_byte(h_addr)
        );
        pc1500.release(Key::Down);
    }

    /// `RND(n)` con `n>255` (no cabe en un byte, p.ej. `RND 256`) dejaba
    /// 1 byte suelto en la pila en cada llamada: `n` se apilaba con
    /// `ApilaInt`, que elige automáticamente 16 bits para valores >255,
    /// pero `CallRnd` solo hacía `pop` de 8 bits. Invisible en una
    /// llamada aislada (el resto del programa sigue funcionando con un
    /// byte de más en la pila); reproducido de verdad jugando
    /// `bathyscaph.bas` en la GUI real, no en ningún test aislado: su
    /// subrutina `"CRASH"` (línea 32, `FOR I=0TO 30:POKE# 64000,RND
    /// 256-1:NEXT I`) llama a `RND 256` 31 veces seguidas, y para
    /// entonces la pila ya estaba tan desincronizada que el propio
    /// `POKE#` leía basura como dirección (opcode ilegal en la ROM real).
    /// Este test reproduce exactamente ese patrón — el mismo número de
    /// vueltas, la misma expresión — y comprueba que la pila hardware
    /// (`S`) vuelve a `stack_top` tras el bucle, sin ningún resto.
    #[test]
    fn test_oracle_rnd_over_255_does_not_leak_stack_on_real_rom() {
        use crate::codegen::test_oracle::{compile_native, run_lh5, ORACLE_LOAD_ADDR, ORACLE_STACK_TOP};

        let source = "10 FOR I=0TO 30:POKE# 64000,RND 256-1:NEXT I\n20 GOTO 20\n";
        let code = compile_native(source);
        let pc1500 = run_lh5(ORACLE_LOAD_ADDR, &code, 3000);

        assert_eq!(
            pc1500.cpu().s(), ORACLE_STACK_TOP,
            "la pila hardware debe volver a stack_top tras las 31 llamadas a RND(256): S={:#06X}",
            pc1500.cpu().s()
        );
    }

}
