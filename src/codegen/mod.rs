pub mod stack_instruction;
pub mod interpreter;
pub mod lh5801_backend;
pub mod pc1500_tokenizer;
pub mod lh5_format;
pub mod rom_routines;
pub mod system_memory;
#[cfg(test)]
pub(crate) mod test_oracle;

use crate::parse::program::Program;
use crate::parse::code_line::CodeLine;
use crate::parse::statement::{Statement, statement_inner::StatementInner};
use crate::parse::expression::{Expr, expr_inner::ExprInner, binary_op::BinaryOp, unary_op::UnaryOp};
use crate::parse::expression::function::{Function, FunctionInner};
use crate::parse::expression::lvalue::{LValue, LValueInner};
use crate::parse::statement::assignment::Assignment;
use crate::parse::statement::let_inner::LetInner;
use crate::parse::statement::print_inner::PrintInner;
use crate::parse::statement::lprint_inner::LPrintInner;
use crate::parse::statement::using_clause::UsingClause;
use crate::parse::statement::lcursor_clause::LCursorClause;
use crate::parse::statement::line_inner::LineInner;
use crate::parse::statement::beep_params::BeepParams;
use crate::parse::expression::memory_area::MemoryArea;
use crate::parse::statement::{dim_inner::DimInner, print_separator::PrintSeparator};
use crate::semantic_analysis::BasicType;
use crate::lex::keyword::Keyword;
use stack_instruction::StackInstruction;
use std::collections::HashMap;

/// Generador de código intermedio de pila
/// Basado en la especificación de gen_Cod
pub struct StackCodeGenerator {
    /// Instrucciones generadas
    instructions: Vec<StackInstruction>,
    
    /// Contador para etiquetas temporales
    label_counter: usize,
    
    /// Tabla de símbolos con información de tipos
    /// Mapeamos identificadores a sus tipos
    symbol_table: HashMap<String, BasicType>,
    
    /// Direcciones de variables (simplificado: índice * tamaño)
    variable_addresses: HashMap<String, usize>,
    next_address: usize,

    /// Stack de contextos FOR (para emparejar FOR con NEXT)
    for_stack: Vec<ForContext>,

    /// Metadatos reales de los arrays declarados con `DIM` (nombre →
    /// dirección base, tamaño de elemento y dimensiones), para que
    /// `gen_lvalue_address` calcule direcciones de acceso correctas en vez
    /// de asumir tamaños hardcodeados. Solo se registran arrays cuyo
    /// tamaño es una constante conocida en tiempo de compilación (el
    /// "núcleo reducido" de esta fase) — un `DIM` con tamaño dinámico
    /// (p.ej. `DIM B$(R)`, con `R` variable) no se registra aquí todavía;
    /// ver el comentario en `gen_dim`.
    array_metadata: HashMap<String, ArrayMeta>,

    /// Valores de todos los `DATA` del programa, recogidos en orden de
    /// número de línea (no de ejecución — `DATA` no es código ejecutable
    /// en BASIC real) por `collect_data_pool`. Solo cadenas por ahora
    /// (núcleo reducido: es lo único que necesitan los programas reales
    /// que usan DATA en este corpus, p.ej. bathyscaph.bas).
    data_pool: Vec<String>,

    /// Número de línea de cada `DATA` → índice de su primer valor en
    /// `data_pool`, para resolver `RESTORE <línea>`.
    data_line_index: HashMap<u16, usize>,

    /// Dirección base del área de variables (`DATA_BASE` como valor de
    /// instancia en vez de constante — ver [`compile_native_two_pass`]).
    data_base: usize,

    /// Nombres de variables escalares (sin `$`) que, en algún `LET` del
    /// programa, reciben el resultado de una expresión real (`is_real_expr`
    /// devuelve `true` para el lado derecho) — recogido por
    /// `collect_real_variables` antes de generar código, igual que
    /// `collect_data_pool` recoge los `DATA`. Sin esto, cada variable
    /// numérica se trataba SIEMPRE como entera de 8 bits (ver el comentario
    /// histórico de `is_real_expr`): una variable pensada para acumular un
    /// valor fraccionario entre sentencias (p.ej. `B=B+.5` en un bucle)
    /// nunca se leía ni escribía de forma consistente, porque "es real" se
    /// decidía solo mirando la expresión de CADA sentencia por separado,
    /// nunca la variable en sí. Bug real jugando bombing.bas: `B=B+.5`
    /// (línea 160) calculaba correctamente un resultado real de 8 bytes
    /// (vía `SumaReal`/`ADDIT`), pero se guardaba con `DesapilaInd` (que
    /// solo consume 1 byte), perdiendo los otros 5 bytes en la pila
    /// hardware en cada vuelta del bucle principal — clase de bug de fuga
    /// de pila ya vista antes con `GPRINT MID$`, aquí con consecuencias
    /// mucho más graves por repetirse decenas de veces por partida hasta
    /// desincronizar la pila por completo (visible como escrituras a
    /// memoria no mapeada, dirección 0x0000).
    ///
    /// Deliberadamente EXCLUYE cualquier variable usada alguna vez como
    /// variable de control de un `FOR` (ver `collect_real_variables`):
    /// `gen_for`/`gen_next` siempre incrementan con `SumaInt` y comparan
    /// como enteros de 8 bits, sin ninguna noción de real — extender eso
    /// es una pieza mayor, fuera de alcance aquí (documentado como
    /// limitación conocida, no un olvido).
    real_variables: std::collections::HashSet<String>,

    /// Nombres de variables escalares enteras (sin `$`, y NO marcadas
    /// `real_variables`) que, en algún `LET` del programa, reciben un
    /// literal entero fuera de 0..=255, o el resultado de sumar una de
    /// estas mismas variables — recogido por `collect_word_variables`
    /// (mismo patrón de punto fijo que `collect_real_variables`, ver ese
    /// comentario). Sin esto, TODA aritmética entera de este backend es
    /// de 8 bits (`SumaInt`/`DesapilaInd`, 1 byte) — una variable que
    /// necesita más rango (p.ej. `C=299` seguido de `C=C+3` en
    /// invader-v2.bas, usada como número de línea en `RESTORE C+RND 3`)
    /// se truncaba en silencio Y desincronizaba la pila (`ApilaInt`
    /// empuja 2 bytes para un literal >255, pero `DesapilaInd`/`ApilaInd`
    /// siempre asumen 1). Bug real: el terreno de invader-v2.bas siempre
    /// mostraba el mismo tramo en vez de variado, porque `C` nunca
    /// llegaba a valer realmente 299+ y `RESTORE` siempre apuntaba casi
    /// al mismo sitio.
    ///
    /// Alcance deliberadamente acotado al patrón real observado: solo
    /// `LET`/asignación, `+` (contagioso, como en `real_variables`),
    /// `RESTORE`/`GOTO`/`GOSUB <expr>` calculado (ya tenían su propio
    /// camino de 16 bits, ver `gen_dynamic_line_number`) y `PRINT`. Una
    /// variable "de palabra" usada en resta/multiplicación/división/
    /// comparación cae a `TruncateWordToInt` en `gen_binary_op` (pierde
    /// precisión por encima de 255 en vez de desincronizar la pila) — no
    /// hay ningún caso así en el corpus real hoy. También EXCLUYE
    /// variables de control de `FOR` (mismo motivo que `real_variables`:
    /// `gen_for`/`gen_next` no tienen ninguna noción de 16 bits).
    word_variables: std::collections::HashSet<String>,

    /// Mecanismo genérico de ritmo de ejecución (opt-in, `--authentic-timing`
    /// en `main.rs`) — ver el comentario largo de
    /// `StackInstruction::AuthenticTimingDelay`. `false` en TODO el código
    /// existente hasta ahora (todos los constructores previos a este
    /// campo, y todos los tests ya escritos, que usan `new()`/
    /// `with_data_base()` sin este flag): con `false`, `gen_code_line`
    /// nunca emite la instrucción de espera, así que el código generado
    /// es byte-idéntico al de antes de que este mecanismo existiera —
    /// cero riesgo para lo que ya funciona. Solo pasa a `true` cuando se
    /// pide explícitamente vía `with_data_base_and_timing`.
    authentic_timing: bool,

    /// Formato `USING` activo, o `None` si no hay ninguno (formato
    /// decimal simple). Se actualiza en tiempo de COMPILACIÓN al procesar
    /// cada sentencia `USING <patrón>` o `PRINT USING <patrón>;...`
    /// (`patrón` siempre debe ser un literal de cadena — núcleo reducido,
    /// como el resto de patrones/tamaños constantes de este backend) y se
    /// consulta cada vez que `PRINT`/`PRINT USING` imprime un valor
    /// numérico. No hace falta ningún estado en tiempo de EJECUCIÓN: como
    /// el patrón siempre se conoce en compilación, todo el ancho fijo se
    /// resuelve aquí, no en la ROM real (que si usara esto en tiempo real
    /// leería el bloque `USING_BLOCK`, `$7895-7898` — investigar su
    /// formato exacto habría sido una tarea de investigación de ROM con
    /// riesgo de acabar en callejón sin salida, como pasó con `RND`).
    current_using_format: Option<UsingFormat>,

    /// `true` si el programa usa `SQR` alguna vez — controla si
    /// `generate()` emite la subrutina compartida `__SQR_ROUTINE` (ver el
    /// comentario de `FunctionInner::Sqr`). Emitirla siempre, la use el
    /// programa o no, desperdiciaría ~200 bytes de un presupuesto de
    /// código ya ajustado (ver la investigación de RAM de esta misma
    /// tanda de trabajo: 10240 bytes es el techo real de hardware).
    sqr_used: bool,

    /// `true` en cuanto se ha emitido la inicialización única del heap de
    /// arrays con `DIM` dinámico (`__ARRAY_HEAP_PTR = __ARRAY_HEAP`) — ver
    /// el comentario largo en `gen_dim`. Se hace una sola vez, en el punto
    /// del PRIMER `DIM` dinámico que aparece en el código fuente (asume
    /// que los `DIM` se ejecutan una sola vez cada uno, al principio del
    /// programa — el patrón real de todo el corpus; un `DIM` dinámico
    /// dentro de un bucle reinicializaría el heap en cada vuelta y
    /// aliasearía arrays entre sí, límite conocido no verificado en
    /// ningún programa real).
    dynamic_array_heap_initialized: bool,

    /// Todas las etiquetas de usuario (literales de cadena que prefijan
    /// una línea, p.ej. `"*9"A=...`) que aparecen en el programa,
    /// recogidas por `collect_string_labels` antes de generar nada —
    /// mismo motivo que `data_pool`/`line_numbers`: un `GOTO`/`GOSUB` a
    /// una etiqueta de cadena CALCULADA en tiempo de ejecución (p.ej.
    /// `GOTO "*"+INKEY$`, patrón real de invader-v2.bas) necesita conocer
    /// de antemano el conjunto de etiquetas candidatas que empiezan por
    /// el mismo prefijo constante, para generar una cascada de
    /// comparaciones — ver `gen_computed_string_goto`.
    all_string_labels: Vec<String>,
}

/// Patrón `USING` ya parseado (ver `parse_using_pattern`): cuántos
/// dígitos antes/después del punto decimal, y los dos modificadores que
/// aparecen en el corpus real (`*` de relleno con asteriscos, `+` de
/// signo forzado).
#[derive(Debug, Clone, Copy)]
struct UsingFormat {
    digits_before: u8,
    digits_after: u8,
    asterisk_fill: bool,
    forced_sign: bool,
}

/// Metadatos de un array declarado con `DIM`, usados por
/// `gen_lvalue_address` para calcular la dirección real de un elemento.
#[derive(Debug, Clone, Copy)]
struct ArrayMeta {
    base_addr: usize,
    /// Bytes por elemento. Para arrays numéricos, 1 (coincide con el
    /// único ancho que el resto del backend maneja de forma fiable hoy vía
    /// `ApilaInd`/`DesapilaInd`); para arrays de cadena, el `*N` de `DIM`
    /// si es una constante conocida.
    element_size: usize,
    dims: ArrayDims,
    /// `DIM A(N)` con `N` una expresión NO constante (p.ej. `DIM B$(R)*1`
    /// con `R` variable, patrón real de blackjack.bas): la dirección base
    /// real solo se conoce en tiempo de EJECUCIÓN (se reserva de un heap
    /// dinámico de tamaño fijo, `__ARRAY_HEAP`, avanzando
    /// `__ARRAY_HEAP_PTR`), así que `base_addr` de arriba no sirve — este
    /// campo guarda la dirección del "descriptor" (una variable normal de
    /// 2 bytes) donde el código generado por `gen_dim` deja la base real
    /// en tiempo de ejecución. `None` para el caso normal (tamaño
    /// constante, `base_addr` ya es la dirección real).
    dynamic_base_descriptor: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum ArrayDims {
    OneD { len: usize },
    TwoD { rows: usize, cols: usize },
}

/// Área de datos de usuario en memoria (variables escalares y, más
/// adelante, el heap de arrays).
///
/// Mapa de memoria real (PC-1500 **con expansión de RAM CE-155**, 8KB,
/// confirmada contra el manual real — ver `codegen::system_memory` y el
/// doc de `codegen::lh5801_backend`): `STANDARD_USER_MEMORY` mapea
/// `0x3800-0x5FFF` (10240 bytes), repartido en ventanas disjuntas: código
/// en `start_address..` (`Lh5801Backend::start_address`), variables desde
/// `data_base` (ver [`compile_native_two_pass`]), pila propia desde
/// `0x5FFF` hacia abajo (`Lh5801Backend::stack_top`).
///
/// Este valor era antes una constante fija (primero `0x5000`, luego
/// `0x5600` tras un incidente ya documentado en el historial: con
/// bathyscaph.bas compilado por completo el código creció a 4579 bytes,
/// solapándose con `0x5000`, y las variables se escribían encima de código
/// todavía no ejecutado, corrompiéndolo en tiempo de ejecución). Subir la
/// constante "a mano" solo pospone el mismo choque para el siguiente
/// programa que crezca lo suficiente — y volvió a pasar: con INKEY$, el
/// `RND` de 16 bits y el nuevo `GPRINT` de cadenas ya en el compilador,
/// bathyscaph.bas creció a 5546 bytes, invadiendo de nuevo `0x5600`
/// (variables como `S`/`R`/`Q` leían bytes del propio código/pool de datos
/// en vez de basura-cero, visible como una barra sólida en las primeras
/// columnas del display). Sustituido por [`compile_native_two_pass`], que
/// calcula `data_base` dinámicamente a partir del tamaño real del código
/// generado, eliminando la clase de bug entera en vez de solo este caso.
///
/// El valor de abajo solo sobrevive como *placeholder* para `new()` (usado
/// por el camino `--stack-code`, que nunca llega a un backend real ni le
/// importa dónde caigan las direcciones) y como dirección de la primera
/// pasada de [`compile_native_two_pass`], cuyo único propósito es medir
/// `code.len()` — el valor numérico elegido aquí no afecta ese tamaño.
const DEFAULT_DATA_BASE_PLACEHOLDER: usize = 0x5600;

/// Longitud máxima genérica para operaciones de cadena
/// (`MID$`/`LEFT$`/`RIGHT$`/`LEN`/`VAL`) sobre una fuente sin ancho
/// conocido en tiempo de compilación (variable escalar, literal). Fija
/// también el tamaño de los buffers de resultado compartidos de
/// `LEFT$`/`RIGHT$`/`MID$` (`+1` para el NUL) — ver
/// `string_source_max_len`.
const DEFAULT_STRING_MAX_LEN: usize = 40;

/// Contexto de un bucle FOR
#[derive(Clone)]
struct ForContext {
    /// Representación textual (`lvalue.show(false)`) de la variable de
    /// control, p.ej. "I" — usada por `gen_next` para emparejar por
    /// nombre en vez de asumir que el `NEXT` más reciente en orden de
    /// código fuente es siempre el correcto (ver comentario de
    /// `gen_next`).
    variable_name: String,
    loop_start: String,
    loop_end: String,
    /// Dirección de scratch donde se guarda el valor de STEP, evaluado una
    /// única vez en `gen_for` (STEP puede ser una expresión arbitraria, no
    /// solo un literal).
    step_addr: usize,
    /// `true` en cuanto algún `NEXT` de esta variable ya se ha procesado
    /// — ver el comentario largo en `gen_next` sobre por qué esto NO
    /// puede simplemente sacarse (`pop`) de `for_stack` al primer
    /// `NEXT`: un `FOR` puede cerrarse desde más de un `NEXT` distinto
    /// en el código fuente (ramas de control de flujo distintas que
    /// convergen en el mismo bucle), y todos deben poder reencontrar
    /// este mismo contexto. Este flag es lo que permite distinguir, más
    /// tarde, cuando un `NEXT` de un bucle EXTERIOR se encuentra este
    /// contexto todavía en la pila: si ya está cerrado, no hay que
    /// volver a emitir su etiqueta `loop_end` (sería una duplicada, el
    /// bug real que esto arregla); si no lo está, es un bucle interior
    /// genuinamente huérfano (nunca llegó a su propio `NEXT`) y sí hace
    /// falta definir su etiqueta para que el salto de salida de ESE
    /// bucle tenga dónde aterrizar.
    closed: bool,
}

impl StackCodeGenerator {
    pub fn new() -> Self {
        Self::with_data_base(DEFAULT_DATA_BASE_PLACEHOLDER)
    }

    /// Como `new()`, pero fijando explícitamente dónde empieza el área de
    /// variables — ver [`compile_native_two_pass`], que es quien de verdad
    /// calcula ese valor para código nativo.
    pub fn with_data_base(data_base: usize) -> Self {
        Self::with_data_base_and_timing(data_base, false)
    }

    /// Como `with_data_base`, pero además controla el mecanismo genérico
    /// de ritmo de ejecución (ver el comentario de `authentic_timing`).
    /// `with_data_base`/`new()` siguen existiendo y llaman aquí con
    /// `authentic_timing=false`, así que ningún código ni test previo a
    /// la existencia de este parámetro cambia de comportamiento.
    pub fn with_data_base_and_timing(data_base: usize, authentic_timing: bool) -> Self {
        Self {
            instructions: Vec::new(),
            label_counter: 0,
            symbol_table: HashMap::new(),
            variable_addresses: HashMap::new(),
            next_address: 0,
            for_stack: Vec::new(),
            array_metadata: HashMap::new(),
            data_pool: Vec::new(),
            data_line_index: HashMap::new(),
            data_base,
            real_variables: std::collections::HashSet::new(),
            word_variables: std::collections::HashSet::new(),
            authentic_timing,
            current_using_format: None,
            sqr_used: false,
            dynamic_array_heap_initialized: false,
            all_string_labels: Vec::new(),
        }
    }

    /// Generar código para el programa completo
    /// gen_cod(prog(Bloq)):
    ///   gen_cod(Bloq)
    ///   emit stop()
    pub fn generate(&mut self, program: &Program) -> Vec<StackInstruction> {
        self.instructions.clear();
        self.label_counter = 0;

        // Pre-pasada: recoger todos los DATA del programa completo, en
        // orden de número de línea. DATA no es código ejecutable en BASIC
        // real (el intérprete lo salta), así que esto tiene que resolverse
        // antes de generar nada, no en el punto del flujo de control donde
        // aparece cada DATA (que podría ni ejecutarse nunca, p.ej. si está
        // después de un END o en una rama no tomada).
        self.collect_data_pool(program);
        self.collect_string_labels(program);

        // Pre-pasada: qué variables escalares son reales (ver el
        // comentario de `real_variables`) — antes de generar nada, porque
        // una variable puede leerse (p.ej. en un `PRINT` o una condición)
        // antes de la línea donde se le asigna por primera vez un valor
        // real, y `gen_acc_val`/`gen_binary_op` necesitan saberlo desde el
        // principio para elegir `ApilaIndReal` en vez de `ApilaInd`.
        self.collect_real_variables(program);
        // Pre-pasada análoga para variables "de palabra" (ver el
        // comentario de `word_variables`) — DESPUÉS de `collect_real_variables`,
        // para que una variable ya marcada real (que subsume cualquier
        // rango entero) nunca se marque también "de palabra".
        self.collect_word_variables(program);
        if !self.data_pool.is_empty() {
            self.emit(StackInstruction::DataPool(self.data_pool.clone()));
            let mut table: Vec<(u16, usize)> = self.data_line_index.iter().map(|(&k, &v)| (k, v)).collect();
            table.sort_by_key(|(line, _)| *line);
            self.emit(StackInstruction::DataLineTable(table));
        }

        // Tabla completa de números de línea, para GOTO/GOSUB calculado
        // (ver `gen_dynamic_line_number`) — barata de recoger siempre
        // (son enteros de 16 bits, nada de memoria de programa real) y
        // solo genera bytes si algún GOTO/GOSUB dinámico la usa de
        // verdad, así que no hace falta detectar antes si el programa la
        // necesita.
        let mut line_numbers: Vec<u16> = program.lines().map(|l| l.number()).collect();
        line_numbers.sort_unstable();
        self.emit(StackInstruction::LineTable(line_numbers));

        // `__RND_SEED` (el estado del LFSR mock de `RND`/`RANDOM`, ver
        // `FunctionInner::Rnd`/`StatementInner::Random`) a un valor fijo
        // (1) SIEMPRE, incondicionalmente, al arrancar el programa — no
        // solo si CallRnd/Random lo detectan a 0. Motivo: `load_lh5_file`
        // en el emulador solo sobrescribe los bytes del código al cargar
        // un `.lh5`, nunca limpia la región de variables — así que cargar
        // el MISMO programa dos veces seguidas dentro de la misma sesión
        // del emulador (sin reiniciar la app) deja `__RND_SEED` con el
        // valor donde lo dejó la partida anterior, y el mapa/la partida
        // sale distinta cada vez. Encontrado con bathyscaph.bas (línea 19:
        // `RESTORE 999+RND 16`, nunca llama a `RANDOM`): en la ROM real,
        // el programa tokenizado original muestra SIEMPRE el mismo mapa
        // en cada `RUN` — la propia ROM debe resetear su generador
        // pseudoaleatorio interno como parte de arrancar la ejecución,
        // no solo en un arranque en frío de la calculadora. Esto replica
        // ese mismo comportamiento en vez de depender de que la memoria
        // ya esté a cero. Se reserva la dirección siempre (no solo si el
        // programa usa RND) porque detectar "¿se usa RND en algún punto
        // del programa?" exigiría recorrer recursivamente cada tipo de
        // sentencia/expresión — unos pocos bytes de coste fijo en todos
        // los programas es más simple y más difícil de dejarse un caso
        // por el camino que ese recorrido.
        let rnd_seed_addr = self.get_or_create_variable_address("__RND_SEED");
        self.emit(StackInstruction::ApilaInt(rnd_seed_addr as i64)); // dirección primero (modelo Tiny)
        self.emit(StackInstruction::ApilaInt(1));
        self.emit(StackInstruction::DesapilaInd);

        // Comentario inicial
        self.emit_comment("=== INICIO DEL PROGRAMA ===");
        self.emit_comment("");

        // Generar código para cada línea del programa
        for line in program.lines() {
            self.gen_code_line(line);
        }

        // Fin del programa
        self.emit(StackInstruction::Stop);

        // Subrutina compartida de `SQR` (ver `FunctionInner::Sqr`), si
        // algún punto del programa la usó — emitida una sola vez, después
        // del `Stop` (nunca se cae en ella por flujo normal, solo se
        // alcanza vía el `Call` que ya emitió cada punto de llamada).
        if self.sqr_used {
            self.gen_sqr_routine();
        }

        self.instructions.clone()
    }

    /// Cuerpo de `__SQR_ROUTINE`: 15 vueltas de Newton
    /// (`x = (x + v/x) / 2`) sobre `__SQR_V`/`__SQR_X`, como un bucle real
    /// en tiempo de ejecución (contador `__SQR_I`) — no desenrollado, para
    /// no multiplicar el tamaño de código por cada llamada a `SQR` del
    /// programa (ver el comentario en `FunctionInner::Sqr`). 15 iteraciones
    /// es holgado: la convergencia de Newton es cuadrática una vez cerca
    /// de la raíz, y hasta con una estimación inicial mala (`x_0=(v+1)/2`,
    /// mala para `v` grande) hay margen de sobra dentro de la precisión de
    /// 12 dígitos BCD del formato real de esta calculadora.
    fn gen_sqr_routine(&mut self) {
        let i_addr = self.get_or_create_variable_address("__SQR_I");
        let v_addr = self.get_or_create_array_address("__SQR_V", 8);
        let x_addr = self.get_or_create_array_address("__SQR_X", 8);

        let loop_start = self.new_label("SQR_LOOP");
        let loop_end = self.new_label("SQR_FIN");
        let zero_case = self.new_label("SQR_ZERO");

        self.emit(StackInstruction::Label("__SQR_ROUTINE".to_string()));

        // Caso especial v=0: Newton nunca alcanza exactamente 0 partiendo
        // de x_0=(v+1)/2 (para v=0, x_0=0.5, y cada vuelta solo lo divide
        // a la mitad: x=0.5/2^15 ≈ 0.000015, nunca 0 en un nº finito de
        // iteraciones) — visible como un residuo pequeño pero no-cero en
        // `SQR(0)`, confirmado con un test aislado antes de este arreglo.
        self.emit(StackInstruction::ApilaInt(v_addr as i64));
        self.emit(StackInstruction::ApilaIndReal);
        self.emit(StackInstruction::ApilaReal(0.0));
        self.emit(StackInstruction::IgualReal);
        self.emit(StackInstruction::IrV(zero_case.clone()));

        // i = 15
        self.emit(StackInstruction::ApilaInt(i_addr as i64));
        self.emit(StackInstruction::ApilaInt(15));
        self.emit(StackInstruction::DesapilaInd);

        self.emit(StackInstruction::Label(loop_start.clone()));

        // if i == 0 goto fin
        self.emit(StackInstruction::ApilaInt(i_addr as i64));
        self.emit(StackInstruction::ApilaInd);
        self.emit(StackInstruction::ApilaInt(0));
        self.emit(StackInstruction::IgualInt);
        self.emit(StackInstruction::IrV(loop_end.clone()));

        // x = (x + v/x) / 2
        self.emit(StackInstruction::ApilaInt(x_addr as i64));
        self.emit(StackInstruction::ApilaInt(v_addr as i64));
        self.emit(StackInstruction::ApilaIndReal);
        self.emit(StackInstruction::ApilaInt(x_addr as i64));
        self.emit(StackInstruction::ApilaIndReal);
        self.emit(StackInstruction::DivReal);
        self.emit(StackInstruction::ApilaInt(x_addr as i64));
        self.emit(StackInstruction::ApilaIndReal);
        self.emit(StackInstruction::SumaReal);
        self.emit(StackInstruction::ApilaReal(2.0));
        self.emit(StackInstruction::DivReal);
        self.emit(StackInstruction::DesapilaIndReal);

        // i = i - 1
        self.emit(StackInstruction::ApilaInt(i_addr as i64));
        self.emit(StackInstruction::ApilaInt(i_addr as i64));
        self.emit(StackInstruction::ApilaInd);
        self.emit(StackInstruction::ApilaInt(1));
        self.emit(StackInstruction::RestaInt);
        self.emit(StackInstruction::DesapilaInd);

        self.emit(StackInstruction::IrA(loop_start));

        self.emit(StackInstruction::Label(loop_end));
        self.emit(StackInstruction::IrInd);

        self.emit(StackInstruction::Label(zero_case));
        self.emit(StackInstruction::ApilaInt(x_addr as i64));
        self.emit(StackInstruction::ApilaReal(0.0));
        self.emit(StackInstruction::DesapilaIndReal);
        self.emit(StackInstruction::IrInd);
    }

    /// Recorre todo el programa (en orden de línea) recogiendo los valores
    /// de cada `DATA` en `self.data_pool`, y para cada línea con `DATA`
    /// guarda en `self.data_line_index` el índice de su primer valor (para
    /// `RESTORE <línea>`). Solo valores de tipo cadena por ahora — otros
    /// tipos de expresión en DATA quedan documentados como no soportados
    /// (se guarda una cadena vacía en su lugar) en vez de fallar.
    fn collect_data_pool(&mut self, program: &Program) {
        for line in program.lines() {
            for stmt in line.statements() {
                if let StatementInner::Data(exprs) = &stmt.inner {
                    for (i, expr) in exprs.iter().enumerate() {
                        if i == 0 {
                            self.data_line_index.entry(line.number()).or_insert(self.data_pool.len());
                        }
                        match expr.inner() {
                            ExprInner::StringLiteral { value, .. } => {
                                self.data_pool.push(value.clone());
                            }
                            _ => {
                                self.data_pool.push(String::new());
                            }
                        }
                    }
                }
            }
        }
    }
    
    /// Recoge todas las etiquetas de usuario (`line.label()`) del
    /// programa en `self.all_string_labels` — ver el comentario del campo.
    fn collect_string_labels(&mut self, program: &Program) {
        for line in program.lines() {
            if let Some(label) = line.label() {
                self.all_string_labels.push(label.to_string());
            }
        }
    }

    /// Recorre todo el programa (dos veces: ver más abajo) para decidir
    /// qué variables escalares deben tratarse como reales — ver el
    /// comentario del campo `real_variables`.
    ///
    /// Primero recoge el conjunto de variables usadas alguna vez como
    /// variable de control de un `FOR` (esas quedan excluidas siempre,
    /// FOR/NEXT no soporta reales). Luego recorre cada `LET` del programa
    /// (incluyendo los anidados dentro de un `IF ... THEN <stmt>`) y, si el
    /// lado derecho es una expresión real (`is_real_expr`) y el destino es
    /// una variable escalar no excluida, la marca como real.
    ///
    /// Esto se repite hasta un punto fijo (ninguna variable nueva marcada
    /// en una vuelta completa) porque `is_real_expr` de una variable ya
    /// depende de `real_variables` (para que `X=B` propague la realidad de
    /// `B` a `X` si `B` ya se marcó real), y el orden de las sentencias en
    /// el programa no tiene por qué coincidir con el orden en que hace
    /// falta descubrirlas (p.ej. si la asignación que revela que `B` es
    /// real aparece en una línea posterior a otra que ya usa `B`). El
    /// número de variables reales de un programa real es pequeño, así que
    /// esto converge en pocas vueltas.
    fn collect_real_variables(&mut self, program: &Program) {
        let mut for_control_vars = std::collections::HashSet::new();
        for line in program.lines() {
            for stmt in line.statements() {
                self.collect_for_control_vars(stmt, &mut for_control_vars);
            }
        }

        loop {
            let mut changed = false;
            for line in program.lines() {
                for stmt in line.statements() {
                    self.collect_real_variables_from_stmt(stmt, &for_control_vars, &mut changed);
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn collect_for_control_vars(&self, stmt: &Statement, out: &mut std::collections::HashSet<String>) {
        match &stmt.inner {
            StatementInner::For { assignment, .. } => {
                if let LValueInner::Identifier(id) = &assignment.lvalue().inner {
                    out.insert(id.to_string());
                }
            }
            StatementInner::If { then_stmt, .. } => self.collect_for_control_vars(then_stmt, out),
            _ => {}
        }
    }

    fn collect_real_variables_from_stmt(
        &mut self,
        stmt: &Statement,
        for_control_vars: &std::collections::HashSet<String>,
        changed: &mut bool,
    ) {
        match &stmt.inner {
            StatementInner::Let { inner, .. } => {
                for assignment in inner.assignments() {
                    if let LValueInner::Identifier(id) = &assignment.lvalue().inner {
                        let name = id.to_string();
                        if !id.has_dollar()
                            && !for_control_vars.contains(&name)
                            && !self.real_variables.contains(&name)
                            && self.is_real_expr(assignment.expr())
                        {
                            self.real_variables.insert(name);
                            *changed = true;
                        }
                    }
                }
            }
            StatementInner::If { then_stmt, .. } => {
                self.collect_real_variables_from_stmt(then_stmt, for_control_vars, changed);
            }
            _ => {}
        }
    }

    /// Análogo a `collect_real_variables` para variables "de palabra"
    /// (ver el comentario de `word_variables`) — mismo algoritmo de
    /// punto fijo (una variable puede necesitar 16 bits por una
    /// asignación que a su vez depende de OTRA variable ya marcada, p.ej.
    /// `C=299` y luego `D=C` en otra línea), reutilizando el mismo
    /// conjunto de variables de control de `FOR` ya recogido para reales
    /// (misma exclusión, mismo motivo).
    fn collect_word_variables(&mut self, program: &Program) {
        let mut for_control_vars = std::collections::HashSet::new();
        for line in program.lines() {
            for stmt in line.statements() {
                self.collect_for_control_vars(stmt, &mut for_control_vars);
            }
        }

        loop {
            let mut changed = false;
            for line in program.lines() {
                for stmt in line.statements() {
                    self.collect_word_variables_from_stmt(stmt, &for_control_vars, &mut changed);
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn collect_word_variables_from_stmt(
        &mut self,
        stmt: &Statement,
        for_control_vars: &std::collections::HashSet<String>,
        changed: &mut bool,
    ) {
        match &stmt.inner {
            StatementInner::Let { inner, .. } => {
                for assignment in inner.assignments() {
                    if let LValueInner::Identifier(id) = &assignment.lvalue().inner {
                        let name = id.to_string();
                        if !id.has_dollar()
                            && !for_control_vars.contains(&name)
                            && !self.real_variables.contains(&name)
                            && !self.word_variables.contains(&name)
                            && self.is_word_expr(assignment.expr())
                        {
                            self.word_variables.insert(name);
                            *changed = true;
                        }
                    }
                }
            }
            StatementInner::If { then_stmt, .. } => {
                self.collect_word_variables_from_stmt(then_stmt, for_control_vars, changed);
            }
            _ => {}
        }
    }

    /// Generar código para una línea de código
    /// Cada CodeLine tiene un número de línea y opcionalmente una etiqueta de usuario
    fn gen_code_line(&mut self, line: &CodeLine) {
        // Generar etiqueta para el número de línea
        let line_label = format!("LINE_{}", line.number());
        self.emit(StackInstruction::Label(line_label));
        
        // Si hay etiqueta de usuario, también la generamos
        if let Some(user_label) = line.label() {
            self.emit(StackInstruction::Label(user_label.to_string()));
        }
        
        // Comentario con el número de línea
        self.emit_comment(&format!("Línea {}", line.number()));
        
        // Generar código para cada sentencia en la línea
        for stmt in line.statements() {
            self.gen_statement_timed(stmt);
        }

        self.emit_comment("");
    }

    /// Como `gen_statement`, pero además emite la espera calibrada del
    /// mecanismo genérico de ritmo (`AuthenticTimingDelay`, ver el
    /// comentario de `authentic_timing`) justo después, cuando está
    /// activo — usado en los dos sitios donde de verdad se ejecuta una
    /// sentencia BASIC completa por derecho propio: el bucle de
    /// `gen_code_line` (una línea normal) y el de `StatementInner::Multi`
    /// (el consecuente de varias sentencias de un `IF...THEN a:b:c`).
    /// Deliberadamente NO se llama desde dentro de `gen_statement` en sí
    /// (p.ej. el cuerpo de un `IF` de una sola sentencia ya pasa por uno
    /// de estos dos sitios) — evita duplicar la espera para la misma
    /// sentencia lógica.
    fn gen_statement_timed(&mut self, stmt: &Statement) {
        self.gen_statement(stmt);
        if self.authentic_timing {
            self.emit(StackInstruction::AuthenticTimingDelay);
        }
    }
    
    /// Generar código para una sentencia
    /// Dispatcher principal para todas las sentencias del lenguaje
    fn gen_statement(&mut self, stmt: &Statement) {
        match &stmt.inner {
            // === ASIGNACIÓN ===
            StatementInner::Let { inner, .. } => self.gen_let(inner),
            
            // === ENTRADA/SALIDA ===
            StatementInner::Print { inner } => self.gen_print(inner),
            StatementInner::Pause { inner } => self.gen_pause(inner),
            StatementInner::LPrint { inner } => self.gen_lprint(inner),
            StatementInner::Input { input_exprs } => self.gen_input(input_exprs),
            StatementInner::Using { using_clause } => self.apply_using_clause(using_clause),
            
            // === CONTROL DE FLUJO ===
            StatementInner::If { condition, then_stmt, .. } => 
                self.gen_if(condition, then_stmt),
            StatementInner::Goto { target } => self.gen_goto(target),
            StatementInner::Gosub { target } => self.gen_gosub(target),
            StatementInner::Return => self.gen_return(),
            StatementInner::For { assignment, to_expr, step_expr } => 
                self.gen_for(assignment, to_expr, step_expr),
            StatementInner::Next { lvalue } => self.gen_next(lvalue),
            StatementInner::OnGoto { expr, targets } => self.gen_on_goto(expr, targets),
            StatementInner::OnGosub { expr, targets } => self.gen_on_gosub(expr, targets),
            StatementInner::OnErrorGoto { target } => self.gen_on_error_goto(target),
            
            // === DATOS ===
            StatementInner::Dim { decls } => self.gen_dim(decls),
            StatementInner::Read { destinations } => self.gen_read(destinations),
            StatementInner::Data(exprs) => self.gen_data(exprs),
            StatementInner::Restore { expr } => self.gen_restore(expr),
            
            // === SISTEMA ===
            StatementInner::End => self.gen_end(),
            StatementInner::Clear => self.gen_clear(),
            StatementInner::Wait { expr } => self.gen_wait(expr),
            StatementInner::Random => self.gen_random(),
            StatementInner::Arun => self.gen_arun(),
            StatementInner::Lock => self.gen_lock(),
            StatementInner::Unlock => self.gen_unlock(),
            
            // === GRÁFICOS Y CURSOR ===
            StatementInner::Gprint { exprs } => self.gen_gprint(exprs),
            StatementInner::GCursor { expr } => self.gen_gcursor(expr),
            StatementInner::Cursor { expr } => self.gen_cursor(expr),
            StatementInner::LCursor(clause) => self.gen_lcursor(clause),
            StatementInner::GlCursor { x_expr, y_expr } => self.gen_glcursor(x_expr, y_expr),
            StatementInner::Line { inner } => self.gen_line(inner),
            StatementInner::RLine { inner } => self.gen_rline(inner),
            StatementInner::Sorgn => self.gen_sorgn(),
            StatementInner::Rotate { expr } => self.gen_rotate(expr),
            StatementInner::Text => self.gen_text(),
            StatementInner::Graph => self.gen_graph(),
            StatementInner::Color { expr } => self.gen_color(expr),
            StatementInner::CSize { expr } => self.gen_csize(expr),
            
            // === SONIDO ===
            StatementInner::Beep { repetitions_expr, optional_params } => 
                self.gen_beep(repetitions_expr, optional_params),
            StatementInner::BeepOnOff { switch_beep_on } => 
                self.gen_beep_onoff(*switch_beep_on),
            
            // === MEMORIA Y LLAMADAS ===
            StatementInner::Poke { memory_area, exprs } => self.gen_poke(memory_area, exprs),
            StatementInner::Call { expr, variable } => self.gen_call(expr, variable),
            
            // === MATEMÁTICAS ===
            StatementInner::Radian => self.gen_radian(),
            StatementInner::Degree => self.gen_degree(),
            
            // === OTROS ===
            StatementInner::Lf { expr } => self.gen_lf(expr),
            StatementInner::Cls => self.gen_cls(),
            
            // === COMENTARIOS ===
            StatementInner::Remark { text } => {
                self.emit_comment(&format!("REM: {}", text));
            }

            // Ver comentario de `StatementInner::Multi`: todas las
            // sentencias separadas por ':' que forman el consecuente de
            // un IF sin bloque explícito. Cada una es una sentencia BASIC
            // real y ejecutable por derecho propio (el intérprete real las
            // despacha una a una igual que si estuvieran en `gen_code_line`),
            // así que usa `gen_statement_timed` igual que ahí — no
            // `gen_statement` a secas, que se saltaría la espera del
            // mecanismo de ritmo (ver `authentic_timing`) para todo lo que
            // cuelga de un `THEN` con varias sentencias.
            StatementInner::Multi(statements) => {
                for statement in statements {
                    self.gen_statement_timed(statement);
                }
            }
        }
    }
    
    // =========================================================================
    // GENERACIÓN DE CÓDIGO PARA SENTENCIAS
    // =========================================================================
    
    /// gen_cod(asig(lvalue, expr)):
    ///   gen_cod(lvalue)         // Genera dirección
    ///   gen_cod(expr)           // Genera valor
    ///   gen_asig(lvalue, expr)  // Realiza asignación
    fn gen_let(&mut self, let_inner: &LetInner) {
        // Generar comentario explicativo
        self.emit_comment(&format!("LET {}", let_inner.show_with_context(true, false)));
        
        // Almacenar en todas las variables del lado izquierdo
        // (en BASIC se permite: LET A = B = C = 5)
        for assignment in let_inner.assignments() {
            let lvalue = assignment.lvalue();
            let expr = assignment.expr();
            
            // Apilar dirección primero (modelo Tiny)
            self.gen_lvalue_address(lvalue);
            
            // Evaluar la expresión del lado derecho
            self.gen_expression(expr);
            self.gen_acc_val(expr);

            // Si el destino es una variable marcada real
            // (`collect_real_variables`) pero la expresión del lado
            // derecho no lo es (p.ej. `B=0` tras haber marcado `B` real
            // por `B=B+.5` en otra línea), promocionar aquí el valor de 1
            // byte recién apilado a 8 bytes — sin esto, `gen_store_to_lvalue`
            // emitiría `DesapilaIndReal` (que hace pop de 8 bytes de valor)
            // sobre un valor de 1 byte, robando 7 bytes de la pila que no
            // le pertenecen (mismo patrón de fuga que motivó
            // `DesapilaIndReal` en primer lugar, aquí en el lado de la
            // promoción de tipo en vez del formato de la operación).
            if self.is_real_lvalue(lvalue) && !self.is_real_expr(expr) {
                self.emit(StackInstruction::Int2Real);
            } else if self.is_word_lvalue(lvalue) && !self.is_word_expr(expr) {
                // Mismo patrón que la promoción real de arriba, para una
                // variable "de palabra" (ver `word_variables`): si el
                // lado derecho no produce ya un valor de 16 bits (p.ej.
                // `C=0` tras haberse marcado `C` "de palabra" por
                // `C=299` en otra línea), extenderlo aquí antes de
                // `gen_store_to_lvalue`, que emitirá `DesapilaIndWord`
                // (2 bytes) para esta variable.
                self.emit(StackInstruction::ExtendIntToWord);
            }

            // Almacenar: desapila valor, desapila dirección, guarda
            self.gen_store_to_lvalue(lvalue);
        }
    }
    
    /// gen_cod(ins_write(Exp)):
    ///   gen_cod(Exp)
    ///   gen_acc_val(Exp)
    ///   emit systemout()
    fn gen_print(&mut self, print_inner: &PrintInner) {
        // La ROM real comparte la MISMA rutina de bajo nivel entre `PRINT`
        // y `PAUSE` (`BCMD_PRINT` hace `JMP BCMD_PAUSE_2` en su rama de
        // salida a display, confirmado en el desensamblado) — así que un
        // `PRINT` "plano" (sin `USING` activo) también limpia la pantalla
        // entera antes de escribir, igual que ya se arregló para `PAUSE`
        // (ver el comentario largo de `gen_pause`). Confirmado
        // empíricamente comparando la pantalla final de `bombing.bas`
        // (`PRINT "*** SCORE *** :";W`, línea 310) contra el programa
        // original tokenizado: el original la muestra sobre fondo
        // limpio, la nuestra (antes de este fix) dejaba visibles los
        // restos de la ciudad y del marcador antiguo.
        //
        // Un `PRINT USING <patrón>;...` NO debe limpiar — confirmado por
        // el propio comportamiento observado del juego: el marcador
        // (`CURSOR 22:PRINT USING "####";W`, líneas 260/280) se actualiza
        // repetidas veces DURANTE la partida y permanece visible sobre
        // el resto de la pantalla (ciudad, avión) en todo momento; si
        // este `PRINT USING` limpiase la pantalla cada vez, borraría el
        // juego entero en cada acierto. `PRINT USING` es, de hecho, un
        // token BASIC distinto de `PRINT` en la ROM real (con su propia
        // rutina de formateo), así que no comparte la ruta de
        // `BCMD_PRINT`/`BCMD_PAUSE` en absoluto — de ahí que el `Cls` de
        // aquí deba omitirse tanto si ya hay un formato `USING` activo
        // (heredado de una sentencia `USING "patrón"` suelta anterior)
        // como si este propio `PRINT` trae su propia cláusula `USING`
        // incrustada (`PRINT USING "####";W` en una sola sentencia).
        let has_using = self.current_using_format.is_some()
            || print_inner.exprs.iter().any(|(printable, _)| {
                matches!(printable, crate::parse::statement::printable::Printable::UsingClause(_))
            });
        if !has_using {
            // No es el `Cls` incondicional (LCD_CLR+INIT_CURS): la ROM
            // real llama a `CLR_NO_CURSOR`, que respeta un `CURSOR n`
            // que acabe de posicionar el cursor (bug real de
            // invader-v2.bas — ver el comentario largo de
            // `StackInstruction::ClsIfNoCursor`).
            self.emit(StackInstruction::ClsIfNoCursor);
        }

        for (printable, sep) in &print_inner.exprs {
            // Generar código para el elemento a imprimir
            match printable {
                crate::parse::statement::printable::Printable::Expr(expr) => {
                    self.gen_expression(expr);
                    self.gen_acc_val(expr);
                    if self.is_string_expr(expr) {
                        self.emit(StackInstruction::SystemOutString);
                    } else if let Some(fmt) = self.current_using_format {
                        // PRINT USING activo: el valor tiene que estar en
                        // la pila como real de 8 bytes (mismo criterio de
                        // promoción que gen_binary_op/gen_let) antes de
                        // formatearlo con ancho fijo.
                        if !self.is_real_expr(expr) {
                            self.emit(StackInstruction::Int2Real);
                        }
                        let buf_len = 1 // signo
                            + fmt.digits_before as usize
                            + if fmt.digits_after > 0 { 1 + fmt.digits_after as usize } else { 0 }
                            + 1; // NUL
                        let buf = self.get_or_create_array_address("__USING_BUF", buf_len);
                        self.emit(StackInstruction::PrintUsingReal(
                            fmt.digits_before, fmt.digits_after, fmt.asterisk_fill, fmt.forced_sign, buf,
                        ));
                    } else if self.is_real_expr(expr) {
                        // Sin USING activo: variable/expresión real
                        // impresa "a pelo". Antes esto caía en
                        // SystemOutInt (que solo consume 1 byte de los 8
                        // que empuja una variable real), perdiendo 7
                        // bytes de pila en cada PRINT — confirmado contra
                        // la ROM real investigando USING. Formato ancho
                        // fijo generoso (7 enteros + 6 decimales) con
                        // recorte de ceros/espacios sobrantes al imprimir
                        // — ver PrintRealNatural en el backend.
                        let buf = self.get_or_create_array_address("__PRINT_REAL_BUF", 16);
                        self.emit(StackInstruction::PrintRealNatural(buf));
                    } else if self.is_word_expr(expr) {
                        // Variable "de palabra" (ver `word_variables`,
                        // p.ej. `S` tras `S=S+5000` en invader-v2.bas):
                        // `gen_acc_val` ya la cargó como 16 bits —
                        // `SystemOutInt` (que solo desapila 1 byte)
                        // desincronizaría la pila.
                        self.emit(StackInstruction::SystemOutIntWord);
                    } else {
                        self.emit(StackInstruction::SystemOutInt);
                    }
                }
                crate::parse::statement::printable::Printable::UsingClause(using) => {
                    self.apply_using_clause(using);
                }
            }

            // Generar código para el separador
            match sep {
                PrintSeparator::Comma => {
                    // Tabulador (espacio fijo)
                    self.emit(StackInstruction::PrintTab);
                }
                PrintSeparator::Semicolon => {
                    // Sin separación adicional
                }
                PrintSeparator::None => {
                    // Nueva línea
                    self.emit(StackInstruction::Newline);
                }
            }
        }
    }
    
    /// gen_cod(ins_read(Exp)):
    ///   gen_cod(Exp)        // Dirección de la variable
    ///   emit systemin()     // Leer entrada
    ///   emit desapila-ind() // Almacenar en la dirección
    fn gen_input(&mut self, input_exprs: &[(Option<Expr>, LValue)]) {
        for (prompt, lvalue) in input_exprs {
            // Si hay prompt, imprimirlo primero
            if let Some(prompt_expr) = prompt {
                self.gen_expression(prompt_expr);
                self.gen_acc_val(prompt_expr);
                if self.is_string_expr(prompt_expr) {
                    self.emit(StackInstruction::SystemOutString);
                } else if self.is_word_expr(prompt_expr) {
                    self.emit(StackInstruction::SystemOutIntWord);
                } else {
                    self.emit(StackInstruction::SystemOutInt);
                }
            }

            // Generar dirección de la variable
            self.gen_lvalue_address(lvalue);

            // Leer una línea de teclado: SystemIn siempre deja en la
            // pila un puntero (16 bits) a la línea leída, mismo convenio
            // que CallStr/CallChr (ver backend). Para un destino de
            // cadena ese puntero YA ES el valor a guardar (mismo
            // almacenamiento que cualquier otra asignación de cadena,
            // ver gen_store_to_lvalue); para un destino numérico hay que
            // convertirlo primero con la misma lógica que VAL().
            self.emit(StackInstruction::SystemIn);
            if self.is_string_lvalue(lvalue) {
                self.gen_store_to_lvalue(lvalue);
            } else {
                let scratch = self.get_or_create_array_address("__VAL_SCRATCH", 4);
                self.emit(StackInstruction::CallVal(system_memory::IN_BUF_LEN as usize, scratch));
                self.emit(StackInstruction::DesapilaInd);
            }
        }
    }
    
    /// gen_cod(ins_if(Exp, bloq)):
    ///   gen_cod(Exp)
    ///   gen_acc_val(Exp)
    ///   emit ir-f($.sig)
    ///   gen_cod(bloq)
    fn gen_if(&mut self, condition: &Expr, then_stmt: &Statement) {
        let end_label = self.new_label("FIN_IF");
        
        self.emit_comment("IF-THEN");
        
        // Evaluar condición
        self.gen_expression(condition);
        self.gen_acc_val(condition);
        
        // Si es falsa, saltar al final
        self.emit(StackInstruction::IrF(end_label.clone()));

        // Código del THEN. Si `then_stmt` es `Multi` (`IF cond THEN
        // a:b:c`), su propio manejador en `gen_statement` ya llama a
        // `gen_statement_timed` por cada sentencia interior — envolverlo
        // aquí OTRA vez con `gen_statement_timed` añadiría una espera
        // extra para el pseudo-nodo `Multi` en sí, que no es una
        // sentencia BASIC real. Para un THEN de una sola sentencia
        // (`IF cond THEN Y=1`, sin `Multi`), sí hace falta pasar por
        // `gen_statement_timed` aquí explícitamente — si no, esa
        // sentencia se quedaría sin su espera del mecanismo de ritmo.
        if matches!(&then_stmt.inner, StatementInner::Multi(_)) {
            self.gen_statement(then_stmt);
        } else {
            self.gen_statement_timed(then_stmt);
        }

        // Etiqueta de fin
        self.emit(StackInstruction::Label(end_label));
    }
    
    /// Generar código para GOTO
    fn gen_goto(&mut self, target: &Expr) {
        self.emit_comment("GOTO");

        match self.static_goto_label(target) {
            Some(label) => self.emit(StackInstruction::IrA(label)),
            None => match self.computed_string_goto_prefix(target) {
                Some((prefix, suffix)) => self.gen_computed_string_goto(&prefix, suffix, false),
                None => {
                    self.gen_dynamic_line_number(target);
                    self.emit(StackInstruction::IrIndirect);
                }
            },
        }
    }

    /// Generar código para GOSUB (llamada a subrutina)
    /// gen_cod(ins_call(Id, LParamsR)):
    ///   emit activa($.vinculo.nivel, $.vinculo.tam, $.sig)
    ///   emit ir-a($.vinculo.prim)
    fn gen_gosub(&mut self, target: &Expr) {
        self.emit_comment("GOSUB");

        match self.static_goto_label(target) {
            Some(label) => self.emit(StackInstruction::Call(label)),
            None => match self.computed_string_goto_prefix(target) {
                Some((prefix, suffix)) => self.gen_computed_string_goto(&prefix, suffix, true),
                None => {
                    self.gen_dynamic_line_number(target);
                    self.emit(StackInstruction::CallIndirect);
                }
            },
        }
    }

    /// Si `target` es un literal (número de línea o etiqueta de cadena),
    /// la etiqueta estática correspondiente — resolución normal en
    /// tiempo de compilación, la de siempre. `None` si `target` es una
    /// expresión genuinamente dinámica (variable, `<constante>+<expr>`,
    /// ...), que necesita el despacho indirecto de
    /// `gen_dynamic_line_number` + `IrIndirect`/`CallIndirect`.
    fn static_goto_label(&self, target: &Expr) -> Option<String> {
        match target.inner() {
            ExprInner::DecimalNumber(num) => Some(format!("LINE_{}", num.as_f64())),
            ExprInner::StringLiteral { value, .. } => Some(value.clone()),
            ExprInner::Parentheses(inner) => self.static_goto_label(inner),
            _ => None,
        }
    }

    /// Reconoce el patrón `<literal de cadena> + <expresión>` (en ese
    /// orden) — devuelve `(prefijo, &expresión)` si `target` tiene esa
    /// forma. Patrón real de invader-v2.bas: `GOTO "*"+INKEY$`, donde el
    /// conjunto de etiquetas posibles (`"*9"`, `"*="`, `"*"`, `"* "`, cada
    /// una definida en alguna línea del programa) comparte el prefijo
    /// constante `"*"` — ver `gen_computed_string_goto`.
    fn computed_string_goto_prefix<'e>(&self, target: &'e Expr) -> Option<(String, &'e Expr)> {
        match target.inner() {
            ExprInner::Parentheses(inner) => self.computed_string_goto_prefix(inner),
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                if let ExprInner::StringLiteral { value, .. } = left.inner() {
                    Some((value.clone(), right.as_ref()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `GOTO`/`GOSUB` a una etiqueta de cadena CALCULADA en tiempo de
    /// ejecución con un prefijo constante conocido (ver
    /// `computed_string_goto_prefix`). Sin ninguna instrucción de salto
    /// "por nombre calculado" real en el backend (ni la necesita: el
    /// conjunto de etiquetas posibles es siempre pequeño y enumerable en
    /// tiempo de COMPILACIÓN, buscando en `self.all_string_labels` cuáles
    /// empiezan por el prefijo), esto se resuelve como una cascada de
    /// comparaciones: para cada etiqueta candidata, compara el sufijo
    /// dinámico contra el sufijo literal de esa etiqueta concreta
    /// (`IgualCadena`, ya verificado por `IF A$=B$`) y salta/llama si
    /// coincide. Si ninguna candidata coincide en tiempo de ejecución
    /// (valor inesperado), el flujo sigue normalmente tras la última
    /// comprobación — comportamiento indefinido ante un valor sin
    /// etiqueta correspondiente, igual que ya asume el resto del backend
    /// para GOTO/GOSUB calculado (ver `gen_dynamic_line_number`).
    fn gen_computed_string_goto(&mut self, prefix: &str, suffix_expr: &Expr, is_call: bool) {
        self.emit_comment(&format!(
            "{} calculado con prefijo {:?}",
            if is_call { "GOSUB" } else { "GOTO" },
            prefix
        ));

        let mut candidates: Vec<String> = self
            .all_string_labels
            .iter()
            .filter(|label| label.starts_with(prefix))
            .cloned()
            .collect();
        candidates.sort();
        candidates.dedup();

        if candidates.is_empty() {
            eprintln!(
                "WARNING: {} a etiqueta de cadena calculada con prefijo {:?}, pero ninguna etiqueta del programa empieza por ese prefijo: ignorado",
                if is_call { "GOSUB" } else { "GOTO" },
                prefix
            );
            self.emit_comment("Sin etiquetas candidatas: ignorado");
            return;
        }

        let done_label = self.new_label("COMPUTED_GOTO_FIN");

        for candidate in &candidates {
            let suffix_literal = candidate[prefix.len()..].to_string();

            self.gen_expression(suffix_expr);
            self.gen_acc_val(suffix_expr);
            self.emit(StackInstruction::ApilaCadena(suffix_literal));
            self.emit(StackInstruction::IgualCadena);

            if is_call {
                let skip_label = self.new_label("COMPUTED_GOSUB_SKIP");
                self.emit(StackInstruction::IrF(skip_label.clone()));
                self.emit(StackInstruction::Call(candidate.clone()));
                self.emit(StackInstruction::IrA(done_label.clone()));
                self.emit(StackInstruction::Label(skip_label));
            } else {
                self.emit(StackInstruction::IrV(candidate.clone()));
            }
        }

        if is_call {
            self.emit(StackInstruction::Label(done_label));
        }
    }

    /// Genera código que deja en la pila el valor de `expr` como entero
    /// de 16 bits, para usarlo como número de línea calculado (`GOTO`/
    /// `GOSUB <expr>`, o `RESTORE <expr>` vía `gen_restore`). Reconoce
    /// `<constante>+<expresión>` (aritmética de 16 bits real, para bases
    /// de línea grandes con un desplazamiento pequeño, p.ej. `GOSUB
    /// C+10`) y, si no encaja ese patrón pero SÍ es una expresión "de
    /// palabra" (ver `word_variables` — p.ej. `RESTORE C+RND 3` en
    /// invader-v2.bas, con `C` una VARIABLE marcada "de palabra", no una
    /// constante: no encaja en el patrón de arriba, pero `gen_binary_op`
    /// ya sabe generarla como una suma de 16 bits genuina gracias a ese
    /// mismo mecanismo), la deja tal cual — `gen_expression` ya produce
    /// 16 bits reales. Si no es ninguna de las dos cosas, evalúa la
    /// expresión como entero normal de 8 bits y lo extiende con ceros a
    /// 16 bits (cubre el caso común de una variable con un número de
    /// línea pequeño, ≤255, p.ej. `GOTO D`).
    fn gen_dynamic_line_number(&mut self, expr: &Expr) {
        if let Some((base, dynamic_part)) = self.dynamic_line_number_base_and_offset(expr) {
            self.emit(StackInstruction::ApilaIntWord(base));
            self.gen_expression(dynamic_part);
            self.gen_acc_val(dynamic_part);
            self.emit(StackInstruction::SumaIntWord);
        } else if self.is_word_expr(expr) {
            self.gen_expression(expr);
            self.gen_acc_val(expr);
        } else {
            self.gen_expression(expr);
            self.gen_acc_val(expr);
            self.emit(StackInstruction::ExtendIntToWord);
        }
    }
    
    /// Generar código para RETURN
    fn gen_return(&mut self) {
        self.emit_comment("RETURN");
        self.emit(StackInstruction::IrInd);
    }
    
    /// gen_cod(ins_for(...)):
    ///   Inicializar variable de control
    ///   etiqueta(inicio_for)
    ///   Comprobar condición
    ///   ir-f(fin_for)
    ///   ... cuerpo del bucle ...
    ///   Incrementar variable
    ///   ir-a(inicio_for)
    ///   etiqueta(fin_for)
    fn gen_for(&mut self, assignment: &Assignment, to_expr: &Expr, step_expr: &Option<Expr>) {
        self.emit_comment(&format!("FOR {} TO {}",
            assignment.lvalue().show(false),
            to_expr.show(false)));

        let loop_start = self.new_label("FOR_INICIO");
        let loop_end = self.new_label("FOR_FIN");

        // Evaluar y guardar el STEP una única vez, antes de iterar (puede
        // ser una expresión arbitraria, p.ej. una variable, no solo un
        // literal) — 1 por defecto si no hay cláusula STEP. Se guarda en
        // una dirección de scratch propia de este FOR para poder releerlo
        // en cada NEXT y decidir la dirección de la comparación de salida
        // sin volver a evaluar la expresión.
        let step_addr = self.get_or_create_variable_address(&format!("__FOR_STEP_{}", loop_start));
        self.emit(StackInstruction::ApilaInt(step_addr as i64));
        match step_expr {
            Some(step) => {
                self.gen_expression(step);
                self.gen_acc_val(step);
            }
            None => {
                self.emit(StackInstruction::ApilaInt(1));
            }
        }
        self.emit(StackInstruction::DesapilaInd);

        // Guardar contexto en el stack de FORs
        self.for_stack.push(ForContext {
            variable_name: assignment.lvalue().show(false),
            loop_start: loop_start.clone(),
            loop_end: loop_end.clone(),
            step_addr,
            closed: false,
        });

        // Inicializar variable de control
        // Apilar dirección primero (modelo Tiny)
        self.gen_lvalue_address(assignment.lvalue());

        // Evaluar expresión inicial
        self.gen_expression(assignment.expr());
        self.gen_acc_val(assignment.expr());

        // Almacenar valor inicial
        self.emit(StackInstruction::DesapilaInd);

        // Etiqueta de inicio del bucle
        self.emit(StackInstruction::Label(loop_start.clone()));

        // Decidir la dirección de la comparación de salida según el signo
        // real del STEP (evaluado arriba, una sola vez). Como STEP puede
        // ser una expresión arbitraria, el signo se comprueba en tiempo de
        // ejecución, no se puede decidir en compilación.
        let descending = self.new_label("FOR_DESC");
        let body = self.new_label("FOR_BODY");

        // MenorInt/MayorInt comparan sin signo (el backend LH5801 no tiene
        // comparación de enteros con signo, solo Carry/borrow tras SBC), así
        // que "step < 0" no se puede probar directamente: un byte negativo
        // en complemento a dos (p.ej. -1 = 0xFF = 255) NO es "menor que 0"
        // en una comparación sin signo. En cambio, todo byte negativo en
        // complemento a dos es, sin signo, mayor que 127 (128-255), así que
        // "step > 127" (unsigned, ya implementado) detecta el signo igual
        // de bien sin necesitar una comparación con signo.
        self.emit(StackInstruction::ApilaInt(step_addr as i64));
        self.emit(StackInstruction::ApilaInd);
        self.emit(StackInstruction::ApilaInt(127));
        self.emit(StackInstruction::MayorInt);
        self.emit(StackInstruction::IrV(descending.clone()));

        // Caso ascendente (step >= 0): salir si variable > límite
        self.gen_lvalue_load(assignment.lvalue());
        self.gen_expression(to_expr);
        self.gen_acc_val(to_expr);
        self.emit(StackInstruction::MayorInt);
        self.emit(StackInstruction::IrV(loop_end.clone()));
        self.emit(StackInstruction::IrA(body.clone()));

        // Caso descendente (step < 0): salir si variable < límite
        self.emit(StackInstruction::Label(descending));
        self.gen_lvalue_load(assignment.lvalue());
        self.gen_expression(to_expr);
        self.gen_acc_val(to_expr);
        self.emit(StackInstruction::MenorInt);
        self.emit(StackInstruction::IrV(loop_end.clone()));

        self.emit(StackInstruction::Label(body));

        // El cuerpo del FOR se genera entre FOR y NEXT
        // Por ahora, solo preparamos las etiquetas

        // NOTA: El cuerpo real se procesará cuando encontremos las líneas
        // entre FOR y NEXT en el bucle principal
    }
    
    /// Generar código para NEXT
    ///
    /// `for_stack` se busca por NOMBRE de variable (no simplemente el
    /// último elemento) y el contexto encontrado NO se elimina de la
    /// pila — dos motivos, los dos observados en programas reales del
    /// corpus (p.ej. decathlon.bas):
    ///
    /// 1. Un mismo `FOR` puede tener más de un `NEXT` en el código
    ///    fuente: caminos de control alternativos (vía `GOTO`) que
    ///    convergen en el mismo bucle deben generar el mismo salto de
    ///    vuelta, no fallar por "ya consumido" en el segundo `NEXT` que
    ///    aparece en el código fuente.
    /// 2. Un `GOTO` puede abandonar un bucle interior sin pasar nunca
    ///    por su propio `NEXT`, dejando una entrada obsoleta por encima
    ///    del contexto real que un `NEXT` de un bucle más exterior
    ///    necesita — se descarta esa entrada obsoleta (definiendo su
    ///    etiqueta de fin para que la comprobación de salida que ya
    ///    generó su propio `FOR` quede resuelta) en vez de emparejar mal
    ///    por posición.
    fn gen_next(&mut self, lvalue: &LValue) {
        self.emit_comment(&format!("NEXT {}", lvalue.show(false)));

        let next_var = lvalue.show(false);
        let position = self.for_stack.iter().rposition(|ctx| ctx.variable_name == next_var);

        let for_context = match position {
            Some(pos) => {
                // Bug real encontrado testeando invader-v2.bas: los
                // contextos por encima de `pos` que YA estaban cerrados
                // (`closed`, marcados por su propio `NEXT` en una línea
                // anterior) no deben volver a emitir su etiqueta
                // `loop_end` aquí — eso produciría una etiqueta
                // DUPLICADA en el flujo (una ya emitida por su propio
                // `NEXT`, otra aquí), y la resolución de etiquetas del
                // backend se queda con la ÚLTIMA definición: el salto de
                // salida real de ESE bucle interior aterrizaría en la
                // copia duplicada, saltándose todo el código entre su
                // propio `NEXT` y este `NEXT` exterior (en
                // invader-v2.bas, línea 130: `NEXT D:B=L,C=C+3:NEXT Z` —
                // `B=L` nunca se ejecutaba a partir de la 2ª vuelta de
                // `FOR Z`, dejando `B` congelado con el valor de la 1ª
                // vuelta y acortando drásticamente el bucle que genera
                // el terreno). Un contexto NO cerrado aquí sí es
                // genuinamente huérfano (su `FOR` nunca llegó a su
                // propio `NEXT` en el código fuente) y sí necesita esa
                // etiqueta definida, para que el salto de salida de ese
                // bucle tenga dónde aterrizar.
                while self.for_stack.len() > pos + 1 {
                    let stale = self.for_stack.pop().expect("acabamos de comprobar que hay más de pos+1 elementos");
                    if !stale.closed {
                        self.emit(StackInstruction::Label(stale.loop_end));
                    }
                }
                // El contexto encontrado NO se saca de `for_stack`: un
                // mismo `FOR` puede cerrarse desde más de un `NEXT`
                // distinto en el código fuente (ramas de control de
                // flujo que convergen en el mismo bucle — ver
                // `test_oracle_for_next_multiple_next_statements_same_loop_on_real_rom`),
                // y todos deben poder reencontrarlo. Solo se marca
                // `closed` para que, si algún `NEXT` de un bucle
                // exterior lo encuentra después como "huérfano", sepa
                // que no debe volver a emitirle la etiqueta.
                self.for_stack[pos].closed = true;
                self.for_stack[pos].clone()
            }
            None => {
                // Ningún FOR pendiente para esta variable — límite
                // conocido (p.ej. control de flujo todavía más dinámico
                // que este backend no modela), no un error de programa:
                // se ignora este NEXT en vez de hacer panic.
                self.emit_comment(
                    "NEXT sin FOR pendiente para esta variable: ignorado \
                     (límite conocido, ver comentario de gen_next)",
                );
                return;
            }
        };

        // Apilar dirección primero (modelo Tiny)
        self.gen_lvalue_address(lvalue);

        // Incrementar variable de control por el STEP evaluado en gen_for
        // (no siempre 1: puede ser negativo o una expresión arbitraria).
        self.gen_lvalue_load(lvalue);
        self.emit(StackInstruction::ApilaInt(for_context.step_addr as i64));
        self.emit(StackInstruction::ApilaInd);
        self.emit(StackInstruction::SumaInt);
        
        // Almacenar nuevo valor
        self.emit(StackInstruction::DesapilaInd);
        
        // Volver al inicio del FOR
        self.emit(StackInstruction::IrA(for_context.loop_start));
        
        // Etiqueta de fin
        self.emit(StackInstruction::Label(for_context.loop_end));
    }
    
    /// Generar código para ON GOTO
    fn gen_on_goto(&mut self, expr: &Expr, targets: &[Expr]) {
        self.emit_comment("ON...GOTO");
        
        // Evaluar expresión
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        
        // Generar tabla de saltos
        for (i, target) in targets.iter().enumerate() {
            // Duplicar valor en la pila
            self.emit(StackInstruction::Dup);
            // Comparar con índice
            self.emit(StackInstruction::ApilaInt((i + 1) as i64));
            self.emit(StackInstruction::IgualInt);
            
            // Si es igual, saltar
            let label = self.expr_to_label(target);
            self.emit(StackInstruction::IrV(label));
        }
        
        // Si ninguno coincide, descartar y continuar
        self.emit(StackInstruction::Desapila);
    }
    
    /// Generar código para ON GOSUB
    fn gen_on_gosub(&mut self, expr: &Expr, targets: &[Expr]) {
        self.emit_comment("ON...GOSUB");
        
        // Similar a ON GOTO pero con llamadas
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        
        for (i, target) in targets.iter().enumerate() {
            self.emit(StackInstruction::Dup);
            self.emit(StackInstruction::ApilaInt((i + 1) as i64));
            self.emit(StackInstruction::IgualInt);
            
            let label = self.expr_to_label(target);
            let skip_label = self.new_label("SKIP_CALL");
            
            self.emit(StackInstruction::IrF(skip_label.clone()));
            self.emit(StackInstruction::Call(label));
            self.emit(StackInstruction::Label(skip_label));
        }
        
        self.emit(StackInstruction::Desapila);
    }
    
    /// Generar código para DIM
    ///
    /// Núcleo reducido de esta fase: solo se registran metadatos reales de
    /// array (usados luego por `gen_lvalue_address` para direccionar
    /// elementos correctamente) cuando las dimensiones son constantes en
    /// tiempo de compilación — el caso de la inmensa mayoría de `DIM` en
    /// programas reales (`DIM A(10)`, `DIM B(5,5)*3`...). Un tamaño
    /// dinámico (p.ej. `DIM B$(R)` con `R` variable, que sí aparece en
    /// algún programa real) queda documentado como no soportado todavía
    /// — no se genera código para él (nada que desapilar: no hay ninguna
    /// rutina de reserva en tiempo de ejecución implementada aún), en vez
    /// de fingir que funciona.
    fn gen_dim(&mut self, decls: &[DimInner]) {
        for decl in decls {
            match decl {
                DimInner::DimInner1D { identifier, size, string_length, .. } => {
                    self.emit_comment(&format!("DIM {}({})",
                        identifier.to_string(),
                        size.show(false)));

                    match self.const_eval_int(size) {
                        Some(n) if n >= 0 => {
                            // DIM A(N) declara índices 0..=N -> N+1 elementos.
                            let element_count = n as usize + 1;
                            let element_size = self.array_element_size(string_length, identifier.has_dollar());
                            let name = identifier.to_string();
                            let base_addr = self.get_or_create_array_address(&name, element_count * element_size);
                            self.array_metadata.insert(name, ArrayMeta {
                                base_addr,
                                element_size,
                                dims: ArrayDims::OneD { len: element_count },
                                dynamic_base_descriptor: None,
                            });
                        }
                        _ => {
                            // `DIM A(N)` con `N` no constante en tiempo de
                            // compilación (p.ej. `DIM B$(R)*1` con `R`
                            // variable, patrón real de blackjack.bas): la
                            // cuenta de elementos (y por tanto la dirección
                            // base) solo se conoce en tiempo de EJECUCIÓN.
                            // `array_element_size` sigue siendo constante
                            // (viene del `*N` de DIM, no de esto), así que
                            // solo hace falta reservar la base
                            // dinámicamente — de un heap fijo dedicado
                            // (`__ARRAY_HEAP`), avanzando un puntero
                            // (`__ARRAY_HEAP_PTR`) en tiempo de ejecución,
                            // y guardando la base resultante de ESTE array
                            // en su propio "descriptor" de 2 bytes
                            // (`__DIM_BASE_<nombre>`). `gen_lvalue_address`
                            // sabe leer esa base en tiempo de ejecución en
                            // vez de asumir un literal de compilación (ver
                            // `ArrayMeta::dynamic_base_descriptor`).
                            //
                            // Alcance deliberadamente acotado a 1D: el
                            // único caso 2D del corpus (`DIM A(Z,Z)` en
                            // jeu-des-blocs.bas) necesitaría ADEMÁS un
                            // nº de columnas dinámico releído en cada
                            // acceso indexado (hoy siempre una constante
                            // de compilación) — no soportado, cae al
                            // límite ya documentado en el caso 2D de abajo.
                            let element_size = self.array_element_size(string_length, identifier.has_dollar());
                            let name = identifier.to_string();

                            const DYNAMIC_ARRAY_HEAP_SIZE: usize = 512;
                            let heap_ptr_addr = self.get_or_create_variable_address("__ARRAY_HEAP_PTR");
                            let heap_base_addr =
                                self.get_or_create_array_address("__ARRAY_HEAP", DYNAMIC_ARRAY_HEAP_SIZE);
                            let descriptor_addr =
                                self.get_or_create_variable_address(&format!("__DIM_BASE_{}", name));

                            if !self.dynamic_array_heap_initialized {
                                self.dynamic_array_heap_initialized = true;
                                self.emit_comment("Inicializar heap de arrays con DIM dinámico (una sola vez)");
                                self.emit(StackInstruction::ApilaInt(heap_ptr_addr as i64));
                                self.emit(StackInstruction::ApilaInt(heap_base_addr as i64));
                                self.emit(StackInstruction::DesapilaIndWord);
                            }

                            self.emit_comment(&format!(
                                "DIM dinámico: reservar {} en __ARRAY_HEAP",
                                name
                            ));

                            // descriptor(name) = __ARRAY_HEAP_PTR actual (base de este array).
                            self.emit(StackInstruction::ApilaInt(descriptor_addr as i64));
                            self.emit(StackInstruction::ApilaInt(heap_ptr_addr as i64));
                            self.emit(StackInstruction::ApilaIndWord);
                            self.emit(StackInstruction::DesapilaIndWord);

                            // __ARRAY_HEAP_PTR += (size_expr + 1) * element_size.
                            // SumaIntWord espera [base(16 bits), offset(8
                            // bits)] en ese orden en la pila — la dirección
                            // de destino para el DesapilaIndWord final se
                            // apila ANTES de todo lo demás (modelo Tiny).
                            self.emit(StackInstruction::ApilaInt(heap_ptr_addr as i64));
                            self.emit(StackInstruction::ApilaInt(heap_ptr_addr as i64));
                            self.emit(StackInstruction::ApilaIndWord);
                            self.gen_expression(size);
                            self.gen_acc_val(size);
                            self.emit(StackInstruction::ApilaInt(1));
                            self.emit(StackInstruction::SumaInt);
                            self.emit(StackInstruction::ApilaInt(element_size as i64));
                            self.emit(StackInstruction::MulInt);
                            self.emit(StackInstruction::SumaIntWord);
                            self.emit(StackInstruction::DesapilaIndWord);

                            self.array_metadata.insert(name, ArrayMeta {
                                base_addr: 0,
                                element_size,
                                dims: ArrayDims::OneD { len: 0 },
                                dynamic_base_descriptor: Some(descriptor_addr),
                            });
                        }
                    }
                }
                DimInner::DimInner2D { identifier, rows, cols, string_length, .. } => {
                    self.emit_comment(&format!("DIM {}({}, {})",
                        identifier.to_string(),
                        rows.show(false),
                        cols.show(false)));

                    match (self.const_eval_int(rows), self.const_eval_int(cols)) {
                        (Some(r), Some(c)) if r >= 0 && c >= 0 => {
                            // DIM A(R,C) declara índices 0..=R, 0..=C.
                            let row_count = r as usize + 1;
                            let col_count = c as usize + 1;
                            let element_size = self.array_element_size(string_length, identifier.has_dollar());
                            let name = identifier.to_string();
                            let base_addr = self.get_or_create_array_address(
                                &name,
                                row_count * col_count * element_size,
                            );
                            self.array_metadata.insert(name, ArrayMeta {
                                base_addr,
                                element_size,
                                dims: ArrayDims::TwoD { rows: row_count, cols: col_count },
                                dynamic_base_descriptor: None,
                            });
                        }
                        _ => {
                            self.emit_comment("DIM con tamaño dinámico (no constante): no soportado todavía, no se reserva espacio real");
                        }
                    }
                }
            }
        }
    }

    /// Bytes por elemento de un array: el `*N` de `DIM` (si es una
    /// constante conocida) para arrays de cadena, o 1 byte para arrays
    /// numéricos — el único ancho que el resto del backend maneja de
    /// forma fiable hoy vía `ApilaInd`/`DesapilaInd`.
    ///
    /// Para un array de CADENA sin `*N` explícito (`DIM A$(7)`, patrón
    /// real de decathlon.bas/monstres&merveilles.bas), el valor por
    /// defecto NO puede ser 1 byte (el mismo que para un array numérico):
    /// ni siquiera cabe una cadena vacía (necesita al menos el NUL
    /// terminador), y un elemento de 1 byte hace que
    /// `is_direct_string_buffer` (que exige `element_size > 2`) trate el
    /// array como de ancho NO fijo — cada elemento pasaría a guardar un
    /// PUNTERO de 16 bits (`DesapilaIndWord`) en un hueco de solo 1 byte,
    /// pisando el primer byte del elemento siguiente en cada asignación
    /// (bug de solapamiento, distinto pero relacionado con el aliasing de
    /// buffer compartido ya arreglado para variables escalares). Se usa
    /// el mismo tamaño por defecto que una variable de cadena escalar
    /// (`DEFAULT_STRING_MAX_LEN+1`, dueña de su propio buffer con copia de
    /// contenido) para que `A$(i)` reciba exactamente el mismo tratamiento
    /// ya verificado, en vez de inventar un tercer caso.
    fn array_element_size(&self, string_length: &Option<Expr>, is_string_array: bool) -> usize {
        let default = if is_string_array { DEFAULT_STRING_MAX_LEN + 1 } else { 1 };
        string_length
            .as_ref()
            .and_then(|e| self.const_eval_int(e))
            .filter(|&n| n > 0)
            .map_or(default, |n| n as usize)
    }
    
    /// Generar código para READ
    fn gen_read(&mut self, destinations: &[LValue]) {
        let data_index_addr = self.get_or_create_variable_address("__DATA_INDEX");
        for lvalue in destinations {
            self.emit_comment(&format!("READ {}", lvalue.show(false)));

            // Dirección destino primero (modelo Tiny), luego el valor
            // leído. ReadData siempre produce un puntero a cadena (16
            // bits) hoy — collect_data_pool solo soporta valores de
            // cadena en DATA por ahora — así que el destino debe ser una
            // variable o array de cadena (gen_store_to_lvalue elige la
            // variante de 16 bits o de copia de ancho fijo según
            // corresponda; nunca DesapilaInd de 8 bits, le faltaría el
            // byte alto del puntero y descuadraría la pila).
            self.gen_lvalue_address(lvalue);
            self.emit(StackInstruction::ReadData(data_index_addr));
            self.gen_store_to_lvalue(lvalue);
        }
    }

    /// Generar código para DATA
    ///
    /// DATA no genera código ejecutable: sus valores ya se recogieron en
    /// la pre-pasada `collect_data_pool` (ver `generate`), que es la única
    /// forma correcta de tratarlos — en BASIC real el intérprete salta por
    /// encima de una línea DATA sin "ejecutarla", así que sus valores
    /// deben estar disponibles pase lo que pase por el flujo de control
    /// (dentro de un bucle, tras un END, en una rama nunca tomada...).
    fn gen_data(&mut self, _exprs: &[Expr]) {
        self.emit_comment("DATA (valores ya recogidos en la pre-pasada)");
    }

    /// Generar código para RESTORE
    fn gen_restore(&mut self, expr: &Option<Expr>) {
        let data_index_addr = self.get_or_create_variable_address("__DATA_INDEX");

        // RestoreData siempre desapila un número de línea de 16 bits
        // completo (puede comparar contra líneas BASIC de hasta 4 dígitos,
        // p.ej. 1000+), así que el argumento debe apilarse siempre como
        // entero de 16 bits — nunca `ApilaInt` (elige 1 byte para
        // valores <=255, lo que descuadraría la pila aquí). Una línea
        // constante se apila directamente; cualquier otra expresión
        // (variable, `<constante>+<expresión>` como `RESTORE
        // 999+RND 16` en bathyscaph.bas, ...) pasa por
        // `gen_dynamic_line_number`, el mismo mecanismo que usa `GOTO`/
        // `GOSUB` calculado.
        match expr {
            Some(line_expr) => {
                self.emit_comment(&format!("RESTORE {}", line_expr.show(false)));
                match self.const_eval_int(line_expr) {
                    Some(n) if (0..=0xFFFF).contains(&n) => {
                        self.emit(StackInstruction::ApilaIntWord(n));
                    }
                    _ => self.gen_dynamic_line_number(line_expr),
                }
            }
            None => {
                self.emit_comment("RESTORE");
                // 0 = reiniciar al principio (ningún número de línea BASIC real es 0).
                self.emit(StackInstruction::ApilaIntWord(0));
            }
        }
        self.emit(StackInstruction::RestoreData(data_index_addr));
    }

    /// Reconoce el patrón `<constante> + <expresión>` o `<expresión> +
    /// <constante>` (en ese orden en el AST, cualquiera de los dos lados)
    /// — devuelve `(constante, &expresión)` si `expr` tiene esa forma.
    /// Usado por `gen_dynamic_line_number` (`RESTORE`/`GOTO`/`GOSUB` con
    /// línea calculada) para detectar cuándo usar aritmética de 16 bits
    /// real en vez de la extensión con ceros por defecto.
    fn dynamic_line_number_base_and_offset<'e>(&self, expr: &'e Expr) -> Option<(i64, &'e Expr)> {
        match expr.inner() {
            ExprInner::Parentheses(inner) => self.dynamic_line_number_base_and_offset(inner),
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                if let Some(base) = self.const_eval_int(left) {
                    Some((base, right))
                } else {
                    self.const_eval_int(right).map(|base| (base, left.as_ref()))
                }
            }
            _ => None,
        }
    }
    
    /// gen_cod(ins_nl()):
    ///   emit newline()
    fn gen_end(&mut self) {
        self.emit_comment("END");
        self.emit(StackInstruction::Stop);
    }
    
    /// `CLEAR` real: pone a 0/"" todas las variables y arrays de usuario
    /// de tamaño estático conocidos EN ESTE PUNTO de la compilación —
    /// antes esto era un no-op completo (comentario histórico: el
    /// `CLEAR`/`DEL_STD_VARS` real de la ROM limpia la tabla de
    /// variables del INTÉRPRETE, que nunca usamos, así que "no hacía
    /// falta" tocar nada). Bug real encontrado jugando bathyscaph.bas de
    /// verdad (no en ningún test aislado): tras chocar, la subrutina
    /// "CRASH" (línea 210) llama a `CLEAR` antes de reiniciar `P`/`G`/`H`
    /// — pero como `CLEAR` no hacía nada, `S`/`R`/`Q` (la estela del
    /// submarino) se quedaban con el valor de la posición exacta donde
    /// chocó la partida anterior, y el primer par de fotogramas de la
    /// nueva partida dibujaba esa estela vieja en la posición 0 (donde
    /// reaparece el submarino) — un patrón con pinta de aleatorio, que
    /// cambia en cada choque porque el punto de colisión también cambia.
    ///
    /// Limitación deliberada, documentada, no un descuido: NO incluye
    /// (a) nombres que empiezan por `__` (variables internas del propio
    /// compilador — el índice de DATA, el scratch de AND/OR, los
    /// contadores de STEP de cada FOR, etc. — resetearlas rompería el
    /// programa a mitad de ejecución, no es lo que `CLEAR` significa en
    /// BASIC real); (b) arrays de tamaño DINÁMICO (`DIM B$(R)*1` con `R`
    /// variable — `ArrayMeta::dynamic_base_descriptor`), cuya dirección
    /// base real solo se conoce en tiempo de ejecución y necesitaría
    /// código adicional para leerla primero; (c) cualquier variable que
    /// el programa solo llegue a usar DESPUÉS de este `CLEAR` en el
    /// código fuente (`variable_addresses` solo conoce lo ya visto hasta
    /// aquí — una limitación real del recorrido de una sola pasada, ya
    /// señalada antes de intentar este arreglo). Para el patrón real de
    /// `bathyscaph.bas` (`CLEAR` casi al final del programa, después de
    /// que todas las variables relevantes ya se han usado) esto cubre el
    /// caso completo.
    fn gen_clear(&mut self) {
        self.emit_comment("CLEAR");

        let mut regions: Vec<(u16, u16)> = Vec::new();
        let mut names: Vec<&String> = self.variable_addresses.keys().collect();
        names.sort(); // orden determinista, no el de iteración del HashMap

        for name in names {
            // Las entradas de array viven bajo la clave namespaced
            // `"ARRAY:<nombre>"` en `variable_addresses` (ver
            // `get_or_create_array_address` — evita que un array y una
            // variable escalar del mismo nombre, patrón real de
            // invader-v2.bas, se aliasen). `array_metadata` sigue
            // indexado por el nombre BASIC real sin prefijo, así que solo
            // se consulta para entradas que SÍ llevan el prefijo — una
            // entrada ESCALAR nunca debe mirar ahí, aunque exista un
            // array real con el mismo nombre base (si no, la entrada
            // escalar heredaría por error el tamaño del array al
            // calcular cuánto limpiar, aunque su propia dirección sea
            // distinta y correcta).
            let array_basic_name = name.strip_prefix("ARRAY:");
            let filter_name = array_basic_name.unwrap_or(name);
            if filter_name.starts_with("__") {
                continue;
            }
            let addr = self.variable_addresses[name];

            if let Some(meta) = array_basic_name.and_then(|n| self.array_metadata.get(n)) {
                if meta.dynamic_base_descriptor.is_some() {
                    continue; // base real solo conocida en tiempo de ejecución
                }
                let elements = match meta.dims {
                    ArrayDims::OneD { len } => len,
                    ArrayDims::TwoD { rows, cols } => rows * cols,
                };
                let total = elements * meta.element_size;
                if total > 0 && addr <= u16::MAX as usize {
                    Self::push_region_chunked(&mut regions, addr as u16, total);
                }
            } else {
                let size = if name.ends_with('$') { DEFAULT_STRING_MAX_LEN + 1 } else { 10 };
                if addr <= u16::MAX as usize {
                    Self::push_region_chunked(&mut regions, addr as u16, size);
                }
            }
        }

        self.emit(StackInstruction::Clear(regions));
    }

    /// Trocea una región `(addr, byte_count)` en fragmentos de como
    /// mucho 255 bytes: el bucle de `StackInstruction::Clear` en el
    /// backend usa un contador de 8 bits.
    fn push_region_chunked(regions: &mut Vec<(u16, u16)>, addr: u16, byte_count: usize) {
        let mut offset = 0usize;
        while offset < byte_count {
            let chunk = (byte_count - offset).min(255);
            regions.push((addr + offset as u16, chunk as u16));
            offset += chunk;
        }
    }
    
    fn gen_cls(&mut self) {
        self.emit_comment("CLS");
        self.emit(StackInstruction::Cls);
    }
    
    // =========================================================================
    // CONTROL DE SISTEMA (WAIT, RANDOM, ARUN, LOCK, UNLOCK)
    // =========================================================================
    
    fn gen_wait(&mut self, expr: &Option<Expr>) {
        self.emit_comment("WAIT");
        match expr {
            Some(e) => {
                self.gen_expression(e);
                self.gen_acc_val(e);
                self.emit(StackInstruction::Wait);
            }
            None => {
                // `WAIT` sin argumento, en BASIC Sharp real, no es un
                // retardo de duración cero: bloquea indefinidamente
                // hasta que se pulsa cualquier tecla — semántica
                // completamente distinta de `WAIT n` (retardo
                // cronometrado real vía TIME_DELAY). Encontrado
                // compilando bombing.bas (línea 310: `WAIT :USING
                // :PRINT "*** SCORE *** :";W`, tras el choque —
                // claramente pensado para pausar hasta que el jugador
                // reaccione antes de ver la puntuación final).
                self.emit(StackInstruction::WaitForKey);
            }
        }
    }
    
    fn gen_random(&mut self) {
        self.emit_comment("RANDOM");
        // Misma dirección de semilla que RND() (ver FunctionInner::Rnd):
        // RANDOM simplemente avanza el mismo LFSR mock, no hace falta un
        // generador separado.
        let seed_addr = self.get_or_create_variable_address("__RND_SEED");
        self.emit(StackInstruction::Random(seed_addr));
    }
    
    fn gen_arun(&mut self) {
        self.emit_comment("ARUN");
        self.emit(StackInstruction::Arun);
    }
    
    fn gen_lock(&mut self) {
        self.emit_comment("LOCK");
        self.emit(StackInstruction::Lock);
    }
    
    fn gen_unlock(&mut self) {
        self.emit_comment("UNLOCK");
        self.emit(StackInstruction::Unlock);
    }
    
    // =========================================================================
    // I/O AVANZADO (PAUSE, LPRINT, USING, LF)
    // =========================================================================
    
    fn gen_pause(&mut self, print_inner: &PrintInner) {
        self.emit_comment("PAUSE");
        // La ROM real resetea el cursor de texto ANTES de imprimir el
        // mensaje de PAUSE (`BCMD_PAUSE_4`, $E6C1: `SJP (INIT_CURS)`) —
        // si no se replica esto, PAUSE hereda la posición de cursor que
        // dejó la sentencia anterior (p.ej. un INPUT justo antes), y un
        // mensaje que empieza cerca del borde derecho desborda a mitad
        // de frase, envolviéndose a la columna 0 (bug real encontrado en
        // bombing.bas: `INPUT "Explanations (Y/N) ? ";A$` seguido de
        // `PAUSE "Destroying large blocs..."` mostraba "troying..." con
        // "Des" desplazado al extremo derecho de la pantalla).
        //
        // Además, `BCMD_PAUSE` alcanza `CLR_NO_CURSOR` ($EC9C, "Clears
        // LCD if cursor is not allowed") tras forzar `CURSOR_ENA` bit0 a
        // 0 — esa rutina, con el bit forzado, llama a `LCD_CLR` (vector
        // $F2 de la tabla $FF00) antes de resetear `CURSOR_PTR`, así que
        // PAUSE no solo reposiciona el cursor: BORRA la pantalla entera
        // antes de escribir su mensaje. Confirmado también de forma
        // independiente jugando el programa original tokenizado en el
        // emulador: cada `PAUSE` de la secuencia de explicaciones
        // (líneas 340-380 de bombing.bas) limpia el display por
        // completo antes de mostrar su propio texto — sin esto, un
        // mensaje más corto que el anterior deja restos del más largo
        // en las columnas que no llega a tocar (visible sobre todo en
        // el extremo derecho). Usa el mismo mecanismo que `Cls`
        // (`LCD_CLR` + `INIT_CURS`, en ese orden) en vez del `InitCursor`
        // aislado (que solo cubriría el problema de posición, no el de
        // restos visuales).
        //
        // Actualización: no es el `Cls` incondicional — es `CLR_NO_CURSOR`,
        // que SOLO limpia si `CURSOR_ENA` bit0=0 (ver
        // `StackInstruction::ClsIfNoCursor`). En los usos de bombing.bas
        // que motivaron este fix no había ningún `CURSOR n` justo antes,
        // así que el comportamiento observado (limpia siempre) no cambia
        // para ese caso — pero si algún programa hace `CURSOR n:PAUSE
        // ...`, debe preservar posición/contenido igual que `PRINT`,
        // porque comparten literalmente el mismo código de ROM.
        self.emit(StackInstruction::ClsIfNoCursor);
        // PAUSE es similar a PRINT pero pausa después
        for (printable, sep) in &print_inner.exprs {
            match printable {
                crate::parse::statement::printable::Printable::Expr(expr) => {
                    self.gen_expression(expr);
                    self.gen_acc_val(expr);
                    if self.is_string_expr(expr) {
                        self.emit(StackInstruction::SystemOutString);
                    } else if self.is_word_expr(expr) {
                        self.emit(StackInstruction::SystemOutIntWord);
                    } else {
                        self.emit(StackInstruction::SystemOutInt);
                    }
                }
                crate::parse::statement::printable::Printable::UsingClause(_) => {
                    self.emit_comment("USING clause in PAUSE");
                }
            }
            match sep {
                PrintSeparator::Comma => self.emit(StackInstruction::PrintTab),
                PrintSeparator::Semicolon => {},
                PrintSeparator::None => self.emit(StackInstruction::Newline),
            }
        }
        self.emit(StackInstruction::Pause);
    }
    
    fn gen_lprint(&mut self, _lprint_inner: &LPrintInner) {
        self.emit_comment("LPRINT");
        self.emit(StackInstruction::LPrint);
    }
    
    /// `USING <patrón>` (sentencia suelta) y `PRINT USING <patrón>;...`
    /// (dentro de un `PRINT`, ver `gen_print`) comparten exactamente esta
    /// lógica: actualizar `self.current_using_format`, que es lo único
    /// que existe de "USING" en este compilador — no hay ninguna
    /// instrucción de pila para ello, porque el patrón siempre se conoce
    /// en tiempo de compilación (ver el comentario del campo). `USING`
    /// sin argumento (`format` es `None`) reinicia a formato decimal
    /// simple — igual que en BASIC real.
    fn apply_using_clause(&mut self, using_clause: &UsingClause) {
        match using_clause.format() {
            None => {
                self.current_using_format = None;
            }
            Some(expr) => match expr.inner() {
                ExprInner::StringLiteral { value, .. } => {
                    match Self::parse_using_pattern(value) {
                        Some(fmt) => self.current_using_format = Some(fmt),
                        None => {
                            eprintln!("WARNING: patrón USING \"{value}\" no reconocido: se ignora (formato decimal simple)");
                            self.current_using_format = None;
                        }
                    }
                }
                _ => {
                    eprintln!("WARNING: USING con patrón no literal ({}) no soportado todavía: se ignora", expr.show(false));
                    self.current_using_format = None;
                }
            },
        }
    }

    /// Parsea un patrón `USING` real (ver los patrones que de verdad
    /// aparecen en el corpus: `"####"`, `"###.##"`, `"*####"`,
    /// `"*+###"`, `"+####.##"`) a `UsingFormat`. Núcleo reducido:
    /// `[*][+]#+[.#+]` — un `*` opcional (relleno con asteriscos en vez
    /// de espacios), un `+` opcional (signo siempre visible), una tanda
    /// de `#` (dígitos enteros), y opcionalmente un `.` seguido de otra
    /// tanda de `#` (dígitos decimales). Cualquier otra cosa (separador
    /// de miles `,`, `$$`, `**`, patrón vacío) no está soportada y
    /// devuelve `None` — el llamador cae a formato decimal simple con un
    /// aviso, en vez de generar código incorrecto.
    fn parse_using_pattern(pattern: &str) -> Option<UsingFormat> {
        let mut chars = pattern.chars().peekable();

        let asterisk_fill = chars.next_if_eq(&'*').is_some();
        let forced_sign = chars.next_if_eq(&'+').is_some();

        let mut digits_before = 0u8;
        while chars.next_if_eq(&'#').is_some() {
            digits_before += 1;
        }
        if digits_before == 0 {
            return None;
        }

        let mut digits_after = 0u8;
        if chars.next_if_eq(&'.').is_some() {
            while chars.next_if_eq(&'#').is_some() {
                digits_after += 1;
            }
        }

        if chars.next().is_some() {
            // Sobra algo tras el último '#' reconocido (separador de
            // miles, '$$', '**', etc.) — patrón no soportado.
            return None;
        }

        Some(UsingFormat { digits_before, digits_after, asterisk_fill, forced_sign })
    }
    
    fn gen_lf(&mut self, expr: &Expr) {
        self.emit_comment(&format!("LF {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        // LF imprime valor y luego line feed
        if self.is_string_expr(expr) {
            self.emit(StackInstruction::SystemOutString);
        } else if self.is_word_expr(expr) {
            self.emit(StackInstruction::SystemOutIntWord);
        } else {
            self.emit(StackInstruction::SystemOutInt);
        }
        self.emit(StackInstruction::Newline);
    }
    
    // =========================================================================
    // GRÁFICOS Y CURSOR
    // =========================================================================
    
    fn gen_gprint(&mut self, exprs: &[(Expr, PrintSeparator)]) {
        self.emit_comment("GPRINT");
        for (expr, sep) in exprs {
            self.gen_expression(expr);
            self.gen_acc_val(expr);

            if self.is_string_expr(expr) {
                match self.gprint_string_length(expr) {
                    Some(len) => self.emit(StackInstruction::GPrintString(len)),
                    None => {
                        // `gen_expression`+`gen_acc_val` ya empujaron el
                        // puntero (16 bits) de la cadena — si no sabemos
                        // su longitud en tiempo de compilación, no
                        // dejarlo ahí sin más: eso filtra 2 bytes en la
                        // pila software por cada llamada. Bug real
                        // encontrado compilando bombing.bas: `GPRINT
                        // MID$ (A$,RND 5*2-1,2)` (perfil de ciudad, 100
                        // veces por partida) caía justo aquí — la
                        // ciudad nunca se dibujaba (ningún GPRINT real
                        // se emitía) Y además desincronizaba la pila
                        // para el resto del programa. Arreglado en dos
                        // frentes: `gprint_string_length` ahora sí
                        // reconoce `MID$`/`LEFT$`/`RIGHT$` cuando su
                        // longitud es una constante (el caso real de
                        // bombing.bas), y este `None` — que debería ser
                        // un caso ya raro tras ese fix — al menos
                        // descarta el puntero en vez de dejarlo fugado.
                        self.emit_comment(
                            "GPRINT de cadena con longitud no determinable en tiempo de \
                             compilación (p.ej. variable escalar): no soportado todavía — \
                             descartando el puntero ya empujado para no desbalancear la pila",
                        );
                        self.emit(StackInstruction::Desapila);
                        self.emit(StackInstruction::Desapila);
                    }
                }
            } else {
                self.emit(StackInstruction::GPrint);
            }

            match sep {
                PrintSeparator::Comma => self.emit(StackInstruction::PrintTab),
                PrintSeparator::Semicolon => {},
                PrintSeparator::None => {},
            }
        }
    }

    /// Longitud en tiempo de compilación de una expresión de cadena para
    /// `GPRINT` (necesita saber cuántos bytes iterar en el buffer): un
    /// literal usa su propia longitud; un elemento de array de cadena de
    /// ancho fijo (`DIM A$(N)*M`) usa `element_size` de
    /// `array_metadata`; `MID$`/`LEFT$`/`RIGHT$` usan su propio
    /// argumento de longitud cuando es una constante evaluable en
    /// tiempo de compilación (el patrón real de bombing.bas: `GPRINT
    /// MID$ (A$,RND 5*2-1,2)` — la POSICIÓN de inicio es dinámica, pero
    /// la LONGITUD, "2", es un literal, así que sí se puede saber de
    /// antemano cuántos bytes leerá GPRINT aunque no sepamos de qué
    /// posición). Variables de cadena escalares (puntero a un buffer
    /// NUL-terminado de longitud dinámica) no están soportadas — ningún
    /// programa objetivo usa `GPRINT` sobre una variable de cadena
    /// escalar, solo sobre literales, arrays de ancho fijo y
    /// MID$/LEFT$/RIGHT$ con longitud constante.
    fn gprint_string_length(&self, expr: &Expr) -> Option<usize> {
        match expr.inner() {
            ExprInner::Parentheses(inner) => self.gprint_string_length(inner),
            ExprInner::StringLiteral { value, .. } => Some(value.len()),
            ExprInner::LValue(lvalue) => match &lvalue.inner {
                LValueInner::Array1DAccess { identifier, .. }
                | LValueInner::Array2DAccess { identifier, .. } => {
                    self.array_metadata.get(&identifier.to_string()).map(|m| m.element_size)
                }
                _ => None,
            },
            ExprInner::FunctionCall(func) => match &func.inner {
                FunctionInner::Mid { length, .. }
                | FunctionInner::Left { length, .. }
                | FunctionInner::Right { length, .. } => self
                    .const_eval_int(length)
                    .filter(|&n| n >= 0)
                    .map(|n| n as usize),
                _ => None,
            },
            _ => None,
        }
    }
    
    fn gen_gcursor(&mut self, expr: &Expr) {
        self.emit_comment(&format!("GCURSOR {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        self.emit(StackInstruction::GCursor);
    }
    
    fn gen_cursor(&mut self, expr: &Expr) {
        self.emit_comment(&format!("CURSOR {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        self.emit(StackInstruction::Cursor);
    }
    
    fn gen_lcursor(&mut self, _clause: &LCursorClause) {
        self.emit_comment("LCURSOR");
        self.emit(StackInstruction::LCursor);
    }
    
    fn gen_glcursor(&mut self, x_expr: &Expr, y_expr: &Expr) {
        self.emit_comment(&format!("GLCURSOR {},{}", x_expr.show(false), y_expr.show(false)));
        self.gen_expression(x_expr);
        self.gen_acc_val(x_expr);
        self.gen_expression(y_expr);
        self.gen_acc_val(y_expr);
        self.emit(StackInstruction::GlCursor);
    }
    
    fn gen_line(&mut self, _inner: &LineInner) {
        self.emit_comment("LINE");
        self.emit(StackInstruction::Line);
    }
    
    fn gen_rline(&mut self, _inner: &LineInner) {
        self.emit_comment("RLINE");
        self.emit(StackInstruction::RLine);
    }
    
    fn gen_sorgn(&mut self) {
        self.emit_comment("SORGN");
        self.emit(StackInstruction::Sorgn);
    }
    
    fn gen_rotate(&mut self, expr: &Expr) {
        self.emit_comment(&format!("ROTATE {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        self.emit(StackInstruction::Rotate);
    }
    
    fn gen_text(&mut self) {
        self.emit_comment("TEXT");
        self.emit(StackInstruction::Text);
    }
    
    fn gen_graph(&mut self) {
        self.emit_comment("GRAPH");
        self.emit(StackInstruction::Graph);
    }
    
    fn gen_color(&mut self, expr: &Expr) {
        self.emit_comment(&format!("COLOR {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        self.emit(StackInstruction::Color);
    }
    
    fn gen_csize(&mut self, expr: &Expr) {
        self.emit_comment(&format!("CSIZE {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        self.emit(StackInstruction::CSize);
    }
    
    // =========================================================================
    // SONIDO (BEEP)
    // =========================================================================
    
    fn gen_beep(&mut self, repetitions_expr: &Expr, optional_params: &Option<BeepParams>) {
        self.emit_comment("BEEP");

        // BEEP repeticiones, frecuencia, duración. Empuja los 3 valores
        // en ese orden (repeticiones, frecuencia, duración) para que el
        // backend los desapile en orden inverso — ver StackInstruction::Beep
        // para el mapeo a la rutina ROM real (UL=frecuencia, X-Reg=duración).
        self.gen_expression(repetitions_expr);
        self.gen_acc_val(repetitions_expr);

        if let Some(params) = optional_params {
            self.gen_expression(&params.frequency);
            self.gen_acc_val(&params.frequency);

            if let Some(duration_expr) = &params.duration {
                self.gen_expression(duration_expr);
                self.gen_acc_val(duration_expr);
            } else {
                self.emit(StackInstruction::ApilaInt(50));
            }
        } else {
            self.emit(StackInstruction::ApilaInt(40));
            self.emit(StackInstruction::ApilaInt(50));
        }

        self.emit(StackInstruction::Beep);
    }
    
    fn gen_beep_onoff(&mut self, on: bool) {
        if on {
            self.emit_comment("BEEP ON");
            self.emit(StackInstruction::BeepOn);
        } else {
            self.emit_comment("BEEP OFF");
            self.emit(StackInstruction::BeepOff);
        }
    }
    
    // =========================================================================
    // MEMORIA (POKE, CALL)
    // =========================================================================
    
    /// `POKE dirección, v1, v2, ..., vN`: escribe `v1` en `dirección`, `v2`
    /// en `dirección+1`, ..., `vN` en `dirección+N-1` — la forma
    /// multi-valor real de Sharp BASIC, usada para embeber bloques de
    /// código máquina (ver invader.bas: `POKE &7050,&58,&76,&5A,...` con
    /// hasta 17 valores en una sola sentencia). Antes esto solo escribía
    /// `v1` e ignoraba silenciosamente `v2..vN` — cualquier programa con
    /// más de un valor por `POKE` perdía casi todos sus bytes sin ningún
    /// aviso ni error (el propio invader.bas: de los 17-18 bytes por
    /// línea, solo el primero llegaba a escribirse).
    fn gen_poke(&mut self, memory_area: &MemoryArea, exprs: &[Expr]) {
        self.emit_comment(&format!("POKE {:?}", memory_area));

        // La dirección en POKE es absoluta, no relativa a una base.
        // La diferencia entre Me0 y Me1 (POKE vs POKE#) es solo semántica
        // en la ROM real (memoria normal vs espacio de E/S) — ninguna
        // dirección de sistema que nos interesa vive en espacio de E/S,
        // así que ambas se tratan igual aquí (escritura directa a
        // memoria absoluta).

        if exprs.len() < 2 {
            // Si hay menos de 2 expresiones, es un error de sintaxis
            // pero generamos algo para no romper
            self.emit_comment("ERROR: POKE requiere dirección y valor");
            return;
        }

        let values = &exprs[1..];

        // Dirección base: siempre de 16 bits (una dirección de memoria
        // nunca cabe en 1 byte). Con múltiples valores, cada uno necesita
        // su propia dirección (base+i) conocida en tiempo de compilación
        // para poder generarla como literal — así que, a diferencia del
        // caso de un solo valor, aquí una dirección dinámica (variable,
        // no constante) con MÁS de un valor no está soportada (solo se
        // escribiría el primero, documentado como límite conocido, igual
        // que RESTORE con línea calculada); con un único valor sí se
        // soporta la dirección dinámica como antes.
        match self.const_eval_int(&exprs[0]) {
            Some(base) if (0..=0xFFFF).contains(&base) => {
                for (i, value) in values.iter().enumerate() {
                    let addr = base + i as i64;
                    if addr > 0xFFFF {
                        self.emit_comment("POKE: dirección se salió de 16 bits, resto ignorado");
                        break;
                    }
                    self.emit(StackInstruction::ApilaIntWord(addr));
                    self.gen_expression(value);
                    self.gen_acc_val(value);
                    self.emit(StackInstruction::Poke);
                }
            }
            _ => {
                if values.len() > 1 {
                    eprintln!(
                        "WARNING: POKE con dirección dinámica y {} valores: solo se escribirá el primero",
                        values.len()
                    );
                }
                self.emit_comment(
                    "POKE con dirección dinámica (no constante): no soportado todavía \
                     (necesitaría aritmética de 16 bits en tiempo de ejecución)",
                );
                self.gen_expression(&exprs[0]);
                self.gen_acc_val(&exprs[0]);
                self.gen_expression(&values[0]);
                self.gen_acc_val(&values[0]);
                self.emit(StackInstruction::Poke);
            }
        }
    }
    
    /// `CALL <dirección>`: invoca código máquina POKEado en RAM por el
    /// propio programa BASIC (patrón real de la época — el hueco de
    /// rendimiento de la interpretación se sorteaba escribiendo rutinas a
    /// mano en ensamblador y llamándolas desde BASIC; ver invader.bas,
    /// `POKE &7050,...` seguido de `CALL &7050`).
    ///
    /// En la inmensa mayoría de programas reales `<dirección>` es un
    /// literal (decimal o hex `&XXXX`) conocido en tiempo de compilación
    /// — se resuelve con `const_eval_int` (que ya reconoce ambos formatos)
    /// y se emite un `SJP` directo a esa dirección, sin pasar por el
    /// sistema de etiquetas. Antes de esto, CUALQUIER `CALL` (sin importar
    /// la dirección) emitía `Call("MACHINE_CODE")`, una etiqueta que nunca
    /// se definía en ningún sitio (panic "Undefined label" en cuanto un
    /// programa usaba `CALL`) y que, aunque se hubiera definido, habría
    /// mandado todos los `CALL &X` del programa a la MISMA dirección.
    ///
    /// Una dirección genuinamente dinámica (`CALL X`, `X` variable) NO
    /// está soportada todavía — necesitaría un salto indirecto real (el
    /// LH5801 no tiene "SJP (registro)", solo SJP a dirección inmediata;
    /// haría falta código automodificable o una tabla), nunca visto en el
    /// corpus real. Se documenta como límite conocido en vez de generar
    /// código roto: no se evalúa la expresión (evita descuadrar la pila
    /// con un push que nadie consumiría) y se avisa por stderr.
    fn gen_call(&mut self, expr: &Expr, variable: &Option<LValue>) {
        self.emit_comment(&format!("CALL {}", expr.show(false)));

        if let Some(_var) = variable {
            self.emit_comment("CALL with return variable: no soportado (nunca visto en el corpus real)");
        }

        match self.const_eval_int(expr) {
            Some(addr) => {
                self.emit(StackInstruction::CallAddr(addr as u16));
            }
            None => {
                eprintln!(
                    "WARNING: CALL a una dirección no constante ({}) no soportado todavía: ignorado",
                    expr.show(false)
                );
                self.emit_comment("CALL con dirección dinámica: no soportado, ignorado");
            }
        }
    }
    
    // =========================================================================
    // MODOS MATEMÁTICOS (RADIAN, DEGREE)
    // =========================================================================
    
    fn gen_radian(&mut self) {
        self.emit_comment("RADIAN");
        self.emit(StackInstruction::Radian);
    }
    
    fn gen_degree(&mut self) {
        self.emit_comment("DEGREE");
        self.emit(StackInstruction::Degree);
    }
    
    // =========================================================================
    // CONTROL DE ERRORES (ON ERROR GOTO)
    // =========================================================================
    
    fn gen_on_error_goto(&mut self, target: &Expr) {
        self.emit_comment(&format!("ON ERROR GOTO {}", target.show(false)));
        // Versión mínima documentada: solo registra cuál sería la línea
        // manejadora (para que exista una referencia de etiqueta real, ver
        // el backend), sin ninguna detección automática de errores en
        // tiempo de ejecución todavía. Antes esto evaluaba `target` como
        // una expresión en tiempo de ejecución pero luego emitía una
        // etiqueta literal fija ("ERROR_HANDLER") que nunca se definía en
        // ningún sitio -- el valor evaluado tampoco se consumía nunca
        // (fuga en la pila) y el backend paniqueaba con "Undefined label"
        // en cuanto se resolvían las etiquetas. `target` es casi siempre
        // un número de línea literal en el corpus real, así que se resuelve
        // en tiempo de compilación como cualquier GOTO/GOSUB estático; un
        // destino genuinamente dinámico queda fuera de este alcance mínimo
        // (nunca visto en el corpus real) y se ignora sin generar código
        // huérfano.
        if let Some(label) = self.static_goto_label(target) {
            self.emit(StackInstruction::OnErrorGoto(label));
        } else {
            self.emit_comment("ON ERROR GOTO con destino dinámico: ignorado (fuera de alcance)");
        }
    }
    
    // =========================================================================
    // GENERACIÓN DE CÓDIGO PARA EXPRESIONES
    // =========================================================================
    
    /// Generar código para una expresión
    /// Deja la dirección o valor en la pila según el tipo de expresión
    fn gen_expression(&mut self, expr: &Expr) {
        match &expr.inner() {
            // gen_cod(lit_ent(N)): emit apila-int(N)
            // gen_cod(lit_real(R)): emit apila-real(R)
            ExprInner::DecimalNumber(num) => {
                let value = num.as_f64();
                // Si el número tiene parte decimal, usar apila-real
                // Si es un entero exacto, usar apila-int
                if value.fract() == 0.0 && value.abs() <= i64::MAX as f64 {
                    self.emit(StackInstruction::ApilaInt(value as i64));
                } else {
                    self.emit(StackInstruction::ApilaReal(value));
                }
            }
            
            ExprInner::BinaryNumber(num) => {
                self.emit(StackInstruction::ApilaInt(num.as_u16() as i64));
            }
            
            // gen_cod(lit_cad(S)): emit apila-cadena(S)
            ExprInner::StringLiteral { value, .. } => {
                self.emit(StackInstruction::ApilaCadena(value.clone()));
            }
            
            // gen_cod(iden(Id)): gen_acc_id(vinculo)
            ExprInner::LValue(lvalue) => {
                self.gen_lvalue_address(lvalue);
            }
            
            // gen_cod(suma(Opnd0,Opnd1)):
            //   gen_cod_opnds_mat(Opnd0, Opnd1)
            //   if Opnd0.tipo == real || Opnd1.tipo == real
            //     emit suma-real
            //   else
            //     emit suma-int
            ExprInner::Binary(left, op, right) => {
                self.gen_binary_op(left, op, right);
            }
            
            // gen_cod(negativo(Opnd)):
            //   gen_cod(Opnd)
            //   gen_acc_val(Opnd)
            //   emit negativo
            ExprInner::Unary(op, operand) => {
                self.gen_unary_op(op, operand);
            }
            
            // Llamadas a funciones
            ExprInner::FunctionCall(func) => {
                self.gen_function_call(func);
            }
            
            ExprInner::Parentheses(inner) => {
                self.gen_expression(inner);
            }
        }
    }
    
    /// Generar código para operaciones binarias
    fn gen_binary_op(&mut self, left: &Expr, op: &BinaryOp, right: &Expr) {
        // Para las operaciones aritméticas y de comparación (no las
        // lógicas AND/OR, fuera de alcance para reales — ver el
        // comentario de `real_variables`), un operando "real" es
        // contagioso: si CUALQUIERA de los dos lados es una expresión real
        // (`is_real_expr` — literal decimal, o una variable marcada real
        // por `collect_real_variables`), toda la operación pasa a BCD real
        // — el otro lado, si era entero, se promociona con `Int2Real`
        // justo después de evaluarlo (mismo patrón que `ApilaInt` vs
        // `ApilaReal` para un literal suelto). Las comparaciones necesitan
        // esto igual que la aritmética desde que una variable puede quedar
        // marcada real (p.ej. `B=B+.5` en una línea, `IF B>0` en otra):
        // antes de eso ningún programa del corpus comparaba un real de
        // verdad, así que esta rama nunca se ejercía.
        let is_real = matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        ) && (self.is_real_expr(left) || self.is_real_expr(right));

        // Análogo a `is_real` para variables "de palabra" (ver el
        // comentario de `word_variables`) — pero deliberadamente acotado
        // a `Add`, el único caso real del corpus (`C=C+3`, `S=S+100`,
        // `RESTORE C+RND 3`...). `gen_acc_val`, más abajo, ya carga
        // CUALQUIER referencia a una variable "de palabra" como 16 bits
        // sin mirar qué operador la está usando — así que si esta
        // expresión resulta ser un operando de una operación que NO sea
        // `Add` (resta/multiplicación/comparación/...), hace falta la
        // salvaguarda de `TruncateWordToInt` de abajo para no
        // desincronizar la pila con esa instrucción de 8 bits.
        let is_word = !is_real && matches!(op, BinaryOp::Add)
            && (self.is_word_expr(left) || self.is_word_expr(right));

        // Generar código para ambos operandos
        self.gen_expression(left);
        self.gen_acc_val(left);
        if is_real && !self.is_real_expr(left) {
            if self.is_word_expr(left) {
                self.emit(StackInstruction::TruncateWordToInt);
            }
            self.emit(StackInstruction::Int2Real);
        } else if is_word && !self.is_word_expr(left) {
            self.emit(StackInstruction::ExtendIntToWord);
        } else if !is_real && !is_word && self.is_word_expr(left) {
            self.emit(StackInstruction::TruncateWordToInt);
        }
        self.gen_expression(right);
        self.gen_acc_val(right);
        if is_real && !self.is_real_expr(right) {
            if self.is_word_expr(right) {
                self.emit(StackInstruction::TruncateWordToInt);
            }
            self.emit(StackInstruction::Int2Real);
        } else if is_word && !self.is_word_expr(right) {
            self.emit(StackInstruction::ExtendIntToWord);
        } else if !is_real && !is_word && self.is_word_expr(right) {
            self.emit(StackInstruction::TruncateWordToInt);
        }

        match op {
            // Operaciones aritméticas
            BinaryOp::Add => {
                // Concatenación de cadenas (`A$+B$`): antes `+` sobre
                // cadenas caía siempre en `SumaInt`, sumando como enteros
                // de 8 bits dos punteros de 16 bits — corrompía la pila en
                // cualquier concatenación (confirmado contra la ROM real:
                // `A$="X":B$=A$+"Y"` dejaba S un byte por debajo de
                // stack_top). Mismo chequeo que ya usan `Eq`/`Neq` más
                // abajo para elegir comparación de cadenas.
                if self.is_string_expr(left) || self.is_string_expr(right) {
                    let buf = self.get_or_create_array_address("__CONCAT_BUF", 2 * DEFAULT_STRING_MAX_LEN + 1);
                    let right_scratch = self.get_or_create_array_address("__CONCAT_RIGHT_PTR", 2);
                    self.emit(StackInstruction::ConcatString(DEFAULT_STRING_MAX_LEN, buf, right_scratch));
                } else if is_real {
                    self.emit(StackInstruction::SumaReal);
                } else if is_word {
                    self.emit(StackInstruction::SumaWordWord);
                } else {
                    self.emit(StackInstruction::SumaInt);
                }
            }
            BinaryOp::Sub => {
                if is_real {
                    self.emit(StackInstruction::RestaReal);
                } else {
                    self.emit(StackInstruction::RestaInt);
                }
            }
            BinaryOp::Mul => {
                if is_real {
                    self.emit(StackInstruction::MulReal);
                } else {
                    self.emit(StackInstruction::MulInt);
                }
            }
            BinaryOp::Div => {
                if is_real {
                    self.emit(StackInstruction::DivReal);
                } else {
                    self.emit(StackInstruction::DivInt);
                }
            }
            BinaryOp::Exp => {
                self.emit(StackInstruction::PowInt);
            }

            // Operaciones de comparación: cadena si cualquier lado es
            // cadena, si no real si `is_real` (ver comentario de arriba),
            // si no entera.
            BinaryOp::Eq => {
                if self.is_string_expr(left) || self.is_string_expr(right) {
                    self.emit(StackInstruction::IgualCadena);
                } else if is_real {
                    self.emit(StackInstruction::IgualReal);
                } else {
                    self.emit(StackInstruction::IgualInt);
                }
            }
            BinaryOp::Neq => {
                if self.is_string_expr(left) || self.is_string_expr(right) {
                    self.emit(StackInstruction::DistintoCadena);
                } else if is_real {
                    self.emit(StackInstruction::DistintoReal);
                } else {
                    self.emit(StackInstruction::DistintoInt);
                }
            }
            BinaryOp::Lt => {
                if is_real {
                    self.emit(StackInstruction::MenorReal);
                } else {
                    self.emit(StackInstruction::MenorInt);
                }
            }
            BinaryOp::Leq => {
                if is_real {
                    self.emit(StackInstruction::MenorIgualReal);
                } else {
                    self.emit(StackInstruction::MenorIgualInt);
                }
            }
            BinaryOp::Gt => {
                if is_real {
                    self.emit(StackInstruction::MayorReal);
                } else {
                    self.emit(StackInstruction::MayorInt);
                }
            }
            BinaryOp::Geq => {
                if is_real {
                    self.emit(StackInstruction::MayorIgualReal);
                } else {
                    self.emit(StackInstruction::MayorIgualInt);
                }
            }
            
            // Operaciones lógicas
            BinaryOp::And => {
                let scratch = self.get_or_create_variable_address("__AND_OR_SCRATCH");
                self.emit(StackInstruction::AndInt(scratch));
            }
            BinaryOp::Or => {
                let scratch = self.get_or_create_variable_address("__AND_OR_SCRATCH");
                self.emit(StackInstruction::OrInt(scratch));
            }
        }
    }
    
    /// Generar código para operaciones unarias
    fn gen_unary_op(&mut self, op: &UnaryOp, operand: &Expr) {
        self.gen_expression(operand);
        self.gen_acc_val(operand);
        
        match op {
            // Unary plus: no-op
            UnaryOp::Plus => {
                // No hacer nada, el valor ya está en la pila
            }
            // gen_cod(negativo(Opnd)): ... emit negativo
            UnaryOp::Minus => {
                self.emit(StackInstruction::Negativo);
            }
            // gen_cod(not(Opnd)): ... emit not
            UnaryOp::Not => {
                self.emit(StackInstruction::Not);
            }
        }
    }
    
    /// Generar código para llamadas a funciones
    fn gen_function_call(&mut self, func: &Function) {
        match &func.inner {
            // Funciones matemáticas
            FunctionInner::Int { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                // CallInt siempre espera un real de 8 bytes en la pila
                // (ver su comentario en el backend: `emit_pop_8_to`) —
                // si `expr` no es ya real según el mismo criterio que
                // decide DivInt/DivReal en gen_binary_op (`is_real_expr`),
                // promocionar aquí. Sin esto, `INT(A/4)` (ninguno de los
                // dos operandos es un literal real) empuja un entero de 1
                // byte vía DivInt, y CallInt hace pop de 8 — fuga de 7
                // bytes en la pila software por cada llamada, encontrado
                // compilando rasemottes.bas (desbordaba S casi al
                // instante con la tecla de control mantenida, que llama a
                // `M=M+INT (A/4)` en cada vuelta del bucle).
                if !self.is_real_expr(expr) {
                    self.emit(StackInstruction::Int2Real);
                }
                self.emit(StackInstruction::CallInt);
            }
            FunctionInner::Abs { expr } => {
                // ABS(x) sobre un entero de 8 bits con signo (complemento a
                // 2, uso real confirmado en el corpus: siempre sobre una
                // resta entre variables/literales, nunca sobre un real):
                // compuesto con IR ya existente y verificado en vez de una
                // instrucción dedicada. Mismo truco de detección de signo
                // que el STEP descendente de FOR (ver gen_for): un byte
                // negativo en complemento a 2 es, sin signo, siempre >127.
                self.gen_expression(expr);
                self.gen_acc_val(expr);

                let negative = self.new_label("ABS_NEG");
                let done = self.new_label("ABS_DONE");

                self.emit(StackInstruction::Dup);
                self.emit(StackInstruction::ApilaInt(127));
                self.emit(StackInstruction::MayorInt);
                self.emit(StackInstruction::IrV(negative.clone()));
                self.emit(StackInstruction::IrA(done.clone()));

                self.emit(StackInstruction::Label(negative));
                self.emit(StackInstruction::Negativo);

                self.emit(StackInstruction::Label(done));
            }
            // SQR(x): sin rutina ROM verificada ni intención de investigar
            // una (mismo riesgo de callejón sin salida que RND/SIN/COS/TAN,
            // ver el roadmap acordado) — compuesto en su lugar con la
            // aritmética real YA verificada (`SumaReal`/`DivReal`, Fase 4)
            // vía iteración de Newton (`x_(n+1) = (x_n + v/x_n)/2` desde
            // `x_0=(v+1)/2`). El cuerpo del bucle se genera UNA sola vez
            // como subrutina compartida `__SQR_ROUTINE` (bucle real en
            // tiempo de EJECUCIÓN, contador `__SQR_I`) y no desenrollado en
            // cada punto de llamada: la primera versión desenrollaba 15
            // vueltas por cada `SQR`, y ya con solo 3 llamadas en un
            // programa de prueba de 8 líneas se superaba el techo real de
            // 10240 bytes de RAM de usuario (`CodeTooLarge` al cargar en el
            // emulador) — un desastre de tamaño de código si algún programa
            // del corpus llama a `SQR` más de una o dos veces. Cada punto de
            // llamada solo guarda `v`/`x_0` y hace un `Call` (SJP) a la
            // subrutina, igual que una llamada ROM.
            FunctionInner::Sqr { expr } => {
                self.sqr_used = true;
                let v_addr = self.get_or_create_array_address("__SQR_V", 8);
                let x_addr = self.get_or_create_array_address("__SQR_X", 8);

                self.emit(StackInstruction::ApilaInt(v_addr as i64));
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                if !self.is_real_expr(expr) {
                    self.emit(StackInstruction::Int2Real);
                }
                self.emit(StackInstruction::DesapilaIndReal);

                // x = (v + 1) / 2 (estimación inicial).
                self.emit(StackInstruction::ApilaInt(x_addr as i64));
                self.emit(StackInstruction::ApilaInt(v_addr as i64));
                self.emit(StackInstruction::ApilaIndReal);
                self.emit(StackInstruction::ApilaReal(1.0));
                self.emit(StackInstruction::SumaReal);
                self.emit(StackInstruction::ApilaReal(2.0));
                self.emit(StackInstruction::DivReal);
                self.emit(StackInstruction::DesapilaIndReal);

                self.emit(StackInstruction::Call("__SQR_ROUTINE".to_string()));

                self.emit(StackInstruction::ApilaInt(x_addr as i64));
                self.emit(StackInstruction::ApilaIndReal);
            }
            FunctionInner::Sin { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallSin);
            }
            FunctionInner::Cos { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallCos);
            }
            FunctionInner::Tan { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallTan);
            }
            
            // Funciones de cadenas
            FunctionInner::Len { expr } => {
                let max_len = self.string_source_max_len(expr);
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallLen(max_len));
            }
            FunctionInner::Mid { string, start, length } => {
                let max_len = self.string_source_max_len(string);
                let buf = self.get_or_create_array_address("__MID_BUF", DEFAULT_STRING_MAX_LEN + 1);
                self.gen_expression(string);
                self.gen_acc_val(string);
                self.gen_expression(start);
                self.gen_acc_val(start);
                self.gen_expression(length);
                self.gen_acc_val(length);
                self.emit(StackInstruction::CallMid(max_len, buf));
            }
            FunctionInner::Left { string, length } => {
                let max_len = self.string_source_max_len(string);
                let buf = self.get_or_create_array_address("__LEFT_BUF", DEFAULT_STRING_MAX_LEN + 1);
                self.gen_expression(string);
                self.gen_acc_val(string);
                self.gen_expression(length);
                self.gen_acc_val(length);
                self.emit(StackInstruction::CallLeft(max_len, buf));
            }
            FunctionInner::Right { string, length } => {
                let max_len = self.string_source_max_len(string);
                let buf = self.get_or_create_array_address("__RIGHT_BUF", DEFAULT_STRING_MAX_LEN + 1);
                self.gen_expression(string);
                self.gen_acc_val(string);
                self.gen_expression(length);
                self.gen_acc_val(length);
                self.emit(StackInstruction::CallRight(max_len, buf));
            }
            FunctionInner::Chr { expr } => {
                let buf = self.get_or_create_array_address("__CHR_BUF", 2);
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallChr(buf));
            }
            FunctionInner::Asc { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallAsc);
            }
            FunctionInner::Str { expr } => {
                let buf = self.get_or_create_array_address("__STR_BUF", 4);
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallStr(buf));
            }
            FunctionInner::Val { expr } => {
                let max_len = self.string_source_max_len(expr);
                let scratch = self.get_or_create_array_address("__VAL_SCRATCH", 4);
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallVal(max_len, scratch));
            }
            
            // Otras funciones
            FunctionInner::Rnd { range_end } => {
                self.gen_expression(range_end);
                self.gen_acc_val(range_end);
                // CallRnd espera SIEMPRE su argumento como entero de 16
                // bits en la pila. gen_expression solo empuja ya 16 bits
                // cuando range_end es un LITERAL constante >255 (vía
                // ApilaInt, que elige el ancho según el valor) — para
                // cualquier otro caso (variable, expresión, literal
                // <=255) solo empuja 8 bits, y hay que extenderlo aquí o
                // CallRnd desincroniza la pila. Bug real encontrado
                // jugando bathyscaph.bas de verdad: `RND 256-1` (n=256,
                // no cabe en 8 bits) dejaba 1 byte suelto en la pila en
                // cada llamada — ver el comentario de CallRnd en el
                // backend para el resto del arreglo.
                let pushed_as_word = matches!(self.const_eval_int(range_end), Some(n) if n > 255);
                if !pushed_as_word {
                    self.emit(StackInstruction::ExtendIntToWord);
                }
                let seed_addr = self.get_or_create_variable_address("__RND_SEED");
                self.emit(StackInstruction::CallRnd(seed_addr));
            }
            
            // Funciones matemáticas adicionales
            FunctionInner::Sgn { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                // Mismo caso que FunctionInner::Int: CallSgn también
                // espera un real de 8 bytes. bathyscaph.bas solo usa
                // SGN sobre una expresión ya real (`SGN(ASC Z$-10.5)`,
                // real por el literal .5), lo que ocultaba este mismo
                // bug — ver el comentario de la promoción análoga en
                // FunctionInner::Int.
                if !self.is_real_expr(expr) {
                    self.emit(StackInstruction::Int2Real);
                }
                self.emit(StackInstruction::CallSgn);
            }
            
            // Funciones gráficas
            FunctionInner::Point { position } => {
                self.gen_expression(position);
                self.gen_acc_val(position);
                self.emit(StackInstruction::CallPoint);
            }
            
            // Funciones de sistema.
            //
            // `STATUS n`, verificado contra el Manual de Referencia
            // Técnico real (sección 5-1-2, no adivinado): `STATUS 0/1/3`
            // hablan de tamaños/direcciones del ÁREA DE PROGRAMA BASIC
            // interpretado, que no existe como tal en código nativo — sin
            // uso real en el corpus, no implementados (documentado como
            // descarte, no olvido). `STATUS 2` (dirección justo después
            // del programa BASIC, "área de usuario libre") sí se usa de
            // verdad (blackjack.bas, pacman.bas, simulateur-de-vol.bas):
            // aquí se resuelve como la dirección de un buffer de scratch
            // fijo (`__STATUS2_SCRATCH`), reservado por si el programa
            // quiere hacer PEEK/POKE ahí — el patrón real de la época
            // "reservar código máquina justo después del programa". NO
            // cubre el patrón de blackjack.bas/pacman.bas que además hace
            // `POKE`/`CALL` a una dirección CALCULADA en tiempo de
            // ejecución a partir de este valor (`POKE A,...` con `A`
            // variable) — `gen_poke`/`gen_call` solo soportan direcciones
            // constantes; extenderlos es un trabajo aparte, mayor y
            // explícitamente pospuesto (ver roadmap, decisión del
            // 2026-08-26). `STATUS 4` (línea BASIC actual en ejecución,
            // 0 si no hay programa corriendo interpretado) no tiene
            // equivalente real en código nativo sin intérprete de líneas
            // — se documenta como limitación conocida y siempre devuelve
            // 0 (usado en monstres&merveilles.bas solo para un chequeo
            // puntual `IF E=709`, que simplemente nunca se cumplirá).
            //
            // Siempre real (`ApilaReal`, nunca `ApilaInt`): una dirección
            // de 16 bits no cabe en el entero de 8 bits que usa el resto
            // del backend para variables no reales — cabe sin problema en
            // el formato BCD real (ver el comentario de `is_real_expr`).
            FunctionInner::Status { arg } => {
                match self.const_eval_int(arg) {
                    Some(2) => {
                        let addr = self.get_or_create_array_address("__STATUS2_SCRATCH", 32);
                        self.emit(StackInstruction::ApilaReal(addr as f64));
                    }
                    Some(4) => {
                        self.emit(StackInstruction::ApilaReal(0.0));
                    }
                    Some(n) => {
                        eprintln!(
                            "WARNING: STATUS {} no soportado (núcleo reducido: solo 2 y 4), devolviendo 0",
                            n
                        );
                        self.emit(StackInstruction::ApilaReal(0.0));
                    }
                    None => {
                        eprintln!("WARNING: STATUS con argumento no constante no soportado, devolviendo 0");
                        self.emit(StackInstruction::ApilaReal(0.0));
                    }
                }
            }
            
            _ => {
                self.emit_comment(&format!("TODO: función {:?}", 
                    std::mem::discriminant(&func.inner)));
            }
        }
    }
    
    // =========================================================================
    // HELPERS PARA ACCESO A VARIABLES Y ARRAYS
    // =========================================================================

    /// Emitir la instrucción de almacenamiento adecuada para `lvalue`,
    /// asumiendo que dirección y valor ya están en la pila (dirección
    /// primero, valor encima — modelo Tiny). El sufijo `$` del propio
    /// identificador decide el modo (es más fiable que intentar inferir
    /// el tipo de la expresión del lado derecho, ya que en BASIC el `$`
    /// es una declaración de tipo del propio nombre, no de la expresión):
    /// - Variable numérica o array numérico: `DesapilaInd` (1 byte) — salvo
    ///   que sea una variable escalar marcada real por
    ///   `collect_real_variables` (ver el comentario de `real_variables`),
    ///   en cuyo caso `DesapilaIndReal` (8 bytes).
    /// - Variable de cadena escalar (`Z$`, sin DIM de ancho fijo):
    ///   `DesapilaIndWord` — el valor es un puntero de 16 bits.
    /// - Elemento de array de cadena con ancho fijo declarado (`DIM
    ///   A$(N)*M`, `M>2`): `DesapilaIndStringCopy(M)` — copia los
    ///   caracteres al buffer reservado, no sobreescribe un puntero.
    fn gen_store_to_lvalue(&mut self, lvalue: &LValue) {
        let instr = match &lvalue.inner {
            // Variable de cadena escalar: COPIA el contenido real a su
            // propio buffer (`DEFAULT_STRING_MAX_LEN+1` bytes, ver
            // `get_or_create_variable_address`) en vez de guardar solo un
            // puntero — ver `is_direct_string_buffer` para el porqué
            // (bug de aliasing real, confirmado contra la ROM, cuando el
            // lado derecho venía de un buffer compartido como
            // `__LEFT_BUF`/`__MID_BUF`/`__CONCAT_BUF`/...).
            LValueInner::Identifier(id) if id.has_dollar() => {
                StackInstruction::DesapilaIndStringCopy(DEFAULT_STRING_MAX_LEN)
            }
            LValueInner::Array1DAccess { identifier, .. } | LValueInner::Array2DAccess { identifier, .. }
                if identifier.has_dollar() =>
            {
                let element_size = self.array_metadata.get(&identifier.to_string()).map_or(1, |m| m.element_size);
                if element_size > 2 {
                    StackInstruction::DesapilaIndStringCopy(element_size)
                } else {
                    StackInstruction::DesapilaIndWord
                }
            }
            LValueInner::Identifier(id) if self.real_variables.contains(&id.to_string()) => {
                StackInstruction::DesapilaIndReal
            }
            LValueInner::Identifier(id) if self.word_variables.contains(&id.to_string()) => {
                StackInstruction::DesapilaIndWord
            }
            _ => StackInstruction::DesapilaInd,
        };
        self.emit(instr);
    }

    /// Generar código para obtener la dirección de un lvalue
    /// gen_acc_id(variable): emite código para poner la dirección en la pila
    fn gen_lvalue_address(&mut self, lvalue: &LValue) {
        match &lvalue.inner {
            // Variable simple
            LValueInner::Identifier(id) => {
                // gen_acc_id(variable(Tipo, id)):
                //   if $.nivel = 0 then
                //     emit apila-int($.vinculo.dir)
                let addr = self.get_or_create_variable_address(&id.to_string());
                self.emit(StackInstruction::ApilaInt(addr as i64));
            }
            
            // Array 1D
            // gen_cod(index(Opnd0, Opnd1)):
            //   gen_cod(Opnd0)         // dirección base
            //   gen_cod(Opnd1)         // índice
            //   gen_acc_val(Opnd1)
            //   emit apila-int(T.tam)  // tamaño del elemento
            //   emit mul-int
            //   emit suma-int
            LValueInner::Array1DAccess { identifier, index } => {
                // Dirección base del array. Si hubo un DIM con tamaño
                // constante, usa el tamaño de elemento real registrado en
                // gen_dim; si no (array sin DIM), cae al valor histórico
                // de 5 bytes como límite conocido, no una constante
                // "correcta". Si el DIM fue de tamaño DINÁMICO (ver
                // `ArrayMeta::dynamic_base_descriptor`), la base no es un
                // literal de compilación: hay que leerla en tiempo de
                // ejecución del descriptor que `gen_dim` fue rellenando.
                let name = identifier.to_string();
                let meta = self.array_metadata.get(&name).copied();
                let element_size = meta.map_or(5, |m| m.element_size);

                match meta.and_then(|m| m.dynamic_base_descriptor) {
                    Some(descriptor_addr) => {
                        self.emit(StackInstruction::ApilaInt(descriptor_addr as i64));
                        self.emit(StackInstruction::ApilaIndWord);
                    }
                    None => {
                        // Bug real de invader-v2.bas: esto llamaba a
                        // `get_or_create_variable_address` (el namespace
                        // de ESCALARES) en vez de usar el `base_addr` ya
                        // calculado por `gen_dim` (namespace de ARRAYS,
                        // ver `get_or_create_array_address`) — un array
                        // `B(5)` y una variable escalar `B` del mismo
                        // programa (patrón real y válido: la sintaxis ya
                        // los distingue por los paréntesis) acababan
                        // aliasados en la misma dirección, porque CADA
                        // ACCESO al array (no solo el `DIM`) recreaba una
                        // dirección de escalar para el mismo nombre. Si
                        // hay metadatos (el `DIM` con tamaño constante ya
                        // se procesó), usar su `base_addr` real; si no
                        // (array usado sin `DIM` previo), reservar bajo
                        // el namespace de arrays igualmente — nunca el
                        // de escalares — con el mismo tamaño histórico
                        // de reserva (10 bytes) que tenía antes por
                        // accidente vía `get_or_create_variable_address`.
                        let base_addr = match meta {
                            Some(m) => m.base_addr,
                            None => self.get_or_create_array_address(&name, 10),
                        };
                        self.emit(StackInstruction::ApilaInt(base_addr as i64));
                    }
                }

                // Índice
                self.gen_expression(index);
                self.gen_acc_val(index);

                self.emit(StackInstruction::ApilaInt(element_size as i64));
                self.emit(StackInstruction::MulInt);

                // Dirección = base + índice * tamaño. `SumaIntWord`, no
                // `SumaInt`: la base es una dirección de 16 bits (siempre
                // > 255 en la práctica, cae en la rama de 2 bytes de
                // `ApilaInt`/`ApilaIndWord`) y `SumaInt` solo suma el byte
                // bajo, sin propagar el acarreo al alto — invisible
                // mientras `índice*tamaño` no cruce un límite de página de
                // 256 bytes dentro del propio array, pero real: confirmado
                // contra la ROM real con `DIM A$(3)` sin ancho fijo (element
                // ahora de 41 bytes, ver `array_element_size`) — A$(2)/A$(3)
                // escribían 256 bytes por debajo de su dirección real.
                self.emit(StackInstruction::SumaIntWord);
            }

            // Array 2D
            LValueInner::Array2DAccess { identifier, row_index, col_index } => {
                // Dirección base. Misma lógica que Array1DAccess: usa las
                // dimensiones reales del DIM si están registradas, si no
                // cae a los valores históricos (10 columnas, 5 bytes/elem).
                let name = identifier.to_string();
                // Mismo bug/arreglo que en Array1DAccess: usar el
                // `base_addr` real de `array_metadata` (namespace de
                // arrays) en vez de recrear una dirección de ESCALAR
                // para el mismo nombre.
                let (base_addr, col_count, element_size) = match self.array_metadata.get(&name) {
                    Some(ArrayMeta { base_addr, dims: ArrayDims::TwoD { cols, .. }, element_size, .. }) => {
                        (*base_addr, *cols, *element_size)
                    }
                    _ => (self.get_or_create_array_address(&name, 10 * 10 * 5), 10, 5),
                };
                self.emit(StackInstruction::ApilaInt(base_addr as i64));

                // Calcular offset: (i * num_cols + j) * tam_elemento
                self.gen_expression(row_index);
                self.gen_acc_val(row_index);

                self.emit(StackInstruction::ApilaInt(col_count as i64));
                self.emit(StackInstruction::MulInt);

                self.gen_expression(col_index);
                self.gen_acc_val(col_index);
                self.emit(StackInstruction::SumaInt);

                // Multiplicar por tamaño del elemento
                self.emit(StackInstruction::ApilaInt(element_size as i64));
                self.emit(StackInstruction::MulInt);

                // Sumar a la base. `SumaIntWord`, no `SumaInt` — mismo
                // motivo que en Array1DAccess (acarreo al byte alto de la
                // dirección de 16 bits).
                self.emit(StackInstruction::SumaIntWord);
            }
            
            // Built-in identifiers like TIME, PI, INKEY$
            LValueInner::BuiltInIdentifier(keyword) if *keyword == Keyword::InkeyDollar => {
                self.emit_comment("INKEY$");
                let char_buf = self.get_or_create_array_address("__INKEY_CHAR_BUF", 2);
                let ptr_slot = self.get_or_create_array_address("__INKEY_PTR_SLOT", 2);
                self.emit(StackInstruction::CallInkey(char_buf, ptr_slot));
                // Mismo convenio que cualquier otra variable de cadena:
                // se apila la DIRECCIÓN (ptr_slot) para que gen_acc_val la
                // desreferencie con ApilaIndWord y obtenga el puntero real
                // que CallInkey acaba de escribir ahí.
                self.emit(StackInstruction::ApilaInt(ptr_slot as i64));
            }
            LValueInner::BuiltInIdentifier(keyword) => {
                self.emit_comment(&format!("Built-in identifier: {:?}", keyword));
                // TODO: Implementar soporte para el resto de identificadores
                // built-in (TIME, PI...). Por ahora, asignamos una dirección
                // arbitraria.
                let addr = self.get_or_create_variable_address(&format!("{:?}", keyword));
                self.emit(StackInstruction::ApilaInt(addr as i64));
            }
            
            // Fixed memory area access like @(E)
            LValueInner::FixedMemoryAreaAccess { index, has_dollar } => {
                self.emit_comment(&format!("Memory area access, has_dollar: {}", has_dollar));
                self.gen_expression(index);
                self.gen_acc_val(index);
                // La dirección ya está en la pila (es el índice directo en memoria)
            }
        }
    }
    
    /// Cargar el valor de un lvalue (no su dirección)
    fn gen_lvalue_load(&mut self, lvalue: &LValue) {
        self.gen_lvalue_address(lvalue);
        self.emit(StackInstruction::ApilaInd);
    }
    
    /// gen_acc_val(E):
    ///   if es_designador(E) then
    ///     emit apila-ind()
    ///   end if
    fn gen_acc_val(&mut self, expr: &Expr) {
        // Si la expresión es un designador (variable, array, etc.),
        // necesitamos cargar su valor.
        if !self.es_designador(expr) {
            return;
        }
        if self.is_direct_string_buffer(expr) {
            // Cadena cuyo almacenamiento ES el buffer de caracteres (ver
            // `is_direct_string_buffer`: elemento de array de ancho fijo
            // O variable escalar `Z$`) — no hay un puntero intermedio que
            // cargar. La dirección que ya dejó
            // gen_lvalue_address/gen_expression es el "valor" a efectos
            // de comparación/paso a funciones, así que no se emite nada
            // más aquí.
        } else if self.is_string_expr(expr) {
            // Cualquier otra expresión de cadena (literal, resultado de
            // MID$/LEFT$/RIGHT$/CHR$/STR$/concatenación, INKEY$, un
            // array de cadena SIN ancho fijo) sí guarda/produce un
            // puntero de 16 bits, así que cargarla necesita
            // ApilaIndWord, no ApilaInd (8 bits) — usarla ahí perdería
            // el byte alto del puntero y descuadraría la pila.
            self.emit(StackInstruction::ApilaIndWord);
        } else if self.is_real_expr(expr) {
            // Variable escalar marcada real por `collect_real_variables`:
            // guarda 8 bytes (formato ARX/ARY), así que cargarla necesita
            // ApilaIndReal, no ApilaInd (1 byte) — mismo motivo que el
            // caso de cadena de arriba, aquí para el resultado de
            // aritmética real (`SumaReal`/`RestaReal`/...).
            self.emit(StackInstruction::ApilaIndReal);
        } else if self.is_word_expr(expr) {
            // Variable escalar marcada "de palabra" (ver
            // `word_variables`): guarda 2 bytes, así que cargarla
            // necesita `ApilaIndWord`, no `ApilaInd` (1 byte) — mismo
            // motivo que los dos casos de arriba.
            self.emit(StackInstruction::ApilaIndWord);
        } else {
            self.emit(StackInstruction::ApilaInd);
        }
    }

    /// ¿Es `expr` un designador de cadena cuyo almacenamiento ES el
    /// buffer de caracteres en sí — sin ningún puntero intermedio que
    /// desreferenciar? Dos casos:
    /// - Un elemento de array de cadena de ancho fijo (`DIM A$(N)*M`,
    ///   `M>2`).
    /// - Una variable de cadena escalar (`Z$`). Antes SOLO el primer caso
    ///   se trataba así: una variable escalar guardaba un PUNTERO de 16
    ///   bits (vía `DesapilaIndWord`/`ApilaIndWord`) a lo que fuera que
    ///   produjo el lado derecho de su asignación — si eso era el
    ///   resultado de `MID$`/`LEFT$`/`RIGHT$`/`CHR$`/`STR$`/concatenación
    ///   (todas con un único buffer de scratch COMPARTIDO por función,
    ///   ver `__MID_BUF` etc.), la variable quedaba apuntando al buffer
    ///   compartido, no a una copia propia — cualquier llamada POSTERIOR
    ///   a esa MISMA función en cualquier parte del programa invalidaba
    ///   en silencio el valor ya asignado (confirmado contra la ROM real:
    ///   `B$=LEFT$(A$,5)` seguido de una llamada a `LEFT$` no relacionada
    ///   dejaba `B$` con el resultado de la SEGUNDA llamada). Ahora una
    ///   variable escalar reserva sus propios `DEFAULT_STRING_MAX_LEN+1`
    ///   bytes (ver `get_or_create_variable_address`) y la asignación
    ///   COPIA los caracteres ahí (`DesapilaIndStringCopy`, ver
    ///   `gen_store_to_lvalue`) — exactamente el mismo modelo que ya
    ///   usaba un array de ancho fijo, extendido a variables escalares.
    fn is_direct_string_buffer(&self, expr: &Expr) -> bool {
        match expr.inner() {
            ExprInner::Parentheses(inner) => self.is_direct_string_buffer(inner),
            ExprInner::LValue(lvalue) => match &lvalue.inner {
                LValueInner::Array1DAccess { identifier, .. }
                | LValueInner::Array2DAccess { identifier, .. } => {
                    identifier.has_dollar()
                        && self
                            .array_metadata
                            .get(&identifier.to_string())
                            .is_some_and(|m| m.element_size > 2)
                }
                LValueInner::Identifier(id) => id.has_dollar(),
                _ => false,
            },
            _ => false,
        }
    }
    
    /// Cota de bytes segura para recorrer/copiar `expr` como fuente de
    /// cadena (`LEN`/`MID$`/`LEFT$`/`RIGHT$`/`VAL`): para un elemento de
    /// array de cadena de ancho fijo (`DIM A$(N)*M`), el ancho real `M`
    /// (esos buffers no están NUL-terminados, así que el ancho declarado
    /// es el único límite fiable). Para cualquier otra fuente (variable
    /// escalar, literal — ambas NUL-terminadas), un tope genérico
    /// (`DEFAULT_STRING_MAX_LEN`), ya que no hay forma de conocer su
    /// longitud real en tiempo de compilación; el backend para de
    /// recorrer en el primer NUL de todas formas, así que este tope solo
    /// protege contra un buffer sin NUL cerca (p.ej. datos corruptos).
    /// Siempre acotado a `DEFAULT_STRING_MAX_LEN` para que quepa en los
    /// buffers de resultado compartidos de `LEFT$`/`RIGHT$`/`MID$`.
    fn string_source_max_len(&self, expr: &Expr) -> usize {
        let len = match expr.inner() {
            ExprInner::Parentheses(inner) => return self.string_source_max_len(inner),
            ExprInner::LValue(lvalue) => match &lvalue.inner {
                LValueInner::Array1DAccess { identifier, .. }
                | LValueInner::Array2DAccess { identifier, .. } if identifier.has_dollar() => {
                    self.array_metadata.get(&identifier.to_string()).map(|m| m.element_size)
                }
                _ => None,
            },
            _ => None,
        };
        len.unwrap_or(DEFAULT_STRING_MAX_LEN).min(DEFAULT_STRING_MAX_LEN)
    }

    /// es_designador(Exp):
    ///   return Exp es variable, array, o indirección
    fn es_designador(&self, expr: &Expr) -> bool {
        match expr.inner() {
            ExprInner::LValue(_) => true,
            // Los paréntesis transparentes - mirar dentro
            ExprInner::Parentheses(inner) => self.es_designador(inner),
            _ => false,
        }
    }

    /// ¿Es `expr` una expresión de tipo cadena? No hay tabla de tipos real
    /// (ver el comentario de `is_real` en `gen_binary_op`), así que se usa
    /// la misma señal que el propio BASIC: el sufijo `$` es una
    /// declaración de tipo del identificador, más fiable que intentar
    /// inferir el tipo de una expresión arbitraria. Usado para elegir
    /// comparación de cadenas (`IgualCadena`/`DistintoCadena`) en vez de
    /// entera en `gen_binary_op`.
    /// ¿Es `lvalue` un designador de tipo cadena (`A$`, `A$(n)`, `INKEY$`,
    /// ...)? Factorizado de `is_string_expr` para poder usarlo también
    /// sobre un `LValue` suelto (p.ej. el destino de `INPUT`, que no
    /// siempre viene envuelto en un `Expr`).
    fn is_string_lvalue(&self, lvalue: &LValue) -> bool {
        match &lvalue.inner {
            LValueInner::Identifier(id) => id.has_dollar(),
            LValueInner::Array1DAccess { identifier, .. }
            | LValueInner::Array2DAccess { identifier, .. } => identifier.has_dollar(),
            LValueInner::BuiltInIdentifier(keyword) => *keyword == Keyword::InkeyDollar,
            LValueInner::FixedMemoryAreaAccess { has_dollar, .. } => *has_dollar,
        }
    }

    fn is_string_expr(&self, expr: &Expr) -> bool {
        match expr.inner() {
            ExprInner::StringLiteral { .. } => true,
            ExprInner::Parentheses(inner) => self.is_string_expr(inner),
            ExprInner::LValue(lvalue) => self.is_string_lvalue(lvalue),
            ExprInner::FunctionCall(func) => matches!(
                func.inner,
                FunctionInner::Mid { .. }
                    | FunctionInner::Left { .. }
                    | FunctionInner::Right { .. }
                    | FunctionInner::Chr { .. }
                    | FunctionInner::Str { .. }
            ),
            // `A$+B$` (concatenación, ver `ConcatString` en `gen_binary_op`)
            // es una expresión de cadena igual que cualquier otra — sin
            // este caso, un consumidor que pregunte "¿esto es cadena?"
            // sobre la expresión COMPLETA (p.ej. `PRINT A$+"Y"`, que
            // decide `SystemOutString` vs `SystemOutInt` mirando
            // `is_string_expr` del `Expr` entero, no del `LValue`) nunca
            // vería la concatenación como cadena e imprimiría el puntero
            // de 16 bits del resultado como si fuera un entero pequeño.
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                self.is_string_expr(left) || self.is_string_expr(right)
            }
            _ => false,
        }
    }

    /// ¿Es `lvalue` un designador de una variable escalar real? Una
    /// variable se considera real si `collect_real_variables` la marcó así
    /// antes de generar código (ver el comentario de `real_variables`) —
    /// nunca un array (los arrays numéricos de este backend son siempre de
    /// 1 byte por elemento, ver `array_element_size`).
    fn is_real_lvalue(&self, lvalue: &LValue) -> bool {
        match &lvalue.inner {
            LValueInner::Identifier(id) if !id.has_dollar() => {
                self.real_variables.contains(&id.to_string())
            }
            _ => false,
        }
    }

    /// ¿Es `expr` una expresión de tipo real? Una expresión es real si
    /// contiene, en algún punto, un literal con parte decimal (p.ej. `.5`,
    /// `10.5`) combinado mediante operadores aritméticos — coincide con la
    /// misma condición (`fract() != 0.0`) que decide `ApilaInt` vs
    /// `ApilaReal` para un literal suelto en `gen_expression` — o una
    /// referencia a una variable ya marcada real por `collect_real_variables`
    /// (ver `real_variables`/`is_real_lvalue`): sin esto último, leer una
    /// variable real en una sentencia que no repite el literal decimal
    /// (p.ej. `X=B` tras `B=B+.5` en otra línea) volvería a tratarla como
    /// entera de 8 bits e interpretaría mal sus 8 bytes reales. Las
    /// llamadas a función (`SGN`, `INT`, ...) NO se propagan como reales:
    /// siempre devuelven un entero pequeño que ya cabe en el modelo de
    /// enteros existente (ver `CallSgn`/`CallInt` en el backend), así que
    /// no hace falta `Real2Int` genérico.
    fn is_real_expr(&self, expr: &Expr) -> bool {
        match expr.inner() {
            ExprInner::DecimalNumber(num) => num.as_f64().fract() != 0.0,
            ExprInner::Parentheses(inner) => self.is_real_expr(inner),
            ExprInner::Unary(_, operand) => self.is_real_expr(operand),
            ExprInner::LValue(lvalue) => self.is_real_lvalue(lvalue),
            ExprInner::Binary(left, op, right)
                if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div) =>
            {
                self.is_real_expr(left) || self.is_real_expr(right)
            }
            // `SQR` siempre devuelve un real (ver `FunctionInner::Sqr`),
            // sin importar si el argumento era entero o real — a
            // diferencia de `SGN`/`INT`, que sí devuelven enteros pequeños
            // (ver el comentario de arriba). Sin este caso, `B=SQR(A)`
            // hacía que `collect_real_variables` NO marcara `B` como real,
            // y `gen_store_to_lvalue` guardaba el resultado con
            // `DesapilaInd` (1 byte) en vez de `DesapilaIndReal` (8 bytes)
            // — los 7 bytes restantes del real calculado se quedaban en la
            // pila hardware para siempre, desincronizándola.
            // `STATUS n` también siempre real (ver `FunctionInner::Status`
            // más abajo, que empuja `ApilaReal` en todos los casos —
            // aunque el valor devuelto sea conceptualmente una dirección
            // de memoria de 16 bits, cabe sin problema en el formato BCD
            // real, y evita el mismo bug de truncado a 1 byte que `Sqr`
            // ya tuvo).
            ExprInner::FunctionCall(func) => {
                matches!(func.inner, FunctionInner::Sqr { .. } | FunctionInner::Status { .. })
            }
            _ => false,
        }
    }

    /// ¿Es `lvalue` una variable "de palabra" (ver `word_variables`)?
    /// Mismo patrón que `is_real_lvalue`.
    fn is_word_lvalue(&self, lvalue: &LValue) -> bool {
        match &lvalue.inner {
            LValueInner::Identifier(id) if !id.has_dollar() => {
                self.word_variables.contains(&id.to_string())
            }
            _ => false,
        }
    }

    /// ¿Es `expr` una expresión "de palabra" (entera de 16 bits, ver el
    /// comentario de `word_variables`)? Real siempre gana (una variable
    /// real ya cubre cualquier rango entero, nunca hace falta tratarla
    /// además como "de palabra") — de ahí el corte al principio. Una
    /// referencia a una variable ya marcada, un literal fuera de
    /// 0..=255 (vía `const_eval_int`, el mismo criterio que usa
    /// `ApilaInt` en `gen_expression` para elegir su propia rama de 16
    /// bits) o una suma (`+`, contagiosa como en `is_real_expr`) donde
    /// cualquiera de los dos lados ya lo sea. Deliberadamente NO incluye
    /// resta/multiplicación/división/comparación — ver el comentario de
    /// `word_variables` para el porqué (sin caso real en el corpus,
    /// `gen_binary_op` las trata con `TruncateWordToInt` si hiciera
    /// falta en vez de generar una rama de 16 bits para ellas).
    fn is_word_expr(&self, expr: &Expr) -> bool {
        if self.is_real_expr(expr) {
            return false;
        }
        match expr.inner() {
            ExprInner::Parentheses(inner) => self.is_word_expr(inner),
            ExprInner::LValue(lvalue) => self.is_word_lvalue(lvalue),
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                self.is_word_expr(left) || self.is_word_expr(right)
            }
            _ => matches!(self.const_eval_int(expr), Some(n) if !(0..=255).contains(&n)),
        }
    }

    // =========================================================================
    // HELPERS GENERALES
    // =========================================================================
    
    /// Convertir una expresión a etiqueta (para GOTO/GOSUB)
    fn expr_to_label(&mut self, expr: &Expr) -> String {
        match expr.inner() {
            // Número de línea directo: LINE_100
            ExprInner::DecimalNumber(num) => {
                format!("LINE_{}", num.as_f64())
            }
            // Etiqueta de string: "CRASH" → CRASH
            ExprInner::StringLiteral { value, .. } => {
                value.clone()
            }
            // Para expresiones complejas, necesitaríamos evaluación dinámica
            // (no implementado aún - requeriría GOTO calculado)
            _ => {
                // Por ahora, crear etiqueta temporal
                // TODO: Implementar saltos calculados (GOTO variable)
                self.new_label("TARGET")
            }
        }
    }
    
    /// Obtener o crear dirección para una variable
    /// Asigna direcciones en el área de datos (0x4000+)
    fn get_or_create_variable_address(&mut self, name: &str) -> usize {
        if let Some(&addr) = self.variable_addresses.get(name) {
            addr
        } else {
            // Una variable de cadena escalar (`Z$`) reserva su propio
            // buffer de `DEFAULT_STRING_MAX_LEN+1` bytes (caracteres +
            // NUL) en vez del hueco genérico de 10 — ver el comentario de
            // `is_direct_string_buffer`: es lo que permite que
            // `DesapilaIndStringCopy` copie ahí el contenido real en vez
            // de solo un puntero a un buffer compartido y transitorio.
            let size = if name.ends_with('$') { DEFAULT_STRING_MAX_LEN + 1 } else { 10 };
            let addr = self.data_base + self.next_address;
            self.variable_addresses.insert(name.to_string(), addr);
            self.next_address += size;
            addr
        }
    }

    /// Como `get_or_create_variable_address`, pero reserva exactamente
    /// `total_bytes` en vez del hueco fijo de 10 bytes por variable
    /// escalar — para arrays, cuyo tamaño real varía por declaración.
    ///
    /// Bug real encontrado testeando invader-v2.bas: un array `B(n)` y
    /// una variable ESCALAR `B` (ambas nombradas igual, patrón real y
    /// válido en BASIC — la sintaxis ya las distingue por los
    /// paréntesis: `6 DIM ...,B(5)` y luego `95 ...,B=L-4,...` en el
    /// mismo programa) compartían literalmente la misma dirección,
    /// porque esta función y `get_or_create_variable_address` usaban el
    /// MISMO `HashMap<String, usize>` indexado solo por el nombre
    /// crudo — quien se registrara primero "ganaba" la dirección, y el
    /// otro la reutilizaba como si fuera la misma variable. Cada
    /// asignación a la `B` escalar (controla cuántas veces itera `FOR
    /// D=1 TO B`, el bucle que genera/desplaza el terreno) corrompía en
    /// silencio `B(0)`, y viceversa — confirmado con un test aislado
    /// (`DIM B(5):B(0)=99:B=7:...` dejaba `B(0)` en 7, no en 99).
    /// Arreglado con un namespace interno distinto (`"ARRAY:"` + nombre)
    /// para la clave del `HashMap`, exclusivo de este método — los
    /// nombres de los buffers de scratch internos (`__SQR_V`, etc.) ya
    /// no podían colisionar con un nombre BASIC real de todos modos
    /// (BASIC no permite identificadores que empiecen por `__`), así que
    /// aplicar el namespace aquí de forma uniforme es seguro.
    fn get_or_create_array_address(&mut self, name: &str, total_bytes: usize) -> usize {
        let key = format!("ARRAY:{name}");
        if let Some(&addr) = self.variable_addresses.get(&key) {
            addr
        } else {
            let addr = self.data_base + self.next_address;
            self.variable_addresses.insert(key, addr);
            self.next_address += total_bytes;
            addr
        }
    }

    /// Evalúa `expr` como constante entera en tiempo de compilación si es
    /// un literal numérico directo. Deliberadamente simple (no hace
    /// constant-folding de expresiones como `2+3`): es lo que necesita el
    /// núcleo reducido de `DIM` para tamaños declarados como literales,
    /// que es como aparecen en la inmensa mayoría de programas reales.
    /// También reconoce literales hexadecimales `&XXXX` (`BinaryNumber`,
    /// pese al nombre histórico — es hex, no binario), el formato real que
    /// usan las direcciones de memoria en programas con código máquina
    /// embebido vía `POKE`/`CALL` (ver `gen_call`).
    fn const_eval_int(&self, expr: &Expr) -> Option<i64> {
        match expr.inner() {
            ExprInner::DecimalNumber(n) => n.as_integer(),
            ExprInner::BinaryNumber(n) => Some(n.as_u16() as i64),
            ExprInner::Parentheses(inner) => self.const_eval_int(inner),
            _ => None,
        }
    }
    
    /// Emitir una instrucción
    fn emit(&mut self, instr: StackInstruction) {
        self.instructions.push(instr);
    }
    
    /// Emitir un comentario
    fn emit_comment(&mut self, comment: &str) {
        self.emit(StackInstruction::Comment(comment.to_string()));
    }
    
    /// Generar una nueva etiqueta única
    fn new_label(&mut self, prefix: &str) -> String {
        let label = format!("{}_{}", prefix, self.label_counter);
        self.label_counter += 1;
        label
    }
    
    /// Direcciones asignadas a cada variable (nombre BASIC tal y como lo
    /// usa `get_or_create_variable_address`/`get_or_create_array_address`,
    /// p.ej. `"H"`, `"A$"`, `"__RND_SEED"`) — solo tiene sentido consultarlo
    /// tras `generate()`. Usado por tests que necesitan inspeccionar el
    /// valor real de una variable en memoria sin asumir una dirección fija
    /// (ver [`compile_native_two_pass`]: `data_base` varía según el tamaño
    /// del programa compilado).
    pub fn variable_addresses(&self) -> &HashMap<String, usize> {
        &self.variable_addresses
    }

    /// Tamaño total, en bytes, de la región de variables de usuario
    /// asignada hasta ahora (offset desde `data_base` del siguiente hueco
    /// libre) — solo tiene sentido consultarlo tras `generate()`. Usado
    /// por [`compile_native_two_pass`] para que el prólogo del backend
    /// pueda poner a 0 esa región entera de una vez (ver
    /// `Lh5801Backend::set_variable_region`).
    pub fn total_variable_region_size(&self) -> usize {
        self.next_address
    }

    /// Convertir las instrucciones generadas a texto
    pub fn to_string(&self) -> String {
        self.instructions
            .iter()
            .map(|instr| instr.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Compila un `Program` ya parseado a código máquina LH5801 nativo,
/// calculando dónde empieza el área de variables (`DATA_BASE`) a partir
/// del tamaño real del código generado en vez de una constante fija — ver
/// el comentario histórico junto a [`ArrayMeta`]/`DEFAULT_DATA_BASE_PLACEHOLDER`
/// sobre por qué una constante fija ya causó corrupción de código en tiempo
/// de ejecución dos veces (una vez detectada y "arreglada" subiendo el
/// número; esta es la solución real).
///
/// Hace dos pasadas:
///   1. Genera con un `data_base` provisional, solo para medir
///      `código.len()`. Las direcciones de variable son operandos
///      inmediatos de 16 bits: su valor numérico no cambia cuántos bytes
///      ocupa la instrucción que los usa, así que el tamaño del código no
///      depende de qué `data_base` se use aquí.
///   2. Vuelve a generar desde cero (`StackCodeGenerator`/`Lh5801Backend`
///      son deterministas dado el mismo `Program`) con
///      `data_base = start_address + código.len()` — justo después de
///      donde termina el código real de la primera pasada — garantizando
///      que las variables nunca invadan código todavía no ejecutado, sea
///      cual sea el tamaño del programa.
pub fn compile_native_two_pass(
    program: &Program,
    start_address: u16,
    stack_top: u16,
) -> (u16, Vec<u8>, HashMap<String, usize>) {
    compile_native_two_pass_with_timing(program, start_address, stack_top, false)
}

/// Como [`compile_native_two_pass`], pero además controla el mecanismo
/// genérico de ritmo de ejecución (ver el comentario de
/// `StackCodeGenerator::authentic_timing`) — `compile_native_two_pass`
/// sigue existiendo, sin cambiar su firma, y llama aquí con
/// `authentic_timing=false`, así que cualquier llamador existente (todo
/// el código y los tests ya escritos antes de este mecanismo) sigue
/// produciendo exactamente el mismo `.lh5`, byte a byte.
pub fn compile_native_two_pass_with_timing(
    program: &Program,
    start_address: u16,
    stack_top: u16,
    authentic_timing: bool,
) -> (u16, Vec<u8>, HashMap<String, usize>) {
    use lh5801_backend::Lh5801Backend;

    let mut first_pass_gen = StackCodeGenerator::with_data_base_and_timing(
        DEFAULT_DATA_BASE_PLACEHOLDER,
        authentic_timing,
    );
    let first_pass_instructions = first_pass_gen.generate(program);
    let mut first_pass_backend = Lh5801Backend::with_config(start_address, stack_top);
    // La pasada 1 debe emitir el MISMO prólogo (mismo tamaño en bytes)
    // que emitirá la pasada 2, o `first_pass_code.len()` mide el tamaño
    // equivocado y `real_data_base` solapa con el propio código — mismo
    // bug de solape de `DATA_BASE` ya arreglado una vez, reintroducido
    // aquí al añadir el bucle de puesta a cero de variables SOLO en la
    // pasada 2. La dirección real de la región (`real_data_base`, que
    // esta misma pasada 1 todavía no conoce) es irrelevante para el
    // TAMAÑO del código — todos los operandos de dirección son
    // inmediatos de 16 bits de ancho fijo — así que basta con el mismo
    // TAMAÑO, con cualquier dirección de relleno (0).
    let first_pass_region_size = first_pass_gen.total_variable_region_size();
    if first_pass_region_size > 0 {
        let clamped = first_pass_region_size.min(u16::MAX as usize) as u16;
        first_pass_backend.set_variable_region(0, clamped);
    }
    let first_pass_code = first_pass_backend.generate(&first_pass_instructions);

    let real_data_base = start_address as usize + first_pass_code.len();

    let mut second_pass_gen =
        StackCodeGenerator::with_data_base_and_timing(real_data_base, authentic_timing);
    let second_pass_instructions = second_pass_gen.generate(program);
    let mut second_pass_backend = Lh5801Backend::with_config(start_address, stack_top);
    // Poner a 0 TODA la región de variables en el prólogo — ver el
    // comentario en `Lh5801Backend::set_variable_region`/`emit_initialization`
    // (bug real: la GUI reutiliza el mismo `Pc1500` al pulsar "Cargar",
    // así que la región de variables de una ejecución anterior podía
    // sobrevivir a la siguiente si el programa no la reseteaba él mismo).
    let variable_region_size = second_pass_gen.total_variable_region_size();
    if variable_region_size > 0 && real_data_base <= u16::MAX as usize {
        let clamped_size = variable_region_size.min(u16::MAX as usize) as u16;
        second_pass_backend.set_variable_region(real_data_base as u16, clamped_size);
    }
    let second_pass_code = second_pass_backend.generate(&second_pass_instructions);

    (start_address, second_pass_code, second_pass_gen.variable_addresses().clone())
}
