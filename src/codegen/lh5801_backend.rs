/// Backend que traduce instrucciones de la Máquina P a código máquina LH5801
/// 
/// El LH5801 es el procesador de 8 bits del Sharp PC-1500
/// Registros: A (acumulador), X, Y, U (índices de 16 bits), S (stack HW), P (program counter)
/// 
/// Estrategia de pila software:
/// - Usa STANDARD_USER_MEMORY del PC-1500 (0x3800-0x5FFF)
/// - Mapa de memoria:
///   * 0x0000-0x37FF: ROM y sistema
///   * 0x3800-0x57FF: Código de usuario (8KB)
///   * 0x4000+: Área de datos de usuario (variables)
///   * 0x5800-0x5FEF: Stack (752 bytes)
///   * 0x5FF0-0x5FF1: Stack pointer virtual (2 bytes, little-endian)
/// - Valores de 8 bits se almacenan directamente
/// - Direcciones de 16 bits en instrucciones usan little-endian (low byte primero)
/// - X se usa internamente por emit_push_a/emit_pop_a para manejar el stack pointer
/// - Y se usa para direcciones de variables (ApilaInd/DesapilaInd)
/// - U se usa como registro temporal (UL para valores intermedios)

use crate::codegen::stack_instruction::StackInstruction;
use crate::codegen::rom_routines::RomRoutines;
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
    
    /// Dirección de inicio del código (0x3800 = inicio de STANDARD_USER_MEMORY)
    start_address: u16,
    
    /// Dirección donde se almacena el stack pointer virtual (2 bytes, 0x5FF0)
    sp_address: u16,
    
    /// Dirección de inicio del área de stack (0x5800)
    stack_base: u16,
    
    /// Dirección final del área de stack (0x5FEF = top antes del SP)
    stack_top: u16,

    /// Referencias a literales de cadena que se resuelven al final
    /// (literal, posición del byte alto, posición del byte bajo)
    string_refs: Vec<(String, usize, usize)>,
}

/// Tipo de referencia a etiqueta
#[derive(Debug, Clone, Copy)]
enum RefType {
    /// Salto absoluto de 16 bits (JMP, CALL)
    Absolute16,
    /// Branch por Z=1 (BZS), el signo se resuelve en segunda pasada (+/-)
    BranchBzs,
    /// Branch por Z=0 (BZR), el signo se resuelve en segunda pasada (+/-)
    BranchBzr,
}

impl Lh5801Backend {
    /// Crear nuevo backend con configuración por defecto
    pub fn new() -> Self {
        Lh5801Backend {
            code: Vec::new(),
            labels: HashMap::new(),
            label_refs: Vec::new(),
            rom_routines: RomRoutines::new(),
            start_address: 0x3800,  // 0x3800 = Inicio de STANDARD_USER_MEMORY
            sp_address: 0x5FF0,     // 0x5FF0 = Dirección del SP virtual (últimos 2 bytes)
            stack_base: 0x5800,     // 0x5800 = Base del stack (8KB después del código)
            stack_top: 0x5FEF,      // 0x5FEF = Top del stack (antes del SP)
            string_refs: Vec::new(),
        }
    }
    
    /// Crear backend con configuración personalizada
    pub fn with_config(start_address: u16, sp_address: u16, stack_base: u16, stack_top: u16) -> Self {
        Lh5801Backend {
            code: Vec::new(),
            labels: HashMap::new(),
            label_refs: Vec::new(),
            rom_routines: RomRoutines::new(),
            start_address,
            sp_address,
            stack_base,
            stack_top,
            string_refs: Vec::new(),
        }
    }
    
    /// Generar código LH5801 a partir de instrucciones de pila
    pub fn generate(&mut self, instructions: &[StackInstruction]) -> Vec<u8> {
        // Prólogo: inicializar stack pointer
        self.emit_initialization();
        
        // Primera pasada: generar código y marcar etiquetas
        for (idx, instr) in instructions.iter().enumerate() {
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
    fn emit_initialization(&mut self) {
        // Inicializar stack pointer virtual en memoria
        // SP_address = stack_base (stack vacío apunta a la base)
        
        // Guardar high byte del stack base
        self.emit_byte(0xB5); // LDA #imm
        self.emit_byte((self.stack_base >> 8) as u8);
        self.emit_byte(0xAE); // STA addr
        self.emit_word(self.sp_address);
        
        // Guardar low byte del stack base
        self.emit_byte(0xB5); // LDA #imm
        self.emit_byte((self.stack_base & 0xFF) as u8);
        self.emit_byte(0xAE); // STA addr
        self.emit_word(self.sp_address + 1);
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
                match ref_type {
                    RefType::Absolute16 => {
                        // Escribir dirección absoluta de 16 bits (big-endian)
                        let addr = target_addr as u16;
                        self.code[*pos] = (addr >> 8) as u8;
                        self.code[*pos + 1] = (addr & 0xFF) as u8;
                    }
                    RefType::BranchBzs | RefType::BranchBzr => {
                        // En LH5801 el sentido (+/-) va codificado en el opcode.
                        // El operando i es la magnitud del salto.
                        let next_instr = *pos + self.start_address as usize + 1;
                        let delta = (target_addr as i32) - (next_instr as i32);
                        let magnitude = delta.unsigned_abs();

                        if magnitude > 255 {
                            panic!("Branch offset too large for label {}: {}", label_name, delta);
                        }

                        // Posición del opcode del branch (byte anterior al operando)
                        let opcode_pos = *pos - 1;
                        match ref_type {
                            RefType::BranchBzs => {
                                self.code[opcode_pos] = if delta < 0 { 0x9B } else { 0x8B };
                            }
                            RefType::BranchBzr => {
                                self.code[opcode_pos] = if delta < 0 { 0x99 } else { 0x89 };
                            }
                            _ => unreachable!(),
                        }

                        self.code[*pos] = magnitude as u8;
                    }
                }
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
            RefType::BranchBzs | RefType::BranchBzr => {
                self.emit_byte(0x00); // Placeholder de 1 byte
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
    
    /// PUSH A al stack virtual
    /// Entrada: A contiene el valor a pushear
    /// Salida: valor guardado en stack, SP incrementado, A preservado
    fn emit_push_a(&mut self) {
        // 1. Guardar A temporalmente en UL
        self.emit_byte(0x2A); // UL = A
        
        // 2. Cargar SP de memoria a X
        //    XH = [sp_address]
        self.emit_byte(0xA5); // LDA addr
        self.emit_word(self.sp_address);
        self.emit_byte(0x08); // XH = A
        
        //    XL = [sp_address+1]
        self.emit_byte(0xA5); // LDA addr
        self.emit_word(self.sp_address + 1);
        self.emit_byte(0x0A); // XL = A
        
        // 3. Recuperar valor de UL a A
        self.emit_byte(0x24); // LDA UL
        
        // 4. Almacenar A en [X]
        self.emit_byte(0x0E); // STA (X)
        
        // 5. Incrementar X (SP++)
        self.emit_byte(0x44); // X++
        
        // 6. Guardar X de vuelta en memoria como SP
        //    Guardar A primero (lo necesitamos preservado)
        self.emit_byte(0x18); // YH = A (preservar A)
        
        //    [sp_address] = XH
        self.emit_byte(0x84); // LDA XH
        self.emit_byte(0xAE); // STA addr
        self.emit_word(self.sp_address);
        
        //    [sp_address+1] = XL
        self.emit_byte(0x04); // LDA XL
        self.emit_byte(0xAE); // STA addr
        self.emit_word(self.sp_address + 1);
        
        // 7. Restaurar A
        self.emit_byte(0x94); // LDA YH
    }
    
    /// POP del stack virtual a A
    /// Entrada: ninguna
    /// Salida: A contiene el valor popeado, SP decrementado
    fn emit_pop_a(&mut self) {
        // 1. Cargar SP de memoria a X
        //    XH = [sp_address]
        self.emit_byte(0xA5); // LDA addr
        self.emit_word(self.sp_address);
        self.emit_byte(0x08); // XH = A
        
        //    XL = [sp_address+1]
        self.emit_byte(0xA5); // LDA addr
        self.emit_word(self.sp_address + 1);
        self.emit_byte(0x0A); // XL = A
        
        // 2. Decrementar X (SP--)
        self.emit_byte(0x46); // X--
        
        // 3. Leer valor de [X] a A
        self.emit_byte(0x05); // LDA (X)
        
        // 4. Guardar A temporalmente en YH
        self.emit_byte(0x18); // YH = A
        
        // 5. Guardar X decrementado de vuelta en SP
        //    [sp_address] = XH
        self.emit_byte(0x84); // LDA XH
        self.emit_byte(0xAE); // STA addr
        self.emit_word(self.sp_address);
        
        //    [sp_address+1] = XL
        self.emit_byte(0x04); // LDA XL
        self.emit_byte(0xAE); // STA addr
        self.emit_word(self.sp_address + 1);
        
        // 6. Recuperar valor original a A
        self.emit_byte(0x94); // LDA YH
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
            
            StackInstruction::ApilaReal(_r) => {
                // Representación temporal: convertir a entero y apilar como ApilaInt.
                // Cuando se implemente aritmética real nativa, este punto se sustituirá
                // por codificación de punto fijo/BCD compatible con las rutinas ROM.
                let n = _r.round() as i64;
                if n >= 0 && n <= 255 {
                    self.emit_byte(0xB5); // LDI A,#imm
                    self.emit_byte(n as u8);
                    self.emit_push_a();
                } else {
                    let val = n as u16;
                    // Push high byte
                    self.emit_byte(0xB5); // LDI A,#imm
                    self.emit_byte((val >> 8) as u8);
                    self.emit_push_a();
                    // Push low byte
                    self.emit_byte(0xB5); // LDI A,#imm
                    self.emit_byte((val & 0xFF) as u8);
                    self.emit_push_a();
                }
            }
            
            StackInstruction::ApilaCadena(s) => {
                // Apilar puntero a literal (16 bits: high, low).
                // El literal se almacena al final del binario en resolve_string_literals().

                // Byte alto del puntero (placeholder)
                self.emit_byte(0xB5); // LDI A,#imm
                let high_pos = self.code.len();
                self.emit_byte(0x00);
                self.emit_push_a();

                // Byte bajo del puntero (placeholder)
                self.emit_byte(0xB5); // LDI A,#imm
                let low_pos = self.code.len();
                self.emit_byte(0x00);
                self.emit_push_a();

                self.string_refs.push((s.clone(), high_pos, low_pos));
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
                self.emit_byte(0xF9); // REC (Limpiar Carry para A - B - 0)
                self.emit_byte(0x20); // SBC UL
                
                // 4. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::MulInt => {
                // Multiplicación usando bucle simple
                // Pop b, Pop a, Push (a * b)
                // Algoritmo: resultado = 0; while(b > 0) { resultado += a; b--; }
                
                // 1. Pop b a UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b en UL)
                
                // 2. Pop a a UH
                self.emit_pop_a();
                self.emit_byte(0x28); // UH = A (a en UH)
                // Nota: STA UH es 0x28, STA UL es 0x2A
                
                // 3. Inicializar resultado (A) a 0
                self.emit_byte(0xB5); // LDI A, #0
                self.emit_byte(0x00);
                
                // 4. Bucle de multiplicación (solo si b > 0)
                // Si b == 0, saltar al final
                self.emit_byte(0x24); // LDA UL
                self.emit_byte(0xB7); // CPI A, #0
                self.emit_byte(0x00);
                // Si Z=1 (b==0), saltar 9 bytes adelante (fin del loop)
                // Cálculo: REC(1)+ADC(1)+LDA(1)+DEC(1)+STA(1)+CPI(2)+BCH(2) = 9
                self.emit_byte(0x8B); // BZS +offset (Branch Zero Set)
                self.emit_byte(0x09);
                
                // Loop: A += UH; UL--; if (UL != 0) goto Loop
                // A += UH -> REC + ADC UH
                self.emit_byte(0xF9); // REC
                self.emit_byte(0xA2); // ADC UH (0xA2)
                
                // UL--
                self.emit_byte(0x24); // LDA UL (0x24)
                self.emit_byte(0xDF); // DEC A (0xDF)
                self.emit_byte(0x2A); // STA UL (0x2A)
                
                // if (A != 0) goto Start
                self.emit_byte(0xB7); // CPI A, #0
                self.emit_byte(0x00);
                
                // BZR negativo (Jump if Not Zero / Z=0 Backwards)
                // Opcode 0x99 es BZR -offset (100 1 1001)
                self.emit_byte(0x99); 
                // Salto atrás: 7 bytes (CPI, STA, DEC, LDA, ADC, REC) + 2 (BZR) = 9 bytes
                self.emit_byte(0x09); // Offset positivo magnitud 9
                
                // 5. Push resultado
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
                // Comparar UH con UL (UH - UL)
                self.emit_byte(0xF9); // REC
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
                // Salto si falso (Z=1)
                // Pop condición, comparar con 0, saltar si igual
                self.emit_pop_a();
                self.emit_byte(0xB7); // CPA #imm
                self.emit_byte(0x00);
                
                // BZ (branch if zero) - salto relativo
                self.emit_byte(0x8B); // BZS +i (se ajusta a +/- en resolve_labels)
                self.add_label_ref(label.clone(), RefType::BranchBzs);
                self.emit_label_placeholder(RefType::BranchBzs);
            }
            
            StackInstruction::IrV(label) => {
                // Salto si verdadero (Z=0)
                self.emit_pop_a();
                self.emit_byte(0xB7); // CPA #imm
                self.emit_byte(0x00);
                
                // BNZ (branch if not zero)
                self.emit_byte(0x89); // BZR +i (se ajusta a +/- en resolve_labels)
                self.add_label_ref(label.clone(), RefType::BranchBzr);
                self.emit_label_placeholder(RefType::BranchBzr);
            }
            
            StackInstruction::Call(label) => {
                // Llamada a subrutina absoluta.
                // SJP addr guarda retorno en stack HW y transfiere control a la etiqueta.
                self.emit_byte(0xBE); // SJP
                self.add_label_ref(label.clone(), RefType::Absolute16);
                self.emit_label_placeholder(RefType::Absolute16);
            }
            
            // ===== ENTRADA/SALIDA =====
            
            StackInstruction::SystemOut => {
                // Imprimir valor del tope de la pila usando rutina ROM
                // Pop valor a A
                self.emit_pop_a();
                
                // Llamar a rutina ROM PRINT_CHAR para imprimir
                // La rutina espera el carácter en A
                if let Some(addr) = self.rom_routines.address("PRINT_CHAR") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM PRINT_CHAR no encontrada");
                }
            }
            
            StackInstruction::Newline => {
                // Imprimir nueva línea usando rutina ROM
                if let Some(addr) = self.rom_routines.address("PRINT_NEWLINE") {
                    self.emit_call_rom(addr);
                } else {
                    eprintln!("WARNING: Rutina ROM PRINT_NEWLINE no encontrada");
                }
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
                
                // 3. Comparar: A - UL
                self.emit_byte(0xF9); // REC
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
                
                // 3. Comparar: A - UL
                self.emit_byte(0xF9); // REC
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
                
                // 3. Comparar: A - UL
                self.emit_byte(0xF9); // REC
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
                
                // 3. Comparar: A - UL
                self.emit_byte(0xF9); // REC
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
            
            StackInstruction::MayorIgualInt => {
                // Pop b, Pop a, Push (a >= b ? 1 : 0)
                // Implementación: a >= b ⟺ a - b >= 0
                
                // 1. Pop b
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (b)
                
                // 2. Pop a
                self.emit_pop_a();
                
                // 3. Comparar: A - UL
                self.emit_byte(0xF9); // REC
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
                
                // 3. Comparar: A - UL
                self.emit_byte(0xF9); // REC
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
        assert_eq!(backend.start_address, 0x3800);
        assert_eq!(backend.sp_address, 0x5FF0);
        assert_eq!(backend.stack_base, 0x5800);
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

        // Tras inicialización (10 bytes), IrA debe emitir BA y dirección 0x380D
        assert_eq!(code[10], 0xBA);
        assert_eq!(code[11], 0x38);
        assert_eq!(code[12], 0x0D);
    }

    #[test]
    fn test_irf_forward_uses_bzs_plus() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::IrF("end".to_string()),
            StackInstruction::Label("end".to_string()),
        ];

        let code = backend.generate(&instructions);
        let branch_window = code
            .windows(4)
            .find(|w| w[0] == 0xB7 && w[1] == 0x00)
            .expect("No se encontró secuencia CPI #0 del ir-f");

        assert_eq!(branch_window[2], 0x8B); // BZS +i
        assert_eq!(branch_window[3], 0x00); // salto a instrucción siguiente
    }

    #[test]
    fn test_irf_backward_uses_bzs_minus() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::Label("loop".to_string()),
            StackInstruction::IrF("loop".to_string()),
        ];

        let code = backend.generate(&instructions);
        let branch_window = code
            .windows(4)
            .find(|w| w[0] == 0xB7 && w[1] == 0x00)
            .expect("No se encontró secuencia CPI #0 del ir-f");

        assert_eq!(branch_window[2], 0x9B); // BZS -i
        assert!(branch_window[3] > 0x00); // magnitud de salto
    }

    #[test]
    fn test_irv_backward_uses_bzr_minus() {
        let mut backend = Lh5801Backend::new();
        let instructions = vec![
            StackInstruction::Label("loop".to_string()),
            StackInstruction::IrV("loop".to_string()),
        ];

        let code = backend.generate(&instructions);
        let branch_window = code
            .windows(4)
            .find(|w| w[0] == 0xB7 && w[1] == 0x00)
            .expect("No se encontró secuencia CPI #0 del ir-v");

        assert_eq!(branch_window[2], 0x99); // BZR -i
        assert!(branch_window[3] > 0x00); // magnitud de salto
    }

    #[test]
    fn test_divint_loop_exit_uses_bcr_after_subtract_compare() {
        let code = generated_code_for(StackInstruction::DivInt);
        // REC; LDA UH; SBC UL; BCR +6
        assert!(has_subsequence(&code, &[0xF9, 0xA4, 0x20, 0x81, 0x06]));
    }

    #[test]
    fn test_mayorint_translation_matches_expected_flow() {
        let code = generated_code_for(StackInstruction::MayorInt);
        // REC; SBC UL; BCR +8 (a<b => false); CPI #0; BZS +4 (a==b => false)
        assert!(has_subsequence(&code, &[0xF9, 0x20, 0x81, 0x08, 0xB7, 0x00, 0x8B, 0x04]));
    }

    #[test]
    fn test_menorint_translation_matches_expected_flow() {
        let code = generated_code_for(StackInstruction::MenorInt);
        // REC; SBC UL; BCS +4 (a>=b => false path)
        assert!(has_subsequence(&code, &[0xF9, 0x20, 0x83, 0x04]));
    }

    #[test]
    fn test_menorigual_translation_matches_expected_flow() {
        let code = generated_code_for(StackInstruction::MenorIgualInt);
        // REC; SBC UL; BCR +8 (a<b => true); CPI #0; BZS +4 (a==b => true)
        assert!(has_subsequence(&code, &[0xF9, 0x20, 0x81, 0x08, 0xB7, 0x00, 0x8B, 0x04]));
    }

    #[test]
    fn test_all_integer_comparisons_clear_carry_before_sbc() {
        for instr in [
            StackInstruction::MayorInt,
            StackInstruction::MenorInt,
            StackInstruction::IgualInt,
            StackInstruction::DistintoInt,
            StackInstruction::MayorIgualInt,
            StackInstruction::MenorIgualInt,
        ] {
            let code = generated_code_for(instr);
            assert!(has_subsequence(&code, &[0xF9, 0x20]));
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
        // Tras inicialización (10 bytes), CALL debe emitir SJP y dirección 0x380D
        assert_eq!(code[10], 0xBE);
        assert_eq!(code[11], 0x38);
        assert_eq!(code[12], 0x0D);
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
}
