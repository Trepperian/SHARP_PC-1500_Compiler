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
}

/// Tipo de referencia a etiqueta
#[derive(Debug, Clone, Copy)]
enum RefType {
    /// Salto absoluto de 16 bits (JMP, CALL)
    Absolute16,
    /// Salto relativo de 8 bits con signo (branches)
    Relative8,
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
    
    /// Emitir instrucción HALT
    fn emit_halt(&mut self) {
        // 0xFD 0xB1 - HALT (instrucción extendida)
        self.emit_byte(0xFD);
        self.emit_byte(0xB1);
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
                    RefType::Relative8 => {
                        // Calcular offset relativo
                        let current_pos = *pos + self.start_address as usize + 1;
                        let offset = (target_addr as i32) - (current_pos as i32);
                        
                        if offset < -128 || offset > 127 {
                            panic!("Branch offset too large for label {}: {}", label_name, offset);
                        }
                        
                        self.code[*pos] = offset as u8;
                    }
                }
            } else {
                panic!("Undefined label: {}", label_name);
            }
        }
    }
    
    /// Emitir byte individual
    fn emit_byte(&mut self, byte: u8) {
        self.code.push(byte);
    }
    
    /// Emitir word de 16 bits (little-endian)
    /// El LH5801 usa little-endian para direcciones de 16 bits en instrucciones
    fn emit_word(&mut self, word: u16) {
        self.emit_byte((word & 0xFF) as u8);      // Low byte primero
        self.emit_byte((word >> 8) as u8);        // High byte segundo
    }
    
    /// Emitir placeholder para referencia a etiqueta
    fn emit_label_placeholder(&mut self, ref_type: RefType) {
        match ref_type {
            RefType::Absolute16 => {
                self.emit_word(0x0000); // Placeholder de 2 bytes
            }
            RefType::Relative8 => {
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
        // El LH5801 no tiene CALL directo a dirección arbitraria
        // Usamos SJP (SubJumP) que salta via tabla de vectores
        // 
        // Para direcciones ROM, usamos JMP directo ya que:
        // 1. Las rutinas ROM están diseñadas para ser llamadas
        // 2. Retornan con RET (RTN en LH5801)
        // 3. El PC-1500 gestiona el stack hardware automáticamente
        //
        // Alternativa para futuro: implementar CALL manual si es necesario
        
        // Por ahora usamos VEJ (Vector Jump) si la dirección está en tabla
        // o JMP directo para rutinas ROM específicas
        
        // VEJ usa opcode 0x00-0x7F (tabla de 128 vectores)
        // Pero para simplificar, usamos instrucción CDV (Call Vector)
        // CDV: opcode 0xED seguido de dirección de 16 bits
        // 
        // Nota: En LH5801, CDV no existe. Usamos secuencia:
        // PUSH P (retorno), JMP address
        // Pero PUSH P tampoco existe directamente.
        //
        // Solución: Las rutinas ROM del PC-1500 usan RTN para retornar
        // y esperan ser llamadas con SJP o desde BASIC.
        // Para código nativo, simplemente hacemos JMP y dejamos que
        // la ROM gestione el retorno.
        
        // JMP absoluto: 0x4A (JMP addr)
        self.emit_byte(0x4A);
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
                // TODO: Implementar representación de flotantes
                // Por ahora, convertir a entero
                todo!("Flotantes no implementados aún - usar punto fijo o librería BCD")
            }
            
            StackInstruction::ApilaCadena(_s) => {
                // TODO: Apilar puntero a cadena en área de datos
                todo!("Strings no implementados aún - necesita área de datos")
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
                self.emit_byte(0x22); // ADC UL
                // Nota: ADC usa el carry flag, debemos asegurarnos que está limpio
                // TODO: limpiar carry antes si es necesario
                
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
                
                // 3. Inicializar resultado (A) a 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                
                // 4. Bucle de multiplicación (solo si b > 0)
                // Si b == 0, saltar al final
                self.emit_byte(0x26); // LDA UL
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                // Si Z=1 (b==0), saltar 6 bytes adelante (fin del loop)
                self.emit_byte(0x8B); // BZ +offset
                self.emit_byte(0x06);
                
                // Loop: A += UH; UL--; if (UL != 0) goto Loop
                // A += UH
                self.emit_byte(0x20); // ADC UH
                // UL--
                self.emit_byte(0x26); // LDA UL
                self.emit_byte(0xAD); // DEC A
                self.emit_byte(0x2A); // UL = A
                // if (A != 0) goto -8 bytes
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                self.emit_byte(0x89); // BNZ -offset
                self.emit_byte(0xF8); // -8 en complemento a 2
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            StackInstruction::DivInt => {
                // División usando bucle simple
                // Pop b (divisor), Pop a (dividendo), Push (a / b)
                // Algoritmo: resultado = 0; while(a >= b) { a -= b; resultado++; }
                
                // 1. Pop b (divisor) a UL
                self.emit_pop_a();
                self.emit_byte(0x2A); // UL = A (divisor en UL)
                
                // Verificar división por cero
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                // Si es 0, resultado = 0 y salir
                self.emit_byte(0x8B); // BZ +offset (saltar a push 0)
                self.emit_byte(0x15); // Saltar ~21 bytes
                
                // 2. Pop a (dividendo) a UH
                self.emit_pop_a();
                self.emit_byte(0x28); // UH = A (dividendo en UH)
                
                // 3. Inicializar cociente a 0 en YL
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                self.emit_byte(0x1A); // YL = A
                
                // 4. Bucle: while (UH >= UL) { UH -= UL; YL++; }
                // Comparar UH con UL (UH - UL)
                self.emit_byte(0x25); // LDA UH
                self.emit_byte(0x20); // SBC UL
                // Si resultado < 0 (carry=0), terminamos
                self.emit_byte(0x8D); // BC (branch if carry) +offset
                self.emit_byte(0x0A); // Saltar a fin
                
                // UH = UH - UL (ya está en A)
                self.emit_byte(0x28); // UH = A
                
                // YL++
                self.emit_byte(0x14); // LDA YL
                self.emit_byte(0xAF); // INC A
                self.emit_byte(0x1A); // YL = A
                
                // Repetir loop (saltar -13 bytes)
                self.emit_byte(0x81); // BR siempre
                self.emit_byte(0xF3); // -13 en complemento a 2
                
                // Fin: Mover YL a A
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
                self.emit_byte(0x8B); // Branch plus if Z
                self.add_label_ref(label.clone(), RefType::Relative8);
                self.emit_label_placeholder(RefType::Relative8);
            }
            
            StackInstruction::IrV(label) => {
                // Salto si verdadero (Z=0)
                self.emit_pop_a();
                self.emit_byte(0xB7); // CPA #imm
                self.emit_byte(0x00);
                
                // BNZ (branch if not zero)
                self.emit_byte(0x89); // Branch plus if !Z
                self.add_label_ref(label.clone(), RefType::Relative8);
                self.emit_label_placeholder(RefType::Relative8);
            }
            
            StackInstruction::Call(label) => {
                // Llamada a subrutina
                self.emit_byte(0xBE); // SJP
                // SJP usa tabla de vectores, no dirección directa
                // Necesitamos JMP + manual push de dirección de retorno
                
                // Alternativa: usar vector dinámico
                todo!("CALL no implementado - necesita gestión de subrutinas")
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
                self.emit_byte(0x20); // SBC UL
                // Resultado en A
                
                // 4. Si A > 0, push 1; sino push 0
                // Checar si A == 0 (Z flag)
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                
                // Si Z=1 (igual), resultado = 0
                self.emit_byte(0x8B); // BZ +4
                self.emit_byte(0x04);
                
                // Si A > 0, cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                // Saltar 2 bytes
                self.emit_byte(0x81); // BR +2
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
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si resultado negativo (sin carry), push 1
                // Carry=0 significa que hubo borrow (a < b)
                self.emit_byte(0x8D); // BC (branch if carry) +4
                self.emit_byte(0x04);
                
                // Sin carry (a < b), cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                // Saltar 2 bytes
                self.emit_byte(0x81); // BR +2
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
                self.emit_byte(0x81); // BR +2
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
                self.emit_byte(0x81); // BR +2
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
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si carry set (sin borrow), a >= b
                self.emit_byte(0x8D); // BC (branch if carry) +4
                self.emit_byte(0x04);
                
                // Sin carry (a < b), cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                // Saltar 2 bytes
                self.emit_byte(0x81); // BR +2
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
                self.emit_byte(0x20); // SBC UL
                
                // 4. Si A <= 0 (Z=1 o negativo), push 1
                // Primero chequear Z
                self.emit_byte(0xB7); // CPA #0
                self.emit_byte(0x00);
                
                self.emit_byte(0x8B); // BZ +4 (si igual, cargar 1)
                self.emit_byte(0x04);
                
                // No igual, chequear si es negativo (carry=0)
                self.emit_byte(0x8D); // BC +4
                self.emit_byte(0x04);
                
                // Sin carry (a < b), cargar 1
                self.emit_byte(0xB5); // LDA #1
                self.emit_byte(0x01);
                // Saltar 2 bytes
                self.emit_byte(0x81); // BR +2
                self.emit_byte(0x02);
                
                // Con carry y no igual (a > b), cargar 0
                self.emit_byte(0xB5); // LDA #0
                self.emit_byte(0x00);
                
                // 5. Push resultado
                self.emit_push_a();
            }
            
            // ===== CONTROL =====
            
            StackInstruction::Stop => {
                // Terminar programa - saltar a halt
                // El halt se emite en emit_halt() automáticamente
                // Por ahora, solo emitir HALT directamente
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
        assert_eq!(backend.start_address, 0x7000);
        assert_eq!(backend.sp_address, 0x7FF0);
        assert_eq!(backend.stack_base, 0x7800);
    }
}
