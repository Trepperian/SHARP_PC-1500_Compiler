/// Instrucciones de la máquina virtual de pila
/// Basadas en las instrucciones máquina P del documento de especificación
#[derive(Debug, Clone, PartialEq)]
pub enum StackInstruction {
    // ========================================================================
    // INSTRUCCIONES DE PILA
    // ========================================================================
    
    /// apila-int(N) - Apilar un entero literal
    ApilaInt(i64),
    
    /// apila-real(R) - Apilar un flotante literal
    ApilaReal(f64),
    
    /// apila-cadena(S) - Apilar una cadena literal
    ApilaCadena(String),
    
    /// apila-bool(B) - Apilar un booleano
    ApilaBool(bool),
    
    /// apila-ind() - Apilar indirecto (lectura de memoria)
    /// Pop dirección, Push valor en esa dirección
    ApilaInd,
    
    /// desapila-ind() - Desapilar indirecto (escritura a memoria)
    /// Pop valor, Pop dirección, Mem[dirección] = valor
    DesapilaInd,
    
    /// apilad(N) - Apilar dirección de nivel N
    /// Usado para acceder a variables en diferentes niveles de anidamiento
    Apilad(usize),
    
    /// dup() - Duplicar cima de pila
    Dup,
    
    /// desapila() - Descartar cima de pila
    Desapila,
    
    // ========================================================================
    // OPERACIONES ARITMÉTICAS
    // ========================================================================
    
    /// suma-int - Pop b, Pop a, Push (a + b) para enteros
    SumaInt,
    
    /// suma-real - Pop b, Pop a, Push (a + b) para reales
    SumaReal,
    
    /// resta-int - Pop b, Pop a, Push (a - b) para enteros
    RestaInt,
    
    /// resta-real - Pop b, Pop a, Push (a - b) para reales
    RestaReal,
    
    /// mul-int - Pop b, Pop a, Push (a * b) para enteros
    MulInt,
    
    /// mul-real - Pop b, Pop a, Push (a * b) para reales
    MulReal,
    
    /// div-int - Pop b, Pop a, Push (a / b) para enteros
    DivInt,
    
    /// div-real - Pop b, Pop a, Push (a / b) para reales
    DivReal,
    
    /// mod-int - Pop b, Pop a, Push (a % b) para enteros
    ModInt,
    
    /// mod-real - Pop b, Pop a, Push (a % b) para reales
    ModReal,
    
    /// pow-int - Pop b, Pop a, Push (a ^ b) para enteros
    PowInt,
    
    /// pow-real - Pop b, Pop a, Push (a ^ b) para reales
    PowReal,
    
    /// negativo - Pop a, Push (-a)
    Negativo,
    
    // ========================================================================
    // OPERACIONES DE COMPARACIÓN
    // ========================================================================
    
    /// menor-int - Pop b, Pop a, Push (a < b)
    MenorInt,
    
    /// menor-real - Pop b, Pop a, Push (a < b)
    MenorReal,
    
    /// mayor-int - Pop b, Pop a, Push (a > b)
    MayorInt,
    
    /// mayor-real - Pop b, Pop a, Push (a > b)
    MayorReal,
    
    /// menor-igual-int - Pop b, Pop a, Push (a <= b)
    MenorIgualInt,
    
    /// menor-igual-real - Pop b, Pop a, Push (a <= b)
    MenorIgualReal,
    
    /// mayor-igual-int - Pop b, Pop a, Push (a >= b)
    MayorIgualInt,
    
    /// mayor-igual-real - Pop b, Pop a, Push (a >= b)
    MayorIgualReal,
    
    /// igual-int - Pop b, Pop a, Push (a == b)
    IgualInt,
    
    /// igual-real - Pop b, Pop a, Push (a == b)
    IgualReal,
    
    /// distinto-int - Pop b, Pop a, Push (a != b)
    DistintoInt,
    
    /// distinto-real - Pop b, Pop a, Push (a != b)
    DistintoReal,
    
    // ========================================================================
    // OPERACIONES LÓGICAS
    // ========================================================================
    
    /// and-int - Pop b, Pop a, Push (a AND b)
    AndInt,
    
    /// and-real - Pop b, Pop a, Push (a AND b)
    AndReal,
    
    /// or-int - Pop b, Pop a, Push (a OR b)
    OrInt,
    
    /// or-real - Pop b, Pop a, Push (a OR b)
    OrReal,
    
    /// not - Pop a, Push (NOT a)
    Not,
    
    // ========================================================================
    // CONTROL DE FLUJO
    // ========================================================================
    
    /// ir-a(etiqueta) - Salto incondicional
    IrA(String),
    
    /// ir-f(etiqueta) - Salto si falso (Pop condición, si falso salta)
    IrF(String),
    
    /// ir-v(etiqueta) - Salto si verdadero (Pop condición, si verdadero salta)
    IrV(String),
    
    /// ir-ind() - Salto indirecto (Pop dirección, salta a ella)
    /// Usado para RETURN en GOSUB
    IrInd,
    
    /// etiqueta / label - Definir etiqueta
    Label(String),
    
    // ========================================================================
    // GESTIÓN DE MEMORIA Y REGISTROS DE ACTIVACIÓN
    // ========================================================================
    
    /// activa(nivel, tam, sig) - Activar registro de activación
    /// Usado para llamadas a procedimientos/funciones
    Activa { nivel: usize, tam: usize, sig: String },
    
    /// desactiva(nivel, tam) - Desactivar registro de activación
    Desactiva { nivel: usize, tam: usize },
    
    /// desapilad(nivel) - Desapilar dirección de nivel
    Desapilad(usize),
    
    /// call(etiqueta) - Llamada a subrutina (GOSUB)
    /// Guarda dirección de retorno y salta
    Call(String),
    
    /// copia(N) - Copiar N bytes del tope de la pila
    Copia(usize),
    
    // ========================================================================
    // CONVERSIONES DE TIPO
    // ========================================================================
    
    /// int2real - Convertir entero a real
    /// Pop int, Push real
    Int2Real,
    
    /// real2int - Convertir real a entero (truncar)
    /// Pop real, Push int
    Real2Int,
    
    // ========================================================================
    // ENTRADA/SALIDA
    // ========================================================================
    
    /// systemin() - Leer entrada estándar
    /// Push valor leído
    SystemIn,
    
    /// systemout() - Escribir a salida estándar
    /// Pop valor, imprimir
    SystemOut,
    
    /// newline() / nl() - Imprimir nueva línea
    Newline,
    
    /// print-tab - Imprimir tabulador
    PrintTab,
    
    // ========================================================================
    // FUNCIONES INCORPORADAS
    // ========================================================================
    
    /// Funciones matemáticas
    CallInt,     // INT(x) - Parte entera
    CallAbs,     // ABS(x) - Valor absoluto
    CallSqr,     // SQR(x) - Raíz cuadrada
    CallSin,     // SIN(x) - Seno
    CallCos,     // COS(x) - Coseno
    CallTan,     // TAN(x) - Tangente
    CallAtn,     // ATN(x) - Arcotangente
    CallExp,     // EXP(x) - e^x
    CallLog,     // LOG(x) - Logaritmo natural
    CallRnd,     // RND(x) - Número aleatorio
    CallSgn,     // SGN(x) - Signo
    
    ///Funciones de cadenas
    CallLen,     // LEN(s) - Longitud de cadena
    CallMid,     // MID$(s, start, len) - Subcadena
    CallLeft,    // LEFT$(s, n) - n caracteres izquierdos
    CallRight,   // RIGHT$(s, n) - n caracteres derechos
    CallChr,     // CHR$(n) - Carácter ASCII
    CallAsc,     // ASC(s) - Código ASCII
    CallStr,     // STR$(n) - Número a cadena
    CallVal,     // VAL(s) - Cadena a número
    
    // ========================================================================
    // INSTRUCCIONES ESPECÍFICAS DE BASIC
    // ========================================================================
    
    /// Instrucciones de datos
    ReadData,    // Leer siguiente elemento de DATA
    StoreData,   // Almacenar dato en área de DATA
    RestoreData, // Restaurar puntero de DATA
    
    /// Instrucciones de sistema
    Clear,       // CLEAR - Limpiar variables
    Cls,         // CLS - Limpiar pantalla
    Stop,        // STOP/END - Detener ejecución
    
    /// Asignación de memoria
    Alloc(usize),   // Reservar memoria (para arrays)
    Dealloc(usize), // Liberar memoria
    
    // ========================================================================
    // INSTRUCCIONES DE SISTEMA SHARP PC-1500
    // ========================================================================
    
    /// Instrucciones de sistema
    Wait,         // WAIT - Esperar
    Random,       // RANDOM - Inicializar generador aleatorio
    Arun,         // ARUN - Auto-run
    Lock,         // LOCK - Bloquear programa
    Unlock,       // UNLOCK - Desbloquear programa
    
    /// Instrucciones de I/O avanzadas
    Pause,        // PAUSE - Pausar ejecución
    LPrint,       // LPRINT - Imprimir a impresora
    Using,        // USING - Formatear salida
    
    /// Instrucciones gráficas
    GPrint,       // GPRINT - Imprimir en gráficos
    GCursor,      // GCURSOR - Posicionar cursor gráfico
    Cursor,       // CURSOR - Posicionar cursor de texto
    LCursor,      // LCURSOR - Cursor de impresora
    GlCursor,     // GLCURSOR - Cursor gráfico con coordenadas
    Line,         // LINE - Dibujar línea
    RLine,        // RLINE - Dibujar línea relativa
    Sorgn,        // SORGN - Origen de coordenadas
    Rotate,       // ROTATE - Rotar coordenadas
    Text,         // TEXT - Modo texto
    Graph,        // GRAPH - Modo gráficos
    Color,        // COLOR - Color de dibujo
    CSize,        // CSIZE - Tamaño de caracteres
    
    /// Instrucciones de sonido
    Beep,         // BEEP - Emitir sonido
    BeepOn,       // BEEP ON - Activar sonido
    BeepOff,      // BEEP OFF - Desactivar sonido
    
    /// Instrucciones de memoria
    Poke,         // POKE - Escribir en memoria
    // Call ya existe arriba
    
    /// Instrucciones de modos matemáticos
    Radian,       // RADIAN - Modo radianes
    Degree,       // DEGREE - Modo grados
    
    /// Control de errores
    OnErrorGoto(String),  // ON ERROR GOTO - Manejo de errores
    
    /// Funciones gráficas y de sistema
    CallPoint,    // POINT(x,y) - Leer pixel
    CallStatus,   // STATUS(n) - Estado de dispositivo
    
    // ========================================================================
    // UTILIDADES Y DEBUGGING
    // ========================================================================
    
    /// nop - No operación
    Nop,
    
    /// comment - Comentario (solo para debugging del código generado)
    Comment(String),
}

impl StackInstruction {
    /// Convertir la instrucción a su representación textual
    /// Formato compatible con las instrucciones máquina P
    pub fn to_string(&self) -> String {
        match self {
            // Pila
            StackInstruction::ApilaInt(n) => format!("apila-int {}", n),
            StackInstruction::ApilaReal(r) => format!("apila-real {}", r),
            StackInstruction::ApilaCadena(s) => format!("apila-cadena \"{}\"", s),
            StackInstruction::ApilaBool(b) => format!("apila-bool {}", b),
            StackInstruction::ApilaInd => "apila-ind".to_string(),
            StackInstruction::DesapilaInd => "desapila-ind".to_string(),
            StackInstruction::Apilad(n) => format!("apilad {}", n),
            StackInstruction::Dup => "dup".to_string(),
            StackInstruction::Desapila => "desapila".to_string(),
            
            // Aritméticas
            StackInstruction::SumaInt => "suma-int".to_string(),
            StackInstruction::SumaReal => "suma-real".to_string(),
            StackInstruction::RestaInt => "resta-int".to_string(),
            StackInstruction::RestaReal => "resta-real".to_string(),
            StackInstruction::MulInt => "mul-int".to_string(),
            StackInstruction::MulReal => "mul-real".to_string(),
            StackInstruction::DivInt => "div-int".to_string(),
            StackInstruction::DivReal => "div-real".to_string(),
            StackInstruction::ModInt => "mod-int".to_string(),
            StackInstruction::ModReal => "mod-real".to_string(),
            StackInstruction::PowInt => "pow-int".to_string(),
            StackInstruction::PowReal => "pow-real".to_string(),
            StackInstruction::Negativo => "negativo".to_string(),
            
            // Comparación
            StackInstruction::MenorInt => "menor-int".to_string(),
            StackInstruction::MenorReal => "menor-real".to_string(),
            StackInstruction::MayorInt => "mayor-int".to_string(),
            StackInstruction::MayorReal => "mayor-real".to_string(),
            StackInstruction::MenorIgualInt => "menor-igual-int".to_string(),
            StackInstruction::MenorIgualReal => "menor-igual-real".to_string(),
            StackInstruction::MayorIgualInt => "mayor-igual-int".to_string(),
            StackInstruction::MayorIgualReal => "mayor-igual-real".to_string(),
            StackInstruction::IgualInt => "igual-int".to_string(),
            StackInstruction::IgualReal => "igual-real".to_string(),
            StackInstruction::DistintoInt => "distinto-int".to_string(),
            StackInstruction::DistintoReal => "distinto-real".to_string(),
            
            // Lógicas
            StackInstruction::AndInt => "and-int".to_string(),
            StackInstruction::AndReal => "and-real".to_string(),
            StackInstruction::OrInt => "or-int".to_string(),
            StackInstruction::OrReal => "or-real".to_string(),
            StackInstruction::Not => "not".to_string(),
            
            // Control de flujo
            StackInstruction::IrA(label) => format!("ir-a {}", label),
            StackInstruction::IrF(label) => format!("ir-f {}", label),
            StackInstruction::IrV(label) => format!("ir-v {}", label),
            StackInstruction::IrInd => "ir-ind".to_string(),
            StackInstruction::Label(label) => format!("{}:", label),
            
            // Registros de activación
            StackInstruction::Activa { nivel, tam, sig } => 
                format!("activa {}, {}, {}", nivel, tam, sig),
            StackInstruction::Desactiva { nivel, tam } => 
                format!("desactiva {}, {}", nivel, tam),
            StackInstruction::Desapilad(nivel) => format!("desapilad {}", nivel),
            StackInstruction::Call(label) => format!("call {}", label),
            StackInstruction::Copia(n) => format!("copia {}", n),
            
            // Conversiones
            StackInstruction::Int2Real => "int2real".to_string(),
            StackInstruction::Real2Int => "real2int".to_string(),
            
            // I/O
            StackInstruction::SystemIn => "systemin".to_string(),
            StackInstruction::SystemOut => "systemout".to_string(),
            StackInstruction::Newline => "newline".to_string(),
            StackInstruction::PrintTab => "print-tab".to_string(),
            
            // Funciones
            StackInstruction::CallInt => "call INT".to_string(),
            StackInstruction::CallAbs => "call ABS".to_string(),
            StackInstruction::CallSqr => "call SQR".to_string(),
            StackInstruction::CallSin => "call SIN".to_string(),
            StackInstruction::CallCos => "call COS".to_string(),
            StackInstruction::CallTan => "call TAN".to_string(),
            StackInstruction::CallAtn => "call ATN".to_string(),
            StackInstruction::CallExp => "call EXP".to_string(),
            StackInstruction::CallLog => "call LOG".to_string(),
            StackInstruction::CallRnd => "call RND".to_string(),
            StackInstruction::CallSgn => "call SGN".to_string(),
            StackInstruction::CallLen => "call LEN".to_string(),
            StackInstruction::CallMid => "call MID$".to_string(),
            StackInstruction::CallLeft => "call LEFT$".to_string(),
            StackInstruction::CallRight => "call RIGHT$".to_string(),
            StackInstruction::CallChr => "call CHR$".to_string(),
            StackInstruction::CallAsc => "call ASC".to_string(),
            StackInstruction::CallStr => "call STR$".to_string(),
            StackInstruction::CallVal => "call VAL".to_string(),
            
            // BASIC específico
            StackInstruction::ReadData => "read-data".to_string(),
            StackInstruction::StoreData => "store-data".to_string(),
            StackInstruction::RestoreData => "restore-data".to_string(),
            StackInstruction::Clear => "clear".to_string(),
            StackInstruction::Cls => "cls".to_string(),
            StackInstruction::Stop => "stop".to_string(),
            StackInstruction::Alloc(n) => format!("alloc {}", n),
            StackInstruction::Dealloc(n) => format!("dealloc {}", n),
            
            // Sistema Sharp PC-1500
            StackInstruction::Wait => "wait".to_string(),
            StackInstruction::Random => "random".to_string(),
            StackInstruction::Arun => "arun".to_string(),
            StackInstruction::Lock => "lock".to_string(),
            StackInstruction::Unlock => "unlock".to_string(),
            
            // I/O avanzado
            StackInstruction::Pause => "pause".to_string(),
            StackInstruction::LPrint => "lprint".to_string(),
            StackInstruction::Using => "using".to_string(),
            
            // Gráficos
            StackInstruction::GPrint => "gprint".to_string(),
            StackInstruction::GCursor => "gcursor".to_string(),
            StackInstruction::Cursor => "cursor".to_string(),
            StackInstruction::LCursor => "lcursor".to_string(),
            StackInstruction::GlCursor => "glcursor".to_string(),
            StackInstruction::Line => "line".to_string(),
            StackInstruction::RLine => "rline".to_string(),
            StackInstruction::Sorgn => "sorgn".to_string(),
            StackInstruction::Rotate => "rotate".to_string(),
            StackInstruction::Text => "text".to_string(),
            StackInstruction::Graph => "graph".to_string(),
            StackInstruction::Color => "color".to_string(),
            StackInstruction::CSize => "csize".to_string(),
            
            // Sonido
            StackInstruction::Beep => "beep".to_string(),
            StackInstruction::BeepOn => "beep-on".to_string(),
            StackInstruction::BeepOff => "beep-off".to_string(),
            
            // Memoria
            StackInstruction::Poke => "poke".to_string(),
            
            // Modos matemáticos
            StackInstruction::Radian => "radian".to_string(),
            StackInstruction::Degree => "degree".to_string(),
            
            // Control de errores
            StackInstruction::OnErrorGoto(label) => format!("on-error-goto {}", label),
            
            // Funciones Sharp PC-1500
            StackInstruction::CallPoint => "call POINT".to_string(),
            StackInstruction::CallStatus => "call STATUS".to_string(),
            
            // Utilidades
            StackInstruction::Nop => "nop".to_string(),
            StackInstruction::Comment(c) => format!("; {}", c),
        }
    }
}
