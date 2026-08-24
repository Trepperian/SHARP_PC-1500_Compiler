/// Instrucciones de la máquina virtual de pila
/// Basadas en las instrucciones máquina P del documento de especificación
#[derive(Debug, Clone, PartialEq)]
pub enum StackInstruction {
    // ========================================================================
    // INSTRUCCIONES DE PILA
    // ========================================================================
    
    /// apila-int(N) - Apilar un entero literal
    ApilaInt(i64),

    /// apila-int-word(N) - Como `ApilaInt`, pero siempre empuja 2 bytes
    /// (alto, bajo), incluso para N=0 o N<=255. `ApilaInt` elige 1 o 2
    /// bytes según la magnitud, lo que no vale cuando el consumidor
    /// (p.ej. `RestoreData`, que compara contra números de línea BASIC de
    /// hasta 16 bits) siempre necesita exactamente 2 bytes en la pila —
    /// usarla ahí para un valor pequeño desequilibraba la pila (1 byte
    /// empujado, 2 desapilados).
    ApilaIntWord(i64),

    /// apila-real(R) - Apilar un flotante literal
    ApilaReal(f64),
    
    /// apila-cadena(S) - Apilar una cadena literal
    ApilaCadena(String),
    
    /// apila-bool(B) - Apilar un booleano
    ApilaBool(bool),
    
    /// apila-ind() - Apilar indirecto (lectura de memoria)
    /// Pop dirección, Push valor en esa dirección
    ApilaInd,

    /// apila-ind-word() - como `ApilaInd`, pero para un valor de 16 bits
    /// (p.ej. el puntero de una variable de cadena escalar). Espejo de
    /// lectura de `DesapilaIndWord` — mismo motivo: `ApilaInd` solo
    /// maneja 8 bits, perdería el byte alto del puntero.
    ApilaIndWord,

    /// desapila-ind() - Desapilar indirecto (escritura a memoria)
    /// Pop valor, Pop dirección, Mem[dirección] = valor
    DesapilaInd,

    /// desapila-ind-word() - como `DesapilaInd`, pero para un valor de 16
    /// bits (p.ej. un puntero a cadena: alto y bajo, dos bytes en la
    /// pila). Necesario porque `DesapilaInd` solo maneja valores de 8
    /// bits — usarla con un puntero perdería el byte alto y descuadraría
    /// la pila. Usada para variables de cadena escalares (`Z$`, no un
    /// array de ancho fijo).
    DesapilaIndWord,

    /// desapila-ind-string-copy(N) - Pop puntero origen (16 bits), Pop
    /// dirección destino (16 bits), copia hasta N bytes desde el origen
    /// al destino (parando en el primer NUL, rellenando el resto con NUL).
    /// Para elementos de arrays de cadena de ancho fijo (`DIM A$(N)*M`):
    /// a diferencia de una variable de cadena escalar (que solo guarda un
    /// puntero), cada elemento es un buffer de M bytes reservado por su
    /// cuenta — asignarle una cadena copia los caracteres dentro de ese
    /// buffer, no sobreescribe un puntero.
    DesapilaIndStringCopy(usize),

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

    /// suma-int-word - Pop offset (entero de 8 bits), Pop base (entero
    /// de 16 bits, p.ej. de `ApilaIntWord`), Push (base + offset) como
    /// entero de 16 bits (con acarreo del byte bajo al alto). Usado para
    /// `RESTORE <base constante> + <expresión>` (p.ej. `RESTORE
    /// 999+RND 16` en bathyscaph.bas) — el único caso de aritmética de
    /// 16 bits que necesita este backend hoy, así que no hay una familia
    /// completa de operaciones de 16 bits (resta/multiplicación/...),
    /// solo esta suma.
    SumaIntWord,

    /// extiende-int-word - Pop entero de 8 bits, Push el mismo valor como
    /// entero de 16 bits (byte alto = 0). Usado para `GOTO`/`GOSUB
    /// <variable>` calculado sin el patrón `<constante>+<expresión>`
    /// (p.ej. `GOTO D`): cubre el caso común de una variable con un
    /// número de línea que cabe en 8 bits (0-255) — como toda la
    /// aritmética entera de este backend es de 8 bits, es lo único que
    /// puede producir una variable normal.
    ExtendIntToWord,

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

    /// igual-cadena - Pop puntero b, Pop puntero a, Push (a == b ? 1 : 0)
    /// Compara el CONTENIDO de las cadenas byte a byte (no los punteros).
    IgualCadena,

    /// distinto-cadena - Pop puntero b, Pop puntero a, Push (a != b ? 1 : 0)
    DistintoCadena,
    
    // ========================================================================
    // OPERACIONES LÓGICAS
    // ========================================================================
    
    /// and-int(scratch) - Pop b, Pop a, Push (a AND b) bit a bit.
    /// `scratch` es la dirección de un byte de trabajo (asignada una vez
    /// por StackCodeGenerator, igual que el índice de DATA) — el LH5801
    /// no tiene un AND registro-a-registro directo con el operando en la
    /// pila, solo `AND direccion` (memoria absoluta), así que hace falta
    /// un sitio en memoria donde dejar `b` mientras `a` está en A.
    AndInt(usize),

    /// and-real - Pop b, Pop a, Push (a AND b)
    AndReal,

    /// or-int(scratch) - como AndInt, para OR bit a bit.
    OrInt(usize),
    
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

    /// line-table - Tabla completa de números de línea del programa (para
    /// `GOTO`/`GOSUB` calculado). Se emite una vez, al principio, igual
    /// que `DataPool`/`DataLineTable` — no genera código por sí sola,
    /// solo alimenta a `IrIndirect`/`CallIndirect`.
    LineTable(Vec<u16>),

    /// ir-ind-linea() - `GOTO <expresión>` con destino no resoluble en
    /// tiempo de compilación (ni número de línea literal ni etiqueta de
    /// cadena): Pop número de línea (16 bits), buscarlo en `LineTable`
    /// (búsqueda lineal en tiempo de compilación, mismo patrón que
    /// `ReadData`/`RestoreData`) y saltar a esa línea. Línea inexistente
    /// en tiempo de ejecución: comportamiento indefinido (no hay forma
    /// segura de abortar sin más infraestructura del intérprete real,
    /// que tampoco lo comprueba en tiempo de compilación).
    IrIndirect,

    /// call-ind-linea() - Como `IrIndirect`, pero para `GOSUB
    /// <expresión>` calculado: llama (guarda dirección de retorno) en
    /// vez de saltar sin más.
    CallIndirect,


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

    /// call-inkey(char_buf, ptr_slot) - INKEY$: sondeo de teclado NO
    /// bloqueante y SIN eco (a diferencia de `SystemIn`/`INPUT`). No
    /// apila el resultado directamente: escribe el puntero de cadena
    /// resultante en `ptr_slot` (2 bytes), para que el llamador
    /// (`gen_lvalue_address`) lo trate como cualquier otra variable de
    /// cadena — apila `ptr_slot` y deja que `ApilaIndWord` lo desreferencie.
    CallInkey(usize, usize),
    
    /// systemout-int() - Pop entero de 8 bits con signo, imprimir su
    /// representación decimal (dígitos ASCII, con signo '-' si es
    /// negativo) — NO el carácter cuyo código coincide con el valor.
    SystemOutInt,

    /// systemout-string() - Pop puntero de 16 bits, imprimir la cadena
    /// NUL-terminada a la que apunta, carácter a carácter.
    SystemOutString,
    
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
    /// call-rnd(seed_addr) - Pop n, Push un entero pseudoaleatorio en
    /// [0, n). `seed_addr` es la dirección de un byte de estado
    /// persistente (asignado una vez por StackCodeGenerator, igual que
    /// el scratch de AND/OR) — NO es el algoritmo real de la ROM (ver
    /// comentario en el backend: la ruta real de `RAND_GEN` para n>0
    /// pasa por varias subrutinas sin documentar y quedó pendiente de
    /// investigar más), es un LFSR de Galois de 8 bits autocontenido,
    /// documentado como sustituto explícito no auténtico.
    CallRnd(usize),
    CallSgn,     // SGN(x) - Signo
    
    ///Funciones de cadenas
    ///
    /// `max_len` (donde aparece) es la cota de bytes segura para recorrer
    /// la cadena fuente en tiempo de ejecución — calculada en
    /// tiempo de compilación por `StackCodeGenerator::string_source_max_len`
    /// (ancho real declarado si la fuente es un array de cadena de ancho
    /// fijo, o un tope genérico si es una variable escalar/literal
    /// NUL-terminada). `LEFT$`/`RIGHT$`/`MID$` escriben su resultado en un
    /// buffer compartido de tamaño fijo por función (no por sitio de
    /// llamada) — una llamada anidada a la MISMA función dentro de sí
    /// misma (p.ej. `LEFT$(LEFT$(A$,5),2)`) podría corromper el
    /// resultado, caso no soportado (no aparece en los programas
    /// objetivo).
    CallLen(usize),               // LEN(s) - Longitud de cadena
    CallMid(usize, usize),        // MID$(s, start, len) - Subcadena (max_len, buffer de resultado)
    CallLeft(usize, usize),       // LEFT$(s, n) - n caracteres izquierdos (max_len, buffer)
    CallRight(usize, usize),      // RIGHT$(s, n) - n caracteres derechos (max_len, buffer)
    CallChr(usize),                // CHR$(n) - Carácter ASCII (buffer de resultado, 2 bytes)
    CallAsc,     // ASC(s) - Código ASCII
    CallStr(usize),               // STR$(n) - Número a cadena (buffer de resultado)
    CallVal(usize, usize),        // VAL(s) - Cadena a número (max_len, scratch de 4 bytes)
    
    // ========================================================================
    // INSTRUCCIONES ESPECÍFICAS DE BASIC
    // ========================================================================
    
    /// Instrucciones de datos
    ///
    /// A diferencia del resto de instrucciones, `DATA` no es código
    /// ejecutable en BASIC real (el intérprete lo salta al llegar a esa
    /// línea) — los valores de todos los `DATA` del programa son estáticos
    /// y se recogen en una sola pasada previa sobre todo el `Program`
    /// (`StackCodeGenerator::collect_data_pool`), no en el orden en que
    /// aparecen en el flujo de control. `DataPool`/`DataLineTable` se
    /// emiten una única vez, al principio, con el resultado de esa
    /// pasada; el backend LH5801 los usa para construir la tabla de
    /// cadenas y la tabla de búsqueda de RESTORE, no generan código.
    ///
    /// El campo `usize` de `ReadData`/`RestoreData` es la dirección de la
    /// variable de scratch "índice de dato actual", asignada una vez por
    /// `StackCodeGenerator` (igual que el scratch del STEP de un FOR) y
    /// compartida por todas las apariciones de READ/RESTORE del programa.
    DataPool(Vec<String>),
    DataLineTable(Vec<(u16, usize)>),
    ReadData(usize),    // Leer el dato en el índice actual, avanzarlo
    RestoreData(usize), // Pop número de línea (0 = principio), fijar el índice
    
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
    Random(usize),       // RANDOM - Inicializar generador aleatorio (dirección de la semilla, compartida con CallRnd)
    Arun,         // ARUN - Auto-run
    Lock,         // LOCK - Bloquear programa
    Unlock,       // UNLOCK - Desbloquear programa
    
    /// Instrucciones de I/O avanzadas
    Pause,        // PAUSE - Pausar ejecución
    LPrint,       // LPRINT - Imprimir a impresora
    Using,        // USING - Formatear salida
    
    /// Instrucciones gráficas
    GPrint,       // GPRINT - Imprimir en gráficos (valor numérico: 1 byte, 1 columna)

    /// gprint-string(len) - Pop puntero (16 bits) a un buffer de `len`
    /// bytes; imprime cada byte como un patrón de puntos en columnas
    /// consecutivas, avanzando el cursor gráfico tras cada uno. `len` es
    /// el tamaño en tiempo de compilación (literal de cadena o elemento
    /// de array de cadena de ancho fijo) — GPRINT de una cadena en la
    /// ROM real imprime cada byte de la cadena como una columna, no el
    /// texto legible.
    GPrintString(usize),
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
            StackInstruction::ApilaIntWord(n) => format!("apila-int-word {}", n),
            StackInstruction::ApilaReal(r) => format!("apila-real {}", r),
            StackInstruction::ApilaCadena(s) => format!("apila-cadena \"{}\"", s),
            StackInstruction::ApilaBool(b) => format!("apila-bool {}", b),
            StackInstruction::ApilaInd => "apila-ind".to_string(),
            StackInstruction::ApilaIndWord => "apila-ind-word".to_string(),
            StackInstruction::DesapilaInd => "desapila-ind".to_string(),
            StackInstruction::DesapilaIndWord => "desapila-ind-word".to_string(),
            StackInstruction::DesapilaIndStringCopy(n) => format!("desapila-ind-string-copy {n}"),
            StackInstruction::Apilad(n) => format!("apilad {}", n),
            StackInstruction::Dup => "dup".to_string(),
            StackInstruction::Desapila => "desapila".to_string(),
            
            // Aritméticas
            StackInstruction::SumaInt => "suma-int".to_string(),
            StackInstruction::SumaReal => "suma-real".to_string(),
            StackInstruction::SumaIntWord => "suma-int-word".to_string(),
            StackInstruction::ExtendIntToWord => "extiende-int-word".to_string(),
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
            StackInstruction::IgualCadena => "igual-cadena".to_string(),
            StackInstruction::DistintoCadena => "distinto-cadena".to_string(),
            
            // Lógicas
            StackInstruction::AndInt(addr) => format!("and-int @{addr:#x}"),
            StackInstruction::AndReal => "and-real".to_string(),
            StackInstruction::OrInt(addr) => format!("or-int @{addr:#x}"),
            StackInstruction::OrReal => "or-real".to_string(),
            StackInstruction::Not => "not".to_string(),
            
            // Control de flujo
            StackInstruction::IrA(label) => format!("ir-a {}", label),
            StackInstruction::IrF(label) => format!("ir-f {}", label),
            StackInstruction::IrV(label) => format!("ir-v {}", label),
            StackInstruction::IrInd => "ir-ind".to_string(),
            StackInstruction::Label(label) => format!("{}:", label),
            StackInstruction::LineTable(table) => format!("line-table ({} entries)", table.len()),
            StackInstruction::IrIndirect => "ir-ind-linea".to_string(),
            StackInstruction::CallIndirect => "call-ind-linea".to_string(),
            
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
            StackInstruction::CallInkey(char_buf, ptr_slot) => format!("call INKEY$ (buf @{char_buf:#x}, ptr @{ptr_slot:#x})"),
            StackInstruction::SystemOutInt => "systemout-int".to_string(),
            StackInstruction::SystemOutString => "systemout-string".to_string(),
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
            StackInstruction::CallRnd(addr) => format!("call RND @{addr:#x}"),
            StackInstruction::CallSgn => "call SGN".to_string(),
            StackInstruction::CallLen(max_len) => format!("call LEN (max {max_len})"),
            StackInstruction::CallMid(max_len, buf) => format!("call MID$ (max {max_len}, buf @{buf:#x})"),
            StackInstruction::CallLeft(max_len, buf) => format!("call LEFT$ (max {max_len}, buf @{buf:#x})"),
            StackInstruction::CallRight(max_len, buf) => format!("call RIGHT$ (max {max_len}, buf @{buf:#x})"),
            StackInstruction::CallChr(buf) => format!("call CHR$ (buf @{buf:#x})"),
            StackInstruction::CallAsc => "call ASC".to_string(),
            StackInstruction::CallStr(buf) => format!("call STR$ (buf @{buf:#x})"),
            StackInstruction::CallVal(max_len, scratch) => format!("call VAL (max {max_len}, scratch @{scratch:#x})"),
            
            // BASIC específico
            StackInstruction::DataPool(items) => format!("data-pool ({} items)", items.len()),
            StackInstruction::DataLineTable(table) => format!("data-line-table ({} entries)", table.len()),
            StackInstruction::ReadData(addr) => format!("read-data @{addr:#x}"),
            StackInstruction::RestoreData(addr) => format!("restore-data @{addr:#x}"),
            StackInstruction::Clear => "clear".to_string(),
            StackInstruction::Cls => "cls".to_string(),
            StackInstruction::Stop => "stop".to_string(),
            StackInstruction::Alloc(n) => format!("alloc {}", n),
            StackInstruction::Dealloc(n) => format!("dealloc {}", n),
            
            // Sistema Sharp PC-1500
            StackInstruction::Wait => "wait".to_string(),
            StackInstruction::Random(seed_addr) => format!("random (seed @{seed_addr:#x})"),
            StackInstruction::Arun => "arun".to_string(),
            StackInstruction::Lock => "lock".to_string(),
            StackInstruction::Unlock => "unlock".to_string(),
            
            // I/O avanzado
            StackInstruction::Pause => "pause".to_string(),
            StackInstruction::LPrint => "lprint".to_string(),
            StackInstruction::Using => "using".to_string(),
            
            // Gráficos
            StackInstruction::GPrint => "gprint".to_string(),
            StackInstruction::GPrintString(len) => format!("gprint-string({len})"),
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
