pub mod stack_instruction;
pub mod interpreter;
pub mod lh5801_backend;
pub mod pc1500_tokenizer;
pub mod lh5_format;
pub mod rom_routines;

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
}

/// Área de datos de usuario en memoria
/// Mapa de memoria del PC-1500:
/// - 0x0000-0x37FF: ROM y sistema
/// - 0x3800-0x57FF: Código de usuario (8KB)
/// - 0x5800-0x5FEF: Stack (752 bytes)
/// - 0x5FF0-0x5FF1: Stack pointer (2 bytes)
const DATA_BASE: usize = 0x4000;

/// Contexto de un bucle FOR
struct ForContext {
    loop_start: String,
    loop_end: String,
}

impl StackCodeGenerator {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            label_counter: 0,
            symbol_table: HashMap::new(),
            variable_addresses: HashMap::new(),
            next_address: 0,
            for_stack: Vec::new(),
        }
    }
    
    /// Generar código para el programa completo
    /// gen_cod(prog(Bloq)):
    ///   gen_cod(Bloq)
    ///   emit stop()
    pub fn generate(&mut self, program: &Program) -> Vec<StackInstruction> {
        self.instructions.clear();
        self.label_counter = 0;
        
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
            self.emit(StackInstruction::DesapilaInd);
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
                    self.emit(StackInstruction::SystemOut);
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
                self.emit(StackInstruction::SystemOut);
            }
            
            // Generar dirección de la variable
            self.gen_lvalue_address(lvalue);
            
            // Leer entrada y almacenar
            self.emit(StackInstruction::SystemIn);
            self.emit(StackInstruction::DesapilaInd);
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
        
        // Calcular etiqueta destino
        let label = self.expr_to_label(target);
        self.emit(StackInstruction::IrA(label));
    }
    
    /// Generar código para GOSUB (llamada a subrutina)
    /// gen_cod(ins_call(Id, LParamsR)):
    ///   emit activa($.vinculo.nivel, $.vinculo.tam, $.sig)
    ///   emit ir-a($.vinculo.prim)
    fn gen_gosub(&mut self, target: &Expr) {
        self.emit_comment("GOSUB");
        
        let label = self.expr_to_label(target);
        
        // En BASIC simple, GOSUB usa el stack de retorno implícito
        // Guardar dirección de retorno
        self.emit(StackInstruction::Call(label));
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
    fn gen_for(&mut self, assignment: &Assignment, to_expr: &Expr, _step_expr: &Option<Expr>) {
        self.emit_comment(&format!("FOR {} TO {}", 
            assignment.lvalue().show(false), 
            to_expr.show(false)));
        
        let loop_start = self.new_label("FOR_INICIO");
        let loop_end = self.new_label("FOR_FIN");
        
        // Guardar contexto en el stack de FORs
        self.for_stack.push(ForContext {
            loop_start: loop_start.clone(),
            loop_end: loop_end.clone(),
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
        
        // Cargar variable de control
        self.gen_lvalue_load(assignment.lvalue());
        
        // Cargar límite
        self.gen_expression(to_expr);
        self.gen_acc_val(to_expr);
        
        // Comparar: si variable > límite, salir
        // TODO: Considerar signo del step
        self.emit(StackInstruction::MayorInt);
        self.emit(StackInstruction::IrV(loop_end.clone()));
        
        // El cuerpo del FOR se genera entre FOR y NEXT
        // Por ahora, solo preparamos las etiquetas
        
        // NOTA: El cuerpo real se procesará cuando encontremos las líneas
        // entre FOR y NEXT en el bucle principal
    }
    
    /// Generar código para NEXT
    fn gen_next(&mut self, lvalue: &LValue) {
        self.emit_comment(&format!("NEXT {}", lvalue.show(false)));
        
        // Recuperar contexto del FOR correspondiente
        let for_context = self.for_stack.pop()
            .expect("NEXT sin FOR correspondiente");
        
        // Apilar dirección primero (modelo Tiny)
        self.gen_lvalue_address(lvalue);
        
        // Incrementar variable de control
        self.gen_lvalue_load(lvalue);
        
        // TODO: Usar el step correcto
        self.emit(StackInstruction::ApilaInt(1));  // step por defecto
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
    fn gen_dim(&mut self, decls: &[DimInner]) {
        for decl in decls {
            match decl {
                DimInner::DimInner1D { identifier, size, .. } => {
                    self.emit_comment(&format!("DIM {}({})", 
                        identifier.to_string(), 
                        size.show(false)));
                    
                    // Calcular tamaño total del array
                    self.gen_expression(size);
                    self.gen_acc_val(size);
                    
                    // Reservar espacio (simplificado)
                    // En una implementación completa, esto afectaría a la tabla de símbolos
                }
                DimInner::DimInner2D { identifier, rows, cols, ..} => {
                    self.emit_comment(&format!("DIM {}({}, {})", 
                        identifier.to_string(), 
                        rows.show(false),
                        cols.show(false)));
                    
                    // Calcular tamaño total: rows * cols
                    self.gen_expression(rows);
                    self.gen_acc_val(rows);
                    self.gen_expression(cols);
                    self.gen_acc_val(cols);
                    self.emit(StackInstruction::MulInt);
                }
            }
        }
    }
    
    /// Generar código para READ
    fn gen_read(&mut self, destinations: &[LValue]) {
        for lvalue in destinations {
            self.emit_comment(&format!("READ {}", lvalue.show(false)));
            
            // Leer siguiente dato de DATA
            self.gen_lvalue_address(lvalue);
            self.emit(StackInstruction::ReadData);
            self.emit(StackInstruction::DesapilaInd);
        }
    }
    
    /// Generar código para DATA
    fn gen_data(&mut self, exprs: &[Expr]) {
        // DATA no genera código ejecutable, solo almacena valores
        self.emit_comment("DATA (valores almacenados)");
        
        for expr in exprs {
            // Estos valores se almacenarían en una sección de datos separada
            self.gen_expression(expr);
            self.gen_acc_val(expr);
            self.emit(StackInstruction::StoreData);
        }
    }
    
    /// Generar código para RESTORE
    fn gen_restore(&mut self, expr: &Option<Expr>) {
        if let Some(line_expr) = expr {
            self.emit_comment(&format!("RESTORE {}", line_expr.show(false)));
            self.gen_expression(line_expr);
            self.gen_acc_val(line_expr);
        } else {
            self.emit_comment("RESTORE");
        }
        self.emit(StackInstruction::RestoreData);
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
        self.emit(StackInstruction::Random);
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
                    self.emit(StackInstruction::SystemOut);
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
        self.emit(StackInstruction::SystemOut);
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
            self.emit(StackInstruction::GPrint);
            
            match sep {
                PrintSeparator::Comma => self.emit(StackInstruction::PrintTab),
                PrintSeparator::Semicolon => {},
                PrintSeparator::None => {},
            }
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
        
        // Generar repeticiones
        self.gen_expression(repetitions_expr);
        self.gen_acc_val(repetitions_expr);
        
        // Generar parámetros opcionales si existen
        if let Some(_params) = optional_params {
            // Los parámetros adicionales del BEEP (frecuencia, duración)
            // se generan aquí si es necesario
            self.emit_comment("BEEP with parameters");
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
        // La diferencia entre Me0 y Me1 (POKE vs POKE#) es solo semántica,
        // la dirección sigue siendo absoluta
        
        if exprs.len() >= 2 {
            // Primera expresión: dirección absoluta
            self.gen_expression(&exprs[0]);
            self.gen_acc_val(&exprs[0]);
            
            // Segunda expresión: valor a escribir
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
        // Generar expresión del target (número de línea)
        self.gen_expression(target);
        self.gen_acc_val(target);
        self.emit(StackInstruction::OnErrorGoto("ERROR_HANDLER".to_string()));
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
        // Generar código para ambos operandos
        self.gen_expression(left);
        self.gen_acc_val(left);
        self.gen_expression(right);
        self.gen_acc_val(right);
        
        // Determinar el tipo resultante (simplificado)
        // En una implementación completa, consultaríamos la tabla de tipos
        let is_real = false; // Por ahora, asumimos todo entero
        
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
            
            // Operaciones de comparación
            BinaryOp::Eq => {
                if is_real {
                    self.emit(StackInstruction::IgualReal);
                } else {
                    self.emit(StackInstruction::IgualInt);
                }
            }
            BinaryOp::Neq => {
                if is_real {
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
                self.emit(StackInstruction::AndInt);
            }
            BinaryOp::Or => {
                self.emit(StackInstruction::OrInt);
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
                self.emit(StackInstruction::CallInt);
            }
            FunctionInner::Abs { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallAbs);
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
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallLen);
            }
            FunctionInner::Mid { string, start, length } => {
                self.gen_expression(string);
                self.gen_acc_val(string);
                self.gen_expression(start);
                self.gen_acc_val(start);
                self.gen_expression(length);
                self.gen_acc_val(length);
                self.emit(StackInstruction::CallMid);
            }
            FunctionInner::Left { string, length } => {
                self.gen_expression(string);
                self.gen_acc_val(string);
                self.gen_expression(length);
                self.gen_acc_val(length);
                self.emit(StackInstruction::CallLeft);
            }
            FunctionInner::Right { string, length } => {
                self.gen_expression(string);
                self.gen_acc_val(string);
                self.gen_expression(length);
                self.gen_acc_val(length);
                self.emit(StackInstruction::CallRight);
            }
            FunctionInner::Chr { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallChr);
            }
            FunctionInner::Asc { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallAsc);
            }
            FunctionInner::Str { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallStr);
            }
            FunctionInner::Val { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
                self.emit(StackInstruction::CallVal);
            }
            
            // Otras funciones
            FunctionInner::Rnd { range_end } => {
                self.gen_expression(range_end);
                self.gen_acc_val(range_end);
                self.emit(StackInstruction::CallRnd);
            }
            
            // Funciones matemáticas adicionales
            FunctionInner::Sgn { expr } => {
                self.gen_expression(expr);
                self.gen_acc_val(expr);
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
                // Dirección base del array
                let base_addr = self.get_or_create_variable_address(&identifier.to_string());
                self.emit(StackInstruction::ApilaInt(base_addr as i64));
                
                // Índice
                self.gen_expression(index);
                self.gen_acc_val(index);
                
                // Tamaño de cada elemento (simplificado: 5 bytes para números)
                self.emit(StackInstruction::ApilaInt(5));
                self.emit(StackInstruction::MulInt);
                
                // Dirección = base + índice * tamaño
                self.emit(StackInstruction::SumaInt);
            }
            
            // Array 2D
            LValueInner::Array2DAccess { identifier, row_index, col_index } => {
                // Dirección base
                let base_addr = self.get_or_create_variable_address(&identifier.to_string());
                self.emit(StackInstruction::ApilaInt(base_addr as i64));
                
                // Calcular offset: (i * num_cols + j) * tam_elemento
                self.gen_expression(row_index);
                self.gen_acc_val(row_index);
                
                // TODO: Necesitamos saber el número de columnas del array
                // Por ahora, asumimos 10
                self.emit(StackInstruction::ApilaInt(10));
                self.emit(StackInstruction::MulInt);
                
                self.gen_expression(col_index);
                self.gen_acc_val(col_index);
                self.emit(StackInstruction::SumaInt);
                
                // Multiplicar por tamaño del elemento
                self.emit(StackInstruction::ApilaInt(5));
                self.emit(StackInstruction::MulInt);
                
                // Sumar a la base
                self.emit(StackInstruction::SumaInt);
            }
            
            // Built-in identifiers like TIME, PI
            LValueInner::BuiltInIdentifier(keyword) => {
                self.emit_comment(&format!("Built-in identifier: {:?}", keyword));
                // TODO: Implementar soporte para identificadores built-in
                // Por ahora, asignamos una dirección arbitraria
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
        // necesitamos cargar su valor
        if self.es_designador(expr) {
            self.emit(StackInstruction::ApilaInd);
        }
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
            let addr = DATA_BASE + self.next_address;
            self.variable_addresses.insert(name.to_string(), addr);
            self.next_address += 10; // Espacio genérico por variable
            addr
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
    
    /// Convertir las instrucciones generadas a texto
    pub fn to_string(&self) -> String {
        self.instructions
            .iter()
            .map(|instr| instr.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
