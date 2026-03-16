/// Intérprete de la Máquina P
/// Ejecuta las instrucciones generadas por StackCodeGenerator
/// Similar a MaquinaP.ejecuta() en prac4/Tiny

use super::stack_instruction::StackInstruction;
use std::collections::HashMap;
use std::io::{self, Write};

/// Valores que pueden almacenarse en la pila o memoria
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Real(f64),
    String(String),
    Bool(bool),
}

impl Value {
    /// Convertir a entero (para operaciones lógicas)
    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(n) => *n,
            Value::Real(r) => *r as i64,
            Value::Bool(b) => if *b { 1 } else { 0 },
            Value::String(_) => 0,
        }
    }
    
    /// Convertir a real
    pub fn as_real(&self) -> f64 {
        match self {
            Value::Int(n) => *n as f64,
            Value::Real(r) => *r,
            Value::Bool(b) => if *b { 1.0 } else { 0.0 },
            Value::String(_) => 0.0,
        }
    }
    
    /// Verificar si es "falso" (0, false, cadena vacía)
    pub fn is_false(&self) -> bool {
        match self {
            Value::Int(0) => true,
            Value::Real(r) => *r == 0.0,
            Value::Bool(false) => true,
            Value::String(s) => s.is_empty(),
            _ => false,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Real(r) => write!(f, "{}", r),
            Value::String(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
        }
    }
}

/// Intérprete de código de pila
/// Similar a MaquinaP.java en prac4
pub struct StackMachineInterpreter {
    /// Instrucciones a ejecutar
    instructions: Vec<StackInstruction>,
    
    /// Pila de ejecución
    stack: Vec<Value>,
    
    /// Memoria (dirección → valor)
    memory: HashMap<i64, Value>,
    
    /// Tabla de etiquetas (label → posición)
    labels: HashMap<String, usize>,
    
    /// Program Counter
    pc: usize,
    
    /// Call stack (para GOSUB/RETURN)
    call_stack: Vec<usize>,
    
    /// Flag de ejecución
    running: bool,
    
    /// Modo verbose (mostrar trace de ejecución)
    verbose: bool,
    
    /// Almacenamiento DATA (para READ/RESTORE)
    data_storage: Vec<Value>,
    
    /// Puntero DATA (para READ)
    data_pointer: usize,
}

impl StackMachineInterpreter {
    /// Crear nuevo intérprete
    pub fn new(instructions: Vec<StackInstruction>) -> Self {
        // Construir tabla de etiquetas
        let mut labels = HashMap::new();
        for (i, instr) in instructions.iter().enumerate() {
            if let StackInstruction::Label(label) = instr {
                labels.insert(label.clone(), i);
            }
        }
        
        // Pre-procesar DATA (StoreData)
        // Los DATA statements deben cargarse ANTES de ejecutar el programa
        let mut data_storage = Vec::new();
        let mut temp_stack = Vec::new();
        for instr in &instructions {
            match instr {
                StackInstruction::ApilaInt(n) => temp_stack.push(Value::Int(*n)),
                StackInstruction::ApilaReal(r) => temp_stack.push(Value::Real(*r)),
                StackInstruction::ApilaCadena(s) => temp_stack.push(Value::String(s.clone())),
                StackInstruction::StoreData => {
                    if let Some(value) = temp_stack.pop() {
                        data_storage.push(value);
                    }
                }
                _ => {
                    // Limpiar temp_stack si hay otra instrucción
                    if !matches!(instr, StackInstruction::Comment(_)) {
                        temp_stack.clear();
                    }
                }
            }
        }
        
        Self {
            instructions,
            stack: Vec::new(),
            memory: HashMap::new(),
            labels,
            pc: 0,
            call_stack: Vec::new(),
            running: true,
            verbose: false,
            data_storage,
            data_pointer: 0,
        }
    }
    
    /// Activar modo verbose
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }
    
    /// Ejecutar el programa (método principal, similar a ejecuta() en Tiny)
    pub fn ejecuta(&mut self) {
        println!("\n=== EJECUTANDO PROGRAMA ===\n");
        
        while self.running && self.pc < self.instructions.len() {
            let instr = self.instructions[self.pc].clone();
            
            if self.verbose {
                println!("[PC:{}] {:?} | Stack: {:?}", self.pc, instr, self.stack);
            }
            
            self.ejecuta_instruccion(&instr);
            
            // Solo incrementar PC si no hubo salto
            if self.running {
                self.pc += 1;
            }
        }
        
        println!("\n=== PROGRAMA TERMINADO ===");
        if self.verbose {
            println!("Estado final de la pila: {:?}", self.stack);
            println!("Memoria utilizada: {} variables", self.memory.len());
        }
    }
    
    /// Ejecutar una instrucción individual
    fn ejecuta_instruccion(&mut self, instr: &StackInstruction) {
        match instr {
            // ================================================================
            // INSTRUCCIONES DE PILA
            // ================================================================
            
            StackInstruction::ApilaInt(n) => {
                self.stack.push(Value::Int(*n));
            }
            
            StackInstruction::ApilaReal(r) => {
                self.stack.push(Value::Real(*r));
            }
            
            StackInstruction::ApilaCadena(s) => {
                self.stack.push(Value::String(s.clone()));
            }
            
            StackInstruction::ApilaBool(b) => {
                self.stack.push(Value::Bool(*b));
            }
            
            StackInstruction::ApilaInd => {
                // Pop dirección, push valor en esa dirección
                let addr_val = self.stack.pop().expect("Stack underflow en ApilaInd");
                let addr = addr_val.as_int();
                let value = self.memory.get(&addr)
                    .cloned()
                    .unwrap_or(Value::Int(0)); // Variables no inicializadas = 0
                self.stack.push(value);
            }
            
            StackInstruction::DesapilaInd => {
                // Pop valor, pop dirección, memoria[dirección] = valor
                let value = self.stack.pop().expect("Stack underflow en DesapilaInd (valor)");
                let addr_val = self.stack.pop().expect("Stack underflow en DesapilaInd (dirección)");
                let addr = addr_val.as_int();
                self.memory.insert(addr, value);
            }
            
            StackInstruction::Dup => {
                let value = self.stack.last().expect("Stack underflow en Dup").clone();
                self.stack.push(value);
            }
            
            StackInstruction::Desapila => {
                self.stack.pop().expect("Stack underflow en Desapila");
            }
            
            // ================================================================
            // OPERACIONES ARITMÉTICAS
            // ================================================================
            
            StackInstruction::SumaInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Int(a + b));
            }
            
            StackInstruction::SumaReal => {
                let b = self.stack.pop().expect("Stack underflow").as_real();
                let a = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(a + b));
            }
            
            StackInstruction::RestaInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Int(a - b));
            }
            
            StackInstruction::RestaReal => {
                let b = self.stack.pop().expect("Stack underflow").as_real();
                let a = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(a - b));
            }
            
            StackInstruction::MulInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Int(a * b));
            }
            
            StackInstruction::MulReal => {
                let b = self.stack.pop().expect("Stack underflow").as_real();
                let a = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(a * b));
            }
            
            StackInstruction::DivInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                if b == 0 {
                    eprintln!("ERROR: División por cero");
                    self.running = false;
                } else {
                    self.stack.push(Value::Int(a / b));
                }
            }
            
            StackInstruction::DivReal => {
                let b = self.stack.pop().expect("Stack underflow").as_real();
                let a = self.stack.pop().expect("Stack underflow").as_real();
                if b == 0.0 {
                    eprintln!("ERROR: División por cero");
                    self.running = false;
                } else {
                    self.stack.push(Value::Real(a / b));
                }
            }
            
            StackInstruction::ModInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Int(a % b));
            }
            
            StackInstruction::PowInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Int(a.pow(b as u32)));
            }
            
            StackInstruction::Negativo => {
                let a = self.stack.pop().expect("Stack underflow");
                match a {
                    Value::Int(n) => self.stack.push(Value::Int(-n)),
                    Value::Real(r) => self.stack.push(Value::Real(-r)),
                    _ => panic!("Negativo de tipo no numérico"),
                }
            }
            
            // ================================================================
            // OPERACIONES DE COMPARACIÓN
            // ================================================================
            
            StackInstruction::MenorInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a < b));
            }
            
            StackInstruction::MayorInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a > b));
            }
            
            StackInstruction::MenorIgualInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a <= b));
            }
            
            StackInstruction::MayorIgualInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a >= b));
            }
            
            StackInstruction::IgualInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a == b));
            }
            
            StackInstruction::DistintoInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a != b));
            }
            
            // ================================================================
            // OPERACIONES LÓGICAS
            // ================================================================
            
            StackInstruction::AndInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a != 0 && b != 0));
            }
            
            StackInstruction::OrInt => {
                let b = self.stack.pop().expect("Stack underflow").as_int();
                let a = self.stack.pop().expect("Stack underflow").as_int();
                self.stack.push(Value::Bool(a != 0 || b != 0));
            }
            
            StackInstruction::Not => {
                let a = self.stack.pop().expect("Stack underflow");
                self.stack.push(Value::Bool(a.is_false()));
            }
            
            // ================================================================
            // CONTROL DE FLUJO
            // ================================================================
            
            StackInstruction::IrA(label) => {
                self.pc = *self.labels.get(label)
                    .unwrap_or_else(|| panic!("Label '{}' no encontrada", label));
                return; // No incrementar PC
            }
            
            StackInstruction::IrF(label) => {
                let cond = self.stack.pop().expect("Stack underflow en IrF");
                if cond.is_false() {
                    self.pc = *self.labels.get(label)
                        .unwrap_or_else(|| panic!("Label '{}' no encontrada", label));
                    return; // No incrementar PC
                }
            }
            
            StackInstruction::IrV(label) => {
                let cond = self.stack.pop().expect("Stack underflow en IrV");
                if !cond.is_false() {
                    self.pc = *self.labels.get(label)
                        .unwrap_or_else(|| panic!("Label '{}' no encontrada", label));
                    return; // No incrementar PC
                }
            }
            
            StackInstruction::IrInd => {
                // RETURN - volver a dirección guardada
                if let Some(ret_addr) = self.call_stack.pop() {
                    self.pc = ret_addr;
                    return; // No incrementar PC
                } else {
                    eprintln!("ERROR: RETURN sin GOSUB");
                    self.running = false;
                }
            }
            
            StackInstruction::Label(_) => {
                // No hace nada, solo marca posición
            }
            
            StackInstruction::Call(label) => {
                // GOSUB - guardar dirección de retorno y saltar
                self.call_stack.push(self.pc + 1);
                self.pc = *self.labels.get(label)
                    .unwrap_or_else(|| panic!("Label '{}' no encontrada", label));
                return; // No incrementar PC
            }
            
            // ================================================================
            // ENTRADA/SALIDA
            // ================================================================
            
            StackInstruction::SystemIn => {
                print!("? ");
                io::stdout().flush().unwrap();
                let mut input = String::new();
                io::stdin().read_line(&mut input).expect("Error leyendo entrada");
                
                // Intentar parsear como número
                if let Ok(n) = input.trim().parse::<i64>() {
                    self.stack.push(Value::Int(n));
                } else if let Ok(r) = input.trim().parse::<f64>() {
                    self.stack.push(Value::Real(r));
                } else {
                    self.stack.push(Value::String(input.trim().to_string()));
                }
            }
            
            StackInstruction::SystemOut => {
                let value = self.stack.pop().expect("Stack underflow en SystemOut");
                print!("{}", value);
            }
            
            StackInstruction::Newline => {
                println!();
            }
            
            // ================================================================
            // FUNCIONES INCORPORADAS
            // ================================================================
            
            StackInstruction::CallInt => {
                let value = self.stack.pop().expect("Stack underflow");
                self.stack.push(Value::Int(value.as_int()));
            }
            
            StackInstruction::CallSgn => {
                let value = self.stack.pop().expect("Stack underflow").as_real();
                let result = if value > 0.0 { 1 } else if value < 0.0 { -1 } else { 0 };
                self.stack.push(Value::Int(result));
            }
            
            StackInstruction::CallAbs => {
                let value = self.stack.pop().expect("Stack underflow");
                match value {
                    Value::Int(n) => self.stack.push(Value::Int(n.abs())),
                    Value::Real(r) => self.stack.push(Value::Real(r.abs())),
                    _ => panic!("ABS de tipo no numérico"),
                }
            }
            
            StackInstruction::CallSqr => {
                let value = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(value.sqrt()));
            }
            
            StackInstruction::CallSin => {
                let value = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(value.sin()));
            }
            
            StackInstruction::CallCos => {
                let value = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(value.cos()));
            }
            
            StackInstruction::CallTan => {
                let value = self.stack.pop().expect("Stack underflow").as_real();
                self.stack.push(Value::Real(value.tan()));
            }
            
            StackInstruction::CallRnd => {
                let max = self.stack.pop().expect("Stack underflow").as_int();
                let random = (rand::random::<f64>() * max as f64) as i64;
                self.stack.push(Value::Int(random));
            }
            
            StackInstruction::CallAsc => {
                let value = self.stack.pop().expect("Stack underflow");
                if let Value::String(s) = value {
                    if let Some(c) = s.chars().next() {
                        self.stack.push(Value::Int(c as i64));
                    } else {
                        self.stack.push(Value::Int(0));
                    }
                } else {
                    panic!("ASC esperaba cadena");
                }
            }
            
            StackInstruction::CallChr => {
                let code = self.stack.pop().expect("Stack underflow").as_int();
                let ch = char::from_u32(code as u32).unwrap_or('?');
                self.stack.push(Value::String(ch.to_string()));
            }
            
            // ================================================================
            // CONTROL DEL PROGRAMA
            // ================================================================
            
            StackInstruction::Stop => {
                self.running = false;
            }
            
            StackInstruction::Comment(_) => {
                // Los comentarios no hacen nada
            }
            
            // ================================================================
            // DATA/READ/RESTORE
            // ================================================================
            
            StackInstruction::StoreData => {
                // Almacenar valor en data_storage (para DATA statement)
                let value = self.stack.pop().expect("Stack underflow en StoreData");
                self.data_storage.push(value);
            }
            
            StackInstruction::ReadData => {
                // Leer valor de data_storage y apilar
                if self.data_pointer < self.data_storage.len() {
                    let value = self.data_storage[self.data_pointer].clone();
                    self.stack.push(value);
                    self.data_pointer += 1;
                } else {
                    eprintln!("ERROR: READ sin DATA disponible");
                    self.running = false;
                }
            }
            
            StackInstruction::RestoreData => {
                // RESTORE - resetear puntero DATA
                // Si hay un valor en la pila, es el índice al que saltar
                if !self.stack.is_empty() {
                    let index = self.stack.pop().expect("Stack underflow").as_int();
                    self.data_pointer = index as usize;
                } else {
                    // RESTORE sin argumento - volver al inicio
                    self.data_pointer = 0;
                }
            }
            
            // ================================================================
            // INSTRUCCIONES DE SISTEMA
            // ================================================================
            
            StackInstruction::Clear => {
                // CLEAR - limpiar variables (excepto DATA)
                self.memory.clear();
                self.stack.clear();
                self.call_stack.clear();
            }
            
            StackInstruction::Cls => {
                // CLS - Limpiar pantalla (simular con newlines)
                if !self.verbose {
                    print!("\x1B[2J\x1B[H"); // ANSI clear screen
                }
            }
            
            StackInstruction::Wait => {
                // WAIT - esperar (no-op en simulación)
                // En hardware real esperaría tiempo/input
                let _duration = self.stack.pop().expect("Stack underflow en Wait");
            }
            
            StackInstruction::Cursor => {
                // CURSOR - posicionar cursor texto (no-op por ahora)
                let _pos = self.stack.pop().expect("Stack underflow en Cursor");
            }
            
            // ================================================================
            // INSTRUCCIONES GRÁFICAS (simplificadas/no-op)
            // ================================================================
            
            StackInstruction::GCursor => {
                // GCURSOR - posicionar cursor gráfico (no-op)
                let _pos = self.stack.pop().expect("Stack underflow en GCursor");
            }
            
            StackInstruction::GPrint => {
                // GPRINT - imprimir gráfico (simular como print normal)
                // En el Sharp PC-1500 imprimiría caracteres gráficos
                // Por ahora lo tratamos como no-op
                let _value = self.stack.pop().expect("Stack underflow en GPrint");
                // No imprimimos nada para no interferir con la salida
            }
            
            StackInstruction::Beep => {
                // BEEP - sonido (no-op)
                // En hardware real haría beep
                // Desapilar parámetros: duración, tono, volumen
                if !self.stack.is_empty() {
                    let _param1 = self.stack.pop();
                    if !self.stack.is_empty() {
                        let _param2 = self.stack.pop();
                        if !self.stack.is_empty() {
                            let _param3 = self.stack.pop();
                        }
                    }
                }
            }
            
            StackInstruction::Poke => {
                // POKE - escribir en memoria (simular)
                let value = self.stack.pop().expect("Stack underflow en Poke (valor)");
                let addr = self.stack.pop().expect("Stack underflow en Poke (dirección)");
                // Simular escritura en memoria especial
                // Por ahora solo lo registramos
                if self.verbose {
                    eprintln!("POKE addr={} value={}", addr, value);
                }
            }
            
            // ================================================================
            // INSTRUCCIONES NO IMPLEMENTADAS (de momento)
            // ================================================================
            
            _ => {
                if self.verbose {
                    eprintln!("ADVERTENCIA: Instrucción no implementada: {:?}", instr);
                }
                // No detener ejecución, solo advertir
            }
        }
    }
}
