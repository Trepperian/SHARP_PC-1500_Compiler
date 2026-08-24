use crate::parse::{
    expression::{Expr, lvalue::LValue, memory_area::MemoryArea},
    statement::{
        Statement, assignment::Assignment, beep_params::BeepParams, dim_inner::DimInner,
        lcursor_clause::LCursorClause, let_inner::LetInner, line_inner::LineInner,
        lprint_inner::LPrintInner, print_inner::PrintInner, print_separator::PrintSeparator,
        using_clause::UsingClause,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub enum StatementInner {
    Let {
        inner: LetInner,
        is_let_kw_present_in_source: bool,
    },
    If {
        condition: Expr,
        then_stmt: Box<Statement>,
        is_then_kw_present_in_source: bool,
        is_goto_kw_present_in_source: bool,
    },
    Print {
        inner: PrintInner,
    },
    Pause {
        inner: PrintInner,
    },
    LPrint {
        inner: LPrintInner,
    },
    Using {
        using_clause: UsingClause,
    },
    Input {
        input_exprs: Vec<(Option<Expr>, LValue)>,
    },
    End,
    Remark {
        text: String,
    },
    For {
        assignment: Assignment,
        to_expr: Expr,
        step_expr: Option<Expr>,
    },
    Next {
        lvalue: LValue,
    },
    Clear,
    Goto {
        target: Expr,
    },
    Gosub {
        target: Expr,
    },
    OnGoto {
        expr: Expr,
        targets: Vec<Expr>,
    },
    OnGosub {
        expr: Expr,
        targets: Vec<Expr>,
    },
    OnErrorGoto {
        target: Expr,
    },
    Wait {
        expr: Option<Expr>,
    },
    Cls,
    Random,
    Gprint {
        exprs: Vec<(Expr, PrintSeparator)>,
    },
    GCursor {
        expr: Expr,
    },
    Cursor {
        expr: Expr,
    },
    Beep {
        repetitions_expr: Expr,
        optional_params: Option<BeepParams>,
    },
    BeepOnOff {
        switch_beep_on: bool, // true for BEEP ON, false for BEEP OFF
    },
    Return,
    Poke {
        memory_area: MemoryArea,
        exprs: Vec<Expr>,
    },
    Dim {
        decls: Vec<DimInner>,
    },
    Read {
        destinations: Vec<LValue>,
    },
    Data(Vec<Expr>),
    Restore {
        expr: Option<Expr>,
    },
    Arun,
    Lock,
    Unlock,
    Call {
        expr: Expr,
        variable: Option<LValue>,
    },
    Radian,
    Text,
    Graph,
    Color {
        expr: Expr,
    },
    CSize {
        expr: Expr,
    },
    Lf {
        expr: Expr,
    },
    LCursor(LCursorClause),
    GlCursor {
        x_expr: Expr,
        y_expr: Expr,
    },
    Line {
        inner: LineInner,
    },
    RLine {
        inner: LineInner,
    },
    Sorgn,
    Rotate {
        expr: Expr,
    },
    Degree,
    /// Varias sentencias separadas por `:` que forman, en conjunto, el
    /// consecuente de un `IF` sin bloque explícito (p.ej. `IF cond
    /// stmt1:stmt2:stmt3`) — en BASIC clásico (incluido el dialecto
    /// Sharp PC-1500 de todo este corpus), TODAS las sentencias hasta el
    /// final de la línea son condicionales, no solo la primera. Solo se
    /// construye desde `Parser::parse_if_stmt`; en cualquier otro
    /// contexto una lista de sentencias de una línea sigue viviendo como
    /// `Vec<Statement>` plano en `CodeLine`.
    Multi(Vec<Statement>),
}

impl StatementInner {
    pub fn write_bytes(&self, bytes: &mut Vec<u8>, preserve_source_wording: bool) {
        use crate::lex::keyword::Keyword;

        match self {
            StatementInner::Let {
                inner,
                is_let_kw_present_in_source,
            } => {
                if preserve_source_wording && *is_let_kw_present_in_source {
                    bytes.extend_from_slice(Keyword::Let.internal_code().to_be_bytes().as_slice());
                }

                inner.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::If {
                condition,
                then_stmt,
                is_then_kw_present_in_source,
                is_goto_kw_present_in_source,
            } => {
                bytes.extend_from_slice(Keyword::If.internal_code().to_be_bytes().as_slice());
                condition.write_bytes(bytes, preserve_source_wording);
                if preserve_source_wording && *is_then_kw_present_in_source {
                    bytes.extend_from_slice(Keyword::Then.internal_code().to_be_bytes().as_slice());
                }
                match &then_stmt.inner {
                    StatementInner::Let { inner, .. } => {
                        // LET keyword is mandatory here
                        bytes.extend_from_slice(
                            Keyword::Let.internal_code().to_be_bytes().as_slice(),
                        );
                        inner.write_bytes(bytes, preserve_source_wording);
                    }
                    StatementInner::Goto { target } => {
                        if preserve_source_wording {
                            if *is_goto_kw_present_in_source {
                                bytes.extend_from_slice(
                                    Keyword::Goto.internal_code().to_be_bytes().as_slice(),
                                );
                            }
                        } else {
                            bytes.extend_from_slice(
                                Keyword::Then.internal_code().to_be_bytes().as_slice(),
                            );
                        }
                        target.write_bytes(bytes, preserve_source_wording);
                    }
                    _ => {
                        then_stmt.write_bytes(bytes, preserve_source_wording);
                    }
                }
            }
            StatementInner::Print { inner } => {
                bytes.extend_from_slice(Keyword::Print.internal_code().to_be_bytes().as_slice());
                for (printable, sep) in &inner.exprs {
                    printable.write_bytes(bytes, preserve_source_wording);
                    sep.write_bytes(bytes);
                }
            }
            StatementInner::Pause { inner } => {
                bytes.extend_from_slice(Keyword::Pause.internal_code().to_be_bytes().as_slice());
                for (printable, sep) in &inner.exprs {
                    printable.write_bytes(bytes, preserve_source_wording);
                    sep.write_bytes(bytes);
                }
            }
            StatementInner::LPrint { inner } => {
                bytes.extend_from_slice(Keyword::Lprint.internal_code().to_be_bytes().as_slice());
                inner.write_bytes_with_context(true, bytes, preserve_source_wording);
            }
            StatementInner::Using { using_clause } => {
                using_clause.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Input { input_exprs } => {
                bytes.extend_from_slice(Keyword::Input.internal_code().to_be_bytes().as_slice());
                for (i, (prompt, lvalue)) in input_exprs.iter().enumerate() {
                    if let Some(prompt) = prompt {
                        prompt.write_bytes(bytes, preserve_source_wording);
                        bytes.push(b';');
                    }
                    lvalue.write_bytes(bytes, preserve_source_wording);
                    if i < input_exprs.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::End => {
                bytes.extend_from_slice(Keyword::End.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Remark { text } => {
                bytes.extend_from_slice(Keyword::Rem.internal_code().to_be_bytes().as_slice());
                bytes.extend_from_slice(text.as_bytes());
            }
            StatementInner::For {
                assignment,
                to_expr,
                step_expr,
            } => {
                bytes.extend_from_slice(Keyword::For.internal_code().to_be_bytes().as_slice());
                assignment.write_bytes(bytes, preserve_source_wording);
                bytes.extend_from_slice(Keyword::To.internal_code().to_be_bytes().as_slice());
                to_expr.write_bytes(bytes, preserve_source_wording);
                if let Some(step) = step_expr {
                    bytes.extend_from_slice(Keyword::Step.internal_code().to_be_bytes().as_slice());
                    step.write_bytes(bytes, preserve_source_wording);
                }
            }
            StatementInner::Next { lvalue } => {
                bytes.extend_from_slice(Keyword::Next.internal_code().to_be_bytes().as_slice());
                lvalue.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Clear => {
                bytes.extend_from_slice(Keyword::Clear.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Goto { target } => {
                bytes.extend_from_slice(Keyword::Goto.internal_code().to_be_bytes().as_slice());
                target.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Gosub { target } => {
                bytes.extend_from_slice(Keyword::Gosub.internal_code().to_be_bytes().as_slice());
                target.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::OnGoto { expr, targets } => {
                bytes.extend_from_slice(Keyword::On.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
                bytes.extend_from_slice(Keyword::Goto.internal_code().to_be_bytes().as_slice());
                for (i, target) in targets.iter().enumerate() {
                    target.write_bytes(bytes, preserve_source_wording);
                    if i < targets.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::OnGosub { expr, targets } => {
                bytes.extend_from_slice(Keyword::On.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
                bytes.extend_from_slice(Keyword::Gosub.internal_code().to_be_bytes().as_slice());
                for (i, target) in targets.iter().enumerate() {
                    target.write_bytes(bytes, preserve_source_wording);
                    if i < targets.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::OnErrorGoto { target } => {
                bytes.extend_from_slice(Keyword::On.internal_code().to_be_bytes().as_slice());
                bytes.extend_from_slice(Keyword::Error.internal_code().to_be_bytes().as_slice());
                bytes.extend_from_slice(Keyword::Goto.internal_code().to_be_bytes().as_slice());
                target.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Wait { expr } => {
                bytes.extend_from_slice(Keyword::Wait.internal_code().to_be_bytes().as_slice());
                if let Some(expr) = expr {
                    expr.write_bytes(bytes, preserve_source_wording);
                }
            }
            StatementInner::Cls => {
                bytes.extend_from_slice(Keyword::Cls.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Random => {
                bytes.extend_from_slice(Keyword::Random.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Gprint { exprs } => {
                bytes.extend_from_slice(Keyword::Gprint.internal_code().to_be_bytes().as_slice());
                for (expr, sep) in exprs {
                    expr.write_bytes(bytes, preserve_source_wording);
                    sep.write_bytes(bytes);
                }
            }
            StatementInner::GCursor { expr } => {
                bytes.extend_from_slice(Keyword::Gcursor.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Cursor { expr } => {
                bytes.extend_from_slice(Keyword::Cursor.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Beep {
                repetitions_expr,
                optional_params,
            } => {
                bytes.extend_from_slice(Keyword::Beep.internal_code().to_be_bytes().as_slice());
                repetitions_expr.write_bytes(bytes, preserve_source_wording);
                if let Some(params) = optional_params {
                    bytes.push(b',');
                    params.frequency.write_bytes(bytes, preserve_source_wording);
                    if let Some(duration) = &params.duration {
                        bytes.push(b',');
                        duration.write_bytes(bytes, preserve_source_wording);
                    }
                }
            }
            StatementInner::BeepOnOff { switch_beep_on } => {
                bytes.extend_from_slice(Keyword::Beep.internal_code().to_be_bytes().as_slice());
                if *switch_beep_on {
                    bytes.extend_from_slice(Keyword::On.internal_code().to_be_bytes().as_slice());
                } else {
                    bytes.extend_from_slice(Keyword::Off.internal_code().to_be_bytes().as_slice());
                }
            }
            StatementInner::Return => {
                bytes.extend_from_slice(Keyword::Return.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Poke { memory_area, exprs } => {
                match memory_area {
                    MemoryArea::Me0 => bytes
                        .extend_from_slice(Keyword::Poke.internal_code().to_be_bytes().as_slice()),
                    MemoryArea::Me1 => bytes.extend_from_slice(
                        Keyword::PokeHashTag
                            .internal_code()
                            .to_be_bytes()
                            .as_slice(),
                    ),
                }
                for (i, expr) in exprs.iter().enumerate() {
                    expr.write_bytes(bytes, preserve_source_wording);
                    if i < exprs.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::Dim { decls } => {
                bytes.extend_from_slice(Keyword::Dim.internal_code().to_be_bytes().as_slice());
                for (i, decl) in decls.iter().enumerate() {
                    match decl {
                        DimInner::DimInner1D {
                            identifier,
                            size,
                            string_length,
                            span: _,
                        } => {
                            identifier.write_bytes(bytes);
                            bytes.push(b'(');
                            size.write_bytes(bytes, preserve_source_wording);
                            bytes.push(b')');
                            if let Some(len) = string_length {
                                bytes.push(b'*');
                                len.write_bytes(bytes, preserve_source_wording);
                            }
                        }
                        DimInner::DimInner2D {
                            identifier,
                            rows,
                            cols,
                            string_length,
                            span: _,
                        } => {
                            identifier.write_bytes(bytes);
                            bytes.push(b'(');
                            rows.write_bytes(bytes, preserve_source_wording);
                            bytes.push(b',');
                            cols.write_bytes(bytes, preserve_source_wording);
                            bytes.push(b')');
                            if let Some(len) = string_length {
                                bytes.push(b'*');
                                len.write_bytes(bytes, preserve_source_wording);
                            }
                        }
                    }

                    if i < decls.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::Read { destinations } => {
                bytes.extend_from_slice(Keyword::Read.internal_code().to_be_bytes().as_slice());
                for (i, dest) in destinations.iter().enumerate() {
                    dest.write_bytes(bytes, preserve_source_wording);
                    if i < destinations.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::Data(exprs) => {
                bytes.extend_from_slice(Keyword::Data.internal_code().to_be_bytes().as_slice());
                for (i, expr) in exprs.iter().enumerate() {
                    expr.write_bytes(bytes, preserve_source_wording);
                    if i < exprs.len() - 1 {
                        bytes.push(b',');
                    }
                }
            }
            StatementInner::Restore { expr } => {
                bytes.extend_from_slice(Keyword::Restore.internal_code().to_be_bytes().as_slice());
                if let Some(expr) = expr {
                    expr.write_bytes(bytes, preserve_source_wording);
                }
            }
            StatementInner::Arun => {
                bytes.extend_from_slice(Keyword::Arun.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Lock => {
                bytes.extend_from_slice(Keyword::Lock.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Unlock => {
                bytes.extend_from_slice(Keyword::Unlock.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Call { expr, variable } => {
                bytes.extend_from_slice(Keyword::Call.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
                if let Some(var) = variable {
                    bytes.push(b',');
                    var.write_bytes(bytes, preserve_source_wording);
                }
            }
            StatementInner::Radian => {
                bytes.extend_from_slice(Keyword::Radian.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Text => {
                bytes.extend_from_slice(Keyword::Text.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Graph => {
                bytes.extend_from_slice(Keyword::Graph.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Color { expr } => {
                bytes.extend_from_slice(Keyword::Color.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::CSize { expr } => {
                bytes.extend_from_slice(Keyword::Csize.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Lf { expr } => {
                bytes.extend_from_slice(Keyword::Lf.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::LCursor(l_cursor_clause) => {
                l_cursor_clause.write_bytes_with_context(false, bytes, preserve_source_wording);
            }
            StatementInner::GlCursor { x_expr, y_expr } => {
                bytes.extend_from_slice(Keyword::Glcursor.internal_code().to_be_bytes().as_slice());
                bytes.push(b'(');
                x_expr.write_bytes(bytes, preserve_source_wording);
                bytes.push(b',');
                y_expr.write_bytes(bytes, preserve_source_wording);
                bytes.push(b')');
            }
            StatementInner::Line { inner } => {
                bytes.extend_from_slice(Keyword::Line.internal_code().to_be_bytes().as_slice());
                inner.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::RLine { inner } => {
                bytes.extend_from_slice(Keyword::Rline.internal_code().to_be_bytes().as_slice());
                inner.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Sorgn => {
                bytes.extend_from_slice(Keyword::Sorgn.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Rotate { expr } => {
                bytes.extend_from_slice(Keyword::Rotate.internal_code().to_be_bytes().as_slice());
                expr.write_bytes(bytes, preserve_source_wording);
            }
            StatementInner::Degree => {
                bytes.extend_from_slice(Keyword::Degree.internal_code().to_be_bytes().as_slice());
            }
            StatementInner::Multi(statements) => {
                for (i, statement) in statements.iter().enumerate() {
                    statement.write_bytes(bytes, preserve_source_wording);
                    if i < statements.len() - 1 {
                        bytes.push(b':');
                    }
                }
            }
        }
    }

    pub fn show(&self, preserve_source_wording: bool) -> String {
        match self {
            StatementInner::Let { inner, .. } => {
                inner.show_with_context(false, preserve_source_wording)
            }
            StatementInner::If {
                condition,
                then_stmt,
                is_then_kw_present_in_source,
                is_goto_kw_present_in_source,
            } => {
                let mut result = "IF ".to_owned();

                result.push_str(&condition.show(preserve_source_wording));
                result.push(' ');

                if preserve_source_wording && *is_then_kw_present_in_source {
                    result.push_str("THEN ");
                }

                match &then_stmt.as_ref().inner {
                    StatementInner::Let { inner, .. } => {
                        result.push_str(&inner.show_with_context(true, preserve_source_wording))
                    }
                    StatementInner::Goto { target } => {
                        if preserve_source_wording {
                            if *is_goto_kw_present_in_source {
                                result.push_str("GOTO ");
                            }
                        } else {
                            result.push_str("THEN ");
                        }
                        result.push_str(&target.show(preserve_source_wording));
                    }
                    _ => result.push_str(&then_stmt.show(preserve_source_wording)),
                }

                result
            }
            StatementInner::Print { inner } => {
                format!("PRINT {}", inner.show(preserve_source_wording))
            }
            StatementInner::Pause { inner } => {
                format!("PAUSE {}", inner.show(preserve_source_wording))
            }
            StatementInner::LPrint { inner } => {
                format!(
                    "LPRINT {}",
                    inner.to_string_with_context(true, preserve_source_wording)
                )
            }
            StatementInner::Using { using_clause } => {
                using_clause.show(preserve_source_wording).to_string()
            }
            StatementInner::Input { input_exprs } => {
                let inputs = input_exprs
                    .iter()
                    .map(|(prompt, lval)| {
                        if let Some(prompt) = prompt {
                            format!(
                                "{};{}",
                                prompt.show(preserve_source_wording),
                                lval.show(preserve_source_wording)
                            )
                        } else {
                            lval.show(preserve_source_wording)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("INPUT {inputs}")
            }
            StatementInner::End => "END".to_string(),
            StatementInner::Remark { text } => format!("REM {text}"),
            StatementInner::For {
                assignment,
                to_expr,
                step_expr,
            } => {
                let step_str = if let Some(step) = step_expr {
                    format!(" STEP {}", step.show(preserve_source_wording))
                } else {
                    String::new()
                };
                format!(
                    "FOR {} TO {}{}",
                    assignment.show(preserve_source_wording),
                    to_expr.show(preserve_source_wording),
                    step_str
                )
            }
            StatementInner::Next { lvalue: ident } => {
                format!("NEXT {}", ident.show(preserve_source_wording))
            }
            StatementInner::Clear => "CLEAR".to_string(),
            StatementInner::Goto { target } => {
                format!("GOTO {}", target.show(preserve_source_wording))
            }
            StatementInner::Gosub { target } => {
                format!("GOSUB {}", target.show(preserve_source_wording))
            }
            StatementInner::OnGoto { expr, targets } => {
                format!(
                    "ON {} GOTO {}",
                    expr.show(preserve_source_wording),
                    targets
                        .iter()
                        .map(|target| target.show(preserve_source_wording))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            StatementInner::OnGosub { expr, targets } => {
                format!(
                    "ON {} GOSUB {}",
                    expr.show(preserve_source_wording),
                    targets
                        .iter()
                        .map(|target| target.show(preserve_source_wording))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            StatementInner::OnErrorGoto { target } => {
                format!("ON ERROR GOTO {}", target.show(preserve_source_wording))
            }
            StatementInner::Wait { expr } => {
                if let Some(expr) = expr {
                    format!("WAIT {}", expr.show(preserve_source_wording))
                } else {
                    "WAIT".to_string()
                }
            }
            StatementInner::Cls => "CLS".to_string(),
            StatementInner::Random => "RANDOM".to_string(),
            StatementInner::Gprint { exprs } => {
                let exprs_str = exprs
                    .iter()
                    .map(|(expr, sep)| format!("{}{sep}", expr.show(preserve_source_wording)))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("GPRINT {exprs_str}")
            }
            StatementInner::GCursor { expr } => {
                format!("GCURSOR {}", expr.show(preserve_source_wording))
            }
            StatementInner::Cursor { expr } => {
                format!("CURSOR {}", expr.show(preserve_source_wording))
            }
            StatementInner::Beep {
                repetitions_expr,
                optional_params,
            } => {
                let params_str = optional_params
                    .as_ref()
                    .map_or(String::new(), |params| params.show(preserve_source_wording));
                format!(
                    "BEEP {}{}",
                    repetitions_expr.show(preserve_source_wording),
                    params_str
                )
            }
            StatementInner::BeepOnOff { switch_beep_on } => {
                format!("BEEP {}", if *switch_beep_on { "ON" } else { "OFF" })
            }
            StatementInner::Return => "RETURN".to_string(),
            StatementInner::Poke { memory_area, exprs } => {
                let exprs_str = exprs
                    .iter()
                    .map(|expr| expr.show(preserve_source_wording))
                    .collect::<Vec<_>>()
                    .join(",");
                match memory_area {
                    MemoryArea::Me0 => format!("POKE {exprs_str}"),
                    MemoryArea::Me1 => format!("POKE# {exprs_str}"),
                }
            }
            StatementInner::Dim { decls } => {
                let decls_str = decls
                    .iter()
                    .map(|decl| decl.show(preserve_source_wording))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("DIM {decls_str}")
            }
            StatementInner::Read { destinations } => {
                let destinations_str = destinations
                    .iter()
                    .map(|dest| dest.show(preserve_source_wording))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("READ {destinations_str}")
            }
            StatementInner::Data(exprs) => {
                let exprs_str = exprs
                    .iter()
                    .map(|expr| expr.show(preserve_source_wording))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("DATA {exprs_str}")
            }
            StatementInner::Restore { expr } => {
                if let Some(expr) = expr {
                    format!("RESTORE {}", expr.show(preserve_source_wording))
                } else {
                    "RESTORE".to_string()
                }
            }
            StatementInner::Arun => "ARUN".to_string(),
            StatementInner::Lock => "LOCK".to_string(),
            StatementInner::Unlock => "UNLOCK".to_string(),
            StatementInner::Call { expr, variable } => {
                let var_str = variable.as_ref().map_or(String::new(), |v| {
                    format!(", {}", v.show(preserve_source_wording))
                });
                format!("CALL {}{var_str}", expr.show(preserve_source_wording))
            }
            StatementInner::Radian => "RADIAN".to_string(),
            StatementInner::Text => "TEXT".to_string(),
            StatementInner::Graph => "GRAPH".to_string(),
            StatementInner::Color { expr } => {
                format!("COLOR {}", expr.show(preserve_source_wording))
            }
            StatementInner::CSize { expr } => {
                format!("CSIZE {}", expr.show(preserve_source_wording))
            }
            StatementInner::Lf { expr } => format!("LF {}", expr.show(preserve_source_wording)),
            StatementInner::LCursor(l_cursor_clause) => {
                format!(
                    "LCURSOR {}",
                    l_cursor_clause.to_string_with_context(false, preserve_source_wording)
                )
            }
            StatementInner::GlCursor { x_expr, y_expr } => {
                format!(
                    "GLCURSOR ({}, {})",
                    x_expr.show(preserve_source_wording),
                    y_expr.show(preserve_source_wording)
                )
            }
            StatementInner::Line { inner } => {
                inner.to_string_with_prefix("LINE", preserve_source_wording)
            }
            StatementInner::RLine { inner } => {
                inner.to_string_with_prefix("RLINE", preserve_source_wording)
            }
            StatementInner::Sorgn => "SORGN".to_string(),
            StatementInner::Rotate { expr } => {
                format!("ROTATE {}", expr.show(preserve_source_wording))
            }
            StatementInner::Degree => {
                format!("DEGREE")
            }
            StatementInner::Multi(statements) => statements
                .iter()
                .map(|statement| statement.show(preserve_source_wording))
                .collect::<Vec<_>>()
                .join(":"),
        }
    }
}
