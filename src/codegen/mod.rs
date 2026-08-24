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
}

#[derive(Debug, Clone, Copy)]
enum ArrayDims {
    OneD { len: usize },
    TwoD { rows: usize, cols: usize },
}

/// Área de datos de usuario en memoria (variables escalares y, más
/// adelante, el heap de arrays).
///
/// Mapa de memoria real (PC-1500 **con expansión de RAM**, ver
/// `codegen::system_memory` y el doc de `codegen::lh5801_backend`):
/// `STANDARD_USER_MEMORY` mapea `0x4000-0x57FF` (6144 bytes), repartido en
/// ventanas disjuntas: código en `start_address..`
/// (`Lh5801Backend::start_address`), variables desde `data_base`
/// (ver [`compile_native_two_pass`]), pila propia desde `0x57FF` hacia
/// abajo (`Lh5801Backend::stack_top`).
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
}

impl StackCodeGenerator {
    pub fn new() -> Self {
        Self::with_data_base(DEFAULT_DATA_BASE_PLACEHOLDER)
    }

    /// Como `new()`, pero fijando explícitamente dónde empieza el área de
    /// variables — ver [`compile_native_two_pass`], que es quien de verdad
    /// calcula ese valor para código nativo.
    pub fn with_data_base(data_base: usize) -> Self {
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

        // Comentario inicial
        self.emit_comment("=== INICIO DEL PROGRAMA ===");
        self.emit_comment("");

        // Generar código para cada línea del programa
        for line in program.lines() {
            self.gen_code_line(line);
        }

        // Fin del programa
        self.emit(StackInstruction::Stop);

        self.instructions.clone()
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
            self.gen_statement(stmt);
        }
        
        self.emit_comment("");
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
            StatementInner::Using { using_clause } => self.gen_using(using_clause),
            
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
            // un IF sin bloque explícito.
            StatementInner::Multi(statements) => {
                for statement in statements {
                    self.gen_statement(statement);
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

            // Almacenar: desapila valor, desapila dirección, guarda
            self.gen_store_to_lvalue(lvalue);
        }
    }
    
    /// gen_cod(ins_write(Exp)):
    ///   gen_cod(Exp)
    ///   gen_acc_val(Exp)
    ///   emit systemout()
    fn gen_print(&mut self, print_inner: &PrintInner) {
        for (printable, sep) in &print_inner.exprs {
            // Generar código para el elemento a imprimir
            match printable {
                crate::parse::statement::printable::Printable::Expr(expr) => {
                    self.gen_expression(expr);
                    self.gen_acc_val(expr);
                    if self.is_string_expr(expr) {
                        self.emit(StackInstruction::SystemOutString);
                    } else {
                        self.emit(StackInstruction::SystemOutInt);
                    }
                }
                crate::parse::statement::printable::Printable::UsingClause(_using) => {
                    // TODO: Implementar soporte para USING clause
                    self.emit_comment("USING clause no soportado aún");
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
        
        // Código del THEN
        self.gen_statement(then_stmt);
        
        // Etiqueta de fin
        self.emit(StackInstruction::Label(end_label));
    }
    
    /// Generar código para GOTO
    fn gen_goto(&mut self, target: &Expr) {
        self.emit_comment("GOTO");

        match self.static_goto_label(target) {
            Some(label) => self.emit(StackInstruction::IrA(label)),
            None => {
                self.gen_dynamic_line_number(target);
                self.emit(StackInstruction::IrIndirect);
            }
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
            None => {
                self.gen_dynamic_line_number(target);
                self.emit(StackInstruction::CallIndirect);
            }
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

    /// Genera código que deja en la pila el valor de `expr` como entero
    /// de 16 bits, para usarlo como número de línea calculado (`GOTO`/
    /// `GOSUB <expr>`, o `RESTORE <expr>` vía `gen_restore`). Reconoce
    /// `<constante>+<expresión>` (aritmética de 16 bits real, para bases
    /// de línea grandes con un desplazamiento pequeño, p.ej. `GOSUB
    /// C+10`) y, si no encaja ese patrón, evalúa la expresión como
    /// entero normal de 8 bits y lo extiende con ceros a 16 bits (cubre
    /// el caso común de una variable con un número de línea pequeño,
    /// ≤255, p.ej. `GOTO D`).
    fn gen_dynamic_line_number(&mut self, expr: &Expr) {
        if let Some((base, dynamic_part)) = self.dynamic_line_number_base_and_offset(expr) {
            self.emit(StackInstruction::ApilaIntWord(base));
            self.gen_expression(dynamic_part);
            self.gen_acc_val(dynamic_part);
            self.emit(StackInstruction::SumaIntWord);
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
                while self.for_stack.len() > pos + 1 {
                    let stale = self.for_stack.pop().expect("acabamos de comprobar que hay más de pos+1 elementos");
                    self.emit(StackInstruction::Label(stale.loop_end));
                }
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
                            let element_size = self.array_element_size(string_length);
                            let name = identifier.to_string();
                            let base_addr = self.get_or_create_array_address(&name, element_count * element_size);
                            self.array_metadata.insert(name, ArrayMeta {
                                base_addr,
                                element_size,
                                dims: ArrayDims::OneD { len: element_count },
                            });
                        }
                        _ => {
                            self.emit_comment("DIM con tamaño dinámico (no constante): no soportado todavía, no se reserva espacio real");
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
                            let element_size = self.array_element_size(string_length);
                            let name = identifier.to_string();
                            let base_addr = self.get_or_create_array_address(
                                &name,
                                row_count * col_count * element_size,
                            );
                            self.array_metadata.insert(name, ArrayMeta {
                                base_addr,
                                element_size,
                                dims: ArrayDims::TwoD { rows: row_count, cols: col_count },
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
    fn array_element_size(&self, string_length: &Option<Expr>) -> usize {
        string_length
            .as_ref()
            .and_then(|e| self.const_eval_int(e))
            .filter(|&n| n > 0)
            .map_or(1, |n| n as usize)
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
    
    fn gen_clear(&mut self) {
        self.emit_comment("CLEAR");
        self.emit(StackInstruction::Clear);
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
        if let Some(e) = expr {
            self.gen_expression(e);
            self.gen_acc_val(e);
        } else {
            self.emit(StackInstruction::ApilaInt(0));
        }
        self.emit(StackInstruction::Wait);
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
        // PAUSE es similar a PRINT pero pausa después
        for (printable, sep) in &print_inner.exprs {
            match printable {
                crate::parse::statement::printable::Printable::Expr(expr) => {
                    self.gen_expression(expr);
                    self.gen_acc_val(expr);
                    if self.is_string_expr(expr) {
                        self.emit(StackInstruction::SystemOutString);
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
    
    fn gen_using(&mut self, _using_clause: &UsingClause) {
        self.emit_comment("USING");
        self.emit(StackInstruction::Using);
    }
    
    fn gen_lf(&mut self, expr: &Expr) {
        self.emit_comment(&format!("LF {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        // LF imprime valor y luego line feed
        if self.is_string_expr(expr) {
            self.emit(StackInstruction::SystemOutString);
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
                    None => self.emit_comment(
                        "GPRINT de cadena con longitud no determinable en tiempo de \
                         compilación (p.ej. variable escalar): no soportado todavía",
                    ),
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
    /// `array_metadata`. Variables de cadena escalares (puntero a un
    /// buffer NUL-terminado de longitud dinámica) no están soportadas —
    /// ninguno de los programas objetivo usa `GPRINT` sobre una variable
    /// de cadena escalar, solo sobre literales y arrays de ancho fijo
    /// (ver bathyscaph.bas: `GPRINT A$(0)` / `GPRINT "141C"`).
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
    
    fn gen_poke(&mut self, memory_area: &MemoryArea, exprs: &[Expr]) {
        self.emit_comment(&format!("POKE {:?}", memory_area));

        // La dirección en POKE es absoluta, no relativa a una base
        // POKE dirección, valor: apilar dirección, apilar valor, ejecutar poke
        // La diferencia entre Me0 y Me1 (POKE vs POKE#) es solo semántica
        // en la ROM real (memoria normal vs espacio de E/S) — ninguna
        // dirección de sistema que nos interesa vive en espacio de E/S,
        // así que ambas se tratan igual aquí (escritura directa a
        // memoria absoluta).

        if exprs.len() >= 2 {
            // Primera expresión: dirección absoluta. Siempre de 16 bits
            // (una dirección de memoria nunca cabe en 1 byte) — un
            // literal constante se apila con ApilaIntWord (nunca
            // ApilaInt, que elegiría 1 byte para valores <=255 y
            // descuadraría la pila). Dirección dinámica (variable, no
            // constante) no soportada todavía, documentado como límite
            // conocido (igual que RESTORE con línea calculada).
            match self.const_eval_int(&exprs[0]) {
                Some(n) if (0..=0xFFFF).contains(&n) => {
                    self.emit(StackInstruction::ApilaIntWord(n));
                }
                _ => {
                    self.emit_comment(
                        "POKE con dirección dinámica (no constante): no soportado todavía \
                         (necesitaría aritmética de 16 bits en tiempo de ejecución)",
                    );
                    self.gen_expression(&exprs[0]);
                    self.gen_acc_val(&exprs[0]);
                }
            }

            // Segunda expresión: valor a escribir (1 byte).
            self.gen_expression(&exprs[1]);
            self.gen_acc_val(&exprs[1]);

            self.emit(StackInstruction::Poke);
        } else {
            // Si hay menos de 2 expresiones, es un error de sintaxis
            // pero generamos algo para no romper
            self.emit_comment("ERROR: POKE requiere dirección y valor");
        }
    }
    
    fn gen_call(&mut self, expr: &Expr, variable: &Option<LValue>) {
        self.emit_comment(&format!("CALL {}", expr.show(false)));
        self.gen_expression(expr);
        self.gen_acc_val(expr);
        
        // Si hay variable de retorno, generar asignación después
        if let Some(_var) = variable {
            self.emit_comment("CALL with return variable");
        }
        
        self.emit(StackInstruction::Call("MACHINE_CODE".to_string()));
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
        // Para las operaciones aritméticas (no comparaciones/lógicas, que
        // bathyscaph.bas nunca usa sobre reales), un operando "real" es
        // contagioso: si CUALQUIERA de los dos lados contiene un literal
        // decimal (ver `is_real_expr`), toda la operación pasa a BCD real
        // — el otro lado, si era entero, se promociona con `Int2Real`
        // justo después de evaluarlo (mismo patrón que `ApilaInt` vs
        // `ApilaReal` para un literal suelto).
        let is_real = matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div)
            && (self.is_real_expr(left) || self.is_real_expr(right));

        // Generar código para ambos operandos
        self.gen_expression(left);
        self.gen_acc_val(left);
        if is_real && !self.is_real_expr(left) {
            self.emit(StackInstruction::Int2Real);
        }
        self.gen_expression(right);
        self.gen_acc_val(right);
        if is_real && !self.is_real_expr(right) {
            self.emit(StackInstruction::Int2Real);
        }

        match op {
            // Operaciones aritméticas
            BinaryOp::Add => {
                if is_real {
                    self.emit(StackInstruction::SumaReal);
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

            // Operaciones de comparación (siempre enteras/cadena: ningún
            // programa objetivo compara valores reales directamente).
            BinaryOp::Eq => {
                if self.is_string_expr(left) || self.is_string_expr(right) {
                    self.emit(StackInstruction::IgualCadena);
                } else {
                    self.emit(StackInstruction::IgualInt);
                }
            }
            BinaryOp::Neq => {
                if self.is_string_expr(left) || self.is_string_expr(right) {
                    self.emit(StackInstruction::DistintoCadena);
                } else {
                    self.emit(StackInstruction::DistintoInt);
                }
            }
            BinaryOp::Lt => {
                self.emit(StackInstruction::MenorInt);
            }
            BinaryOp::Leq => {
                self.emit(StackInstruction::MenorIgualInt);
            }
            BinaryOp::Gt => {
                self.emit(StackInstruction::MayorInt);
            }
            BinaryOp::Geq => {
                self.emit(StackInstruction::MayorIgualInt);
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
            FunctionInner::Sqr { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallSqr);
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
            
            // Funciones de sistema
            FunctionInner::Status { arg } => {
                self.gen_expression(arg);
                self.gen_acc_val(arg);
                self.emit(StackInstruction::CallStatus);
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
    /// - Variable numérica o array numérico: `DesapilaInd` (1 byte).
    /// - Variable de cadena escalar (`Z$`, sin DIM de ancho fijo):
    ///   `DesapilaIndWord` — el valor es un puntero de 16 bits.
    /// - Elemento de array de cadena con ancho fijo declarado (`DIM
    ///   A$(N)*M`, `M>2`): `DesapilaIndStringCopy(M)` — copia los
    ///   caracteres al buffer reservado, no sobreescribe un puntero.
    fn gen_store_to_lvalue(&mut self, lvalue: &LValue) {
        let instr = match &lvalue.inner {
            LValueInner::Identifier(id) if id.has_dollar() => StackInstruction::DesapilaIndWord,
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
                // gen_dim; si no (array sin DIM, o con tamaño dinámico
                // todavía no soportado), cae al valor histórico de 5 bytes
                // como límite conocido, no una constante "correcta".
                let name = identifier.to_string();
                let base_addr = self.get_or_create_variable_address(&name);
                let element_size = self.array_metadata.get(&name).map_or(5, |m| m.element_size);
                self.emit(StackInstruction::ApilaInt(base_addr as i64));

                // Índice
                self.gen_expression(index);
                self.gen_acc_val(index);

                self.emit(StackInstruction::ApilaInt(element_size as i64));
                self.emit(StackInstruction::MulInt);

                // Dirección = base + índice * tamaño
                self.emit(StackInstruction::SumaInt);
            }

            // Array 2D
            LValueInner::Array2DAccess { identifier, row_index, col_index } => {
                // Dirección base. Misma lógica que Array1DAccess: usa las
                // dimensiones reales del DIM si están registradas, si no
                // cae a los valores históricos (10 columnas, 5 bytes/elem).
                let name = identifier.to_string();
                let base_addr = self.get_or_create_variable_address(&name);
                let (col_count, element_size) = match self.array_metadata.get(&name) {
                    Some(ArrayMeta { dims: ArrayDims::TwoD { cols, .. }, element_size, .. }) => {
                        (*cols, *element_size)
                    }
                    _ => (10, 5),
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

                // Sumar a la base
                self.emit(StackInstruction::SumaInt);
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
        if self.is_fixed_width_string_array_access(expr) {
            // Un elemento de array de cadena de ancho fijo (`DIM
            // A$(N)*M`) es el buffer en sí — sus caracteres viven ahí
            // directamente, no hay un puntero intermedio que cargar. La
            // dirección que ya dejó gen_lvalue_address/gen_expression es
            // el "valor" a efectos de comparación/paso a funciones, así
            // que no se emite nada más aquí.
        } else if self.is_string_expr(expr) {
            // Una variable de cadena escalar (`Z$`) guarda un puntero de
            // 16 bits, así que cargarla necesita ApilaIndWord, no
            // ApilaInd (8 bits) — usarla ahí perdería el byte alto del
            // puntero y descuadraría la pila (mismo bug que DesapilaInd
            // tenía para el lado de escritura, aquí en el de lectura).
            self.emit(StackInstruction::ApilaIndWord);
        } else {
            self.emit(StackInstruction::ApilaInd);
        }
    }

    /// ¿Es `expr` el acceso a un elemento de un array de cadena de ancho
    /// fijo (`DIM A$(N)*M`, `M>2`)? Ver `gen_acc_val`.
    fn is_fixed_width_string_array_access(&self, expr: &Expr) -> bool {
        match expr.inner() {
            ExprInner::Parentheses(inner) => self.is_fixed_width_string_array_access(inner),
            ExprInner::LValue(lvalue) => match &lvalue.inner {
                LValueInner::Array1DAccess { identifier, .. }
                | LValueInner::Array2DAccess { identifier, .. } => {
                    identifier.has_dollar()
                        && self
                            .array_metadata
                            .get(&identifier.to_string())
                            .is_some_and(|m| m.element_size > 2)
                }
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
            _ => false,
        }
    }

    /// ¿Es `expr` una expresión de tipo real? No hay tabla de tipos real
    /// (las variables numéricas siempre se tratan como enteras de 8 bits,
    /// ver `is_real` en `gen_binary_op`): una expresión es "real" solo si
    /// contiene, en algún punto, un literal con parte decimal (p.ej. `.5`,
    /// `10.5`) combinado mediante operadores aritméticos — coincide con la
    /// misma condición (`fract() != 0.0`) que decide `ApilaInt` vs
    /// `ApilaReal` para un literal suelto en `gen_expression`. Las llamadas
    /// a función (`SGN`, `INT`, ...) NO se propagan como reales: siempre
    /// devuelven un entero pequeño que ya cabe en el modelo de enteros
    /// existente (ver `CallSgn`/`CallInt` en el backend), así que no hace
    /// falta `Real2Int` genérico.
    fn is_real_expr(&self, expr: &Expr) -> bool {
        match expr.inner() {
            ExprInner::DecimalNumber(num) => num.as_f64().fract() != 0.0,
            ExprInner::Parentheses(inner) => self.is_real_expr(inner),
            ExprInner::Unary(_, operand) => self.is_real_expr(operand),
            ExprInner::Binary(left, op, right)
                if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div) =>
            {
                self.is_real_expr(left) || self.is_real_expr(right)
            }
            _ => false,
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
            let addr = self.data_base + self.next_address;
            self.variable_addresses.insert(name.to_string(), addr);
            self.next_address += 10; // Espacio genérico por variable
            addr
        }
    }

    /// Como `get_or_create_variable_address`, pero reserva exactamente
    /// `total_bytes` en vez del hueco fijo de 10 bytes por variable
    /// escalar — para arrays, cuyo tamaño real varía por declaración.
    fn get_or_create_array_address(&mut self, name: &str, total_bytes: usize) -> usize {
        if let Some(&addr) = self.variable_addresses.get(name) {
            addr
        } else {
            let addr = self.data_base + self.next_address;
            self.variable_addresses.insert(name.to_string(), addr);
            self.next_address += total_bytes;
            addr
        }
    }

    /// Evalúa `expr` como constante entera en tiempo de compilación si es
    /// un literal numérico directo. Deliberadamente simple (no hace
    /// constant-folding de expresiones como `2+3`): es lo que necesita el
    /// núcleo reducido de `DIM` para tamaños declarados como literales,
    /// que es como aparecen en la inmensa mayoría de programas reales.
    fn const_eval_int(&self, expr: &Expr) -> Option<i64> {
        match expr.inner() {
            ExprInner::DecimalNumber(n) => n.as_integer(),
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
    use lh5801_backend::Lh5801Backend;

    let mut first_pass_gen = StackCodeGenerator::new();
    let first_pass_instructions = first_pass_gen.generate(program);
    let mut first_pass_backend = Lh5801Backend::with_config(start_address, stack_top);
    let first_pass_code = first_pass_backend.generate(&first_pass_instructions);

    let real_data_base = start_address as usize + first_pass_code.len();

    let mut second_pass_gen = StackCodeGenerator::with_data_base(real_data_base);
    let second_pass_instructions = second_pass_gen.generate(program);
    let mut second_pass_backend = Lh5801Backend::with_config(start_address, stack_top);
    let second_pass_code = second_pass_backend.generate(&second_pass_instructions);

    (start_address, second_pass_code, second_pass_gen.variable_addresses().clone())
}
