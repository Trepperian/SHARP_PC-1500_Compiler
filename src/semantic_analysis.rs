use crate::error::{SemanticError, SemanticResult};
use crate::lex::{identifier::Identifier, keyword::Keyword};
use crate::parse::statement::assignment::Assignment;
use crate::parse::statement::beep_params::BeepParams;
use crate::parse::statement::dim_inner::DimInner;
use crate::parse::statement::statement_inner::StatementInner;
use crate::parse::{
    code_line::CodeLine,
    expression::{
        Expr,
        binary_op::BinaryOp,
        expr_inner::ExprInner,
        function::{Function, FunctionInner},
        lvalue::{LValue, LValueInner},
        unary_op::UnaryOp,
    },
    program::Program,
    statement::{Statement, lcursor_clause::LCursorClause, line_inner::LineInner},
};
use std::collections::HashMap;

/// Basic types in the BASIC language type system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicType {
    Numeric,
    String,
    NumericArray,
    StringArray,
    Numeric2DArray,
    String2DArray,
}

/// Symbol table for tracking variable and label declarations
#[derive(Debug, Clone)]
pub struct SymbolTable {
    variables: HashMap<Identifier, BasicType>,
    labels: HashMap<String, u16>, // label name -> line number
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            labels: HashMap::new(),
        }
    }

    pub fn insert_variable(&mut self, id: Identifier, type_: BasicType) {
        self.variables.insert(id, type_);
    }

    pub fn insert_label(&mut self, name: String, line_number: u16) {
        self.labels.insert(name, line_number);
    }
}

pub struct SemanticAnalyzer {
    symbol_table: SymbolTable,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            symbol_table: SymbolTable::new(),
        }
    }

    /// Analyze a complete program
    pub fn analyze_program(&mut self, program: &Program) -> SemanticResult<()> {
        // First pass: collect all labels
        for line in program.lines() {
            if let Some(label) = line.label() {
                self.symbol_table
                    .insert_label(label.to_owned(), line.number());
            }
        }

        // Second pass: analyze statements
        for line in program.lines() {
            self.analyze_line(line)?;
        }

        Ok(())
    }

    /// Analyze a single line
    fn analyze_line(&mut self, line: &CodeLine) -> SemanticResult<()> {
        // println!("Analyzing line {} with statements:", line.number);
        // for (i, statement) in line.statements.iter().enumerate() {
        //     println!("  Statement {}: {}", i + 1, statement);
        // }

        for statement in line.statements() {
            self.analyze_statement(statement)?;
        }
        Ok(())
    }

    /// Analyze a statement
    fn analyze_statement(&mut self, statement: &Statement) -> SemanticResult<()> {
        match &statement.inner {
            StatementInner::Let { inner, .. } => {
                for assignment in inner.assignments() {
                    self.analyze_assignment(assignment)?;
                }
            }
            StatementInner::If {
                condition,
                then_stmt,
                ..
            } => {
                // Surprisingly, the condition can be a string
                let _condition_type = self.analyze_expression(condition)?;
                self.analyze_statement(then_stmt)?;
            }
            StatementInner::For {
                assignment,
                to_expr,
                step_expr,
            } => {
                self.analyze_assignment(assignment)?;
                let to_type = self.analyze_expression(to_expr)?;
                if to_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: to_type,
                        span: to_expr.span(),
                    });
                }
                if let Some(step) = step_expr {
                    let step_type = self.analyze_expression(step)?;
                    if step_type != BasicType::Numeric {
                        return Err(SemanticError::TypeMismatch {
                            expected: BasicType::Numeric,
                            found: step_type,
                            span: step.span(),
                        });
                    }
                }
            }
            StatementInner::Next { lvalue } => {
                // Ensure the identifier is numeric (FOR loop variable)
                let ty = self.analyze_lvalue(lvalue)?;
                if ty != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: ty,
                        span: lvalue.span,
                    });
                }
            }
            StatementInner::Goto { target } => {
                self.analyze_expression(target)?;
            }
            StatementInner::Gosub { target } => {
                self.analyze_expression(target)?;
            }
            StatementInner::Dim { decls } => {
                for dim in decls {
                    self.analyze_dim_declaration(dim)?;
                }
            }
            StatementInner::Read { destinations } => {
                for lvalue in destinations {
                    self.analyze_lvalue(lvalue)?;
                }
            }
            StatementInner::Data(expressions) => {
                for expr in expressions {
                    self.analyze_expression(expr)?;
                }
            }
            StatementInner::Restore { expr } => {
                if let Some(expr) = expr {
                    self.analyze_expression(expr)?;
                }
            }
            StatementInner::OnGoto { expr, targets } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
                for target in targets {
                    self.analyze_expression(target)?;
                }
            }
            StatementInner::OnGosub { expr, targets } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
                for target in targets {
                    self.analyze_expression(target)?;
                }
            }
            StatementInner::Wait { expr } => {
                if let Some(expr) = expr {
                    let expr_type = self.analyze_expression(expr)?;
                    if expr_type != BasicType::Numeric {
                        return Err(SemanticError::TypeMismatch {
                            expected: BasicType::Numeric,
                            found: expr_type,
                            span: expr.span(),
                        });
                    }
                }
            }
            StatementInner::Beep {
                repetitions_expr,
                optional_params,
            } => {
                let rep_type = self.analyze_expression(repetitions_expr)?;
                if rep_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: rep_type,
                        span: repetitions_expr.span(),
                    });
                }
                if let Some(params) = optional_params {
                    self.analyze_beep_params(params)?;
                }
            }
            StatementInner::Cursor { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
            }
            StatementInner::GCursor { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
            }
            StatementInner::LCursor(clause) => {
                self.analyze_lcursor_clause(clause)?;
            }
            StatementInner::Color { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
            }
            StatementInner::CSize { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
            }
            StatementInner::Lf { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
            }
            // Statements that don't require type checking
            StatementInner::Print { .. } => {} // Print can handle any type
            StatementInner::LPrint { .. } => {} // LPrint can handle any type
            StatementInner::Gprint { .. } => {} // GPrint can handle any type
            StatementInner::Pause { .. } => {} // Pause can handle any type
            StatementInner::Input { input_exprs } => {
                for (prompt, lvalue) in input_exprs {
                    if let Some(prompt) = prompt {
                        let prompt_type = self.analyze_expression(prompt)?;
                        if prompt_type != BasicType::String {
                            return Err(SemanticError::TypeMismatch {
                                expected: BasicType::String,
                                found: prompt_type,
                                span: prompt.span(),
                            });
                        }
                    }
                    self.analyze_lvalue(lvalue)?;
                }
            }
            StatementInner::Using { using_clause } => {
                if let Some(format) = using_clause.format() {
                    self.analyze_expression(format)?;
                }
            }
            StatementInner::End => {}
            StatementInner::Clear => {}
            StatementInner::Cls => {}
            StatementInner::Random => {}
            StatementInner::Return => {}
            StatementInner::Remark { .. } => {}
            StatementInner::Poke { .. } => {} // Poke requires numeric but we'll handle separately
            StatementInner::Arun => {}
            StatementInner::Lock => {}
            StatementInner::Unlock => {}
            StatementInner::OnErrorGoto { .. } => {}
            StatementInner::Call { .. } => {}
            StatementInner::BeepOnOff { .. } => {}
            StatementInner::Text => {}
            StatementInner::Graph => {}
            StatementInner::Radian => {}
            StatementInner::GlCursor { x_expr, y_expr } => {
                let x_type = self.analyze_expression(x_expr)?;
                let y_type = self.analyze_expression(y_expr)?;
                if x_type != BasicType::Numeric || y_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: if x_type != BasicType::Numeric {
                            x_type
                        } else {
                            y_type
                        },
                        span: if x_type != BasicType::Numeric {
                            x_expr.span()
                        } else {
                            y_expr.span()
                        },
                    });
                }
            }
            StatementInner::Line { inner } => {
                self.analyze_line_inner(inner)?;
            }
            StatementInner::RLine { inner } => {
                self.analyze_line_inner(inner)?;
            }
            StatementInner::Sorgn => {}
            StatementInner::Rotate { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: expr_type,
                        span: expr.span(),
                    });
                }
            }
            StatementInner::Degree => {}
            StatementInner::Multi(statements) => {
                for statement in statements {
                    self.analyze_statement(statement)?;
                }
            }
        }
        Ok(())
    }

    /// Analyze an assignment statement
    fn analyze_assignment(&mut self, assignment: &Assignment) -> SemanticResult<()> {
        let lvalue_type = self.analyze_lvalue(assignment.lvalue())?;
        let expr_type = self.analyze_expression(assignment.expr())?;

        if lvalue_type != expr_type {
            return Err(SemanticError::TypeMismatch {
                expected: lvalue_type,
                found: expr_type,
                span: assignment.expr().span(),
            });
        }

        Ok(())
    }

    /// Analyze an lvalue and return its type, also inserting it into the symbol table if it's a new variable
    fn analyze_lvalue(&mut self, lvalue: &LValue) -> SemanticResult<BasicType> {
        match &lvalue.inner {
            LValueInner::Identifier(identifier) => {
                let var_type = if identifier.has_dollar() {
                    BasicType::String
                } else {
                    BasicType::Numeric
                };
                self.symbol_table.insert_variable(*identifier, var_type);
                Ok(var_type)
            }
            LValueInner::BuiltInIdentifier(keyword) => {
                let var_type = match keyword {
                    Keyword::Time => BasicType::Numeric,
                    Keyword::InkeyDollar => BasicType::String,
                    _ => {
                        return Err(SemanticError::InvalidExpression {
                            span: lvalue.span,
                            message: format!("Invalid built-in identifier: {keyword:?}"),
                        });
                    }
                };
                Ok(var_type)
            }
            LValueInner::Array1DAccess { identifier, index } => {
                let index_type = self.analyze_expression(index)?;
                if index_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidArrayIndex {
                        span: index.span(),
                        message: "Array index must be numeric".to_string(),
                    });
                }
                let array_type = if identifier.has_dollar() {
                    BasicType::StringArray
                } else {
                    BasicType::NumericArray
                };
                self.symbol_table.insert_variable(*identifier, array_type);

                let element_type = if identifier.has_dollar() {
                    BasicType::String
                } else {
                    BasicType::Numeric
                };
                Ok(element_type)
            }
            LValueInner::Array2DAccess {
                identifier,
                row_index,
                col_index,
            } => {
                let row_type = self.analyze_expression(row_index)?;
                let col_type = self.analyze_expression(col_index)?;
                if row_type != BasicType::Numeric || col_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidArrayIndex {
                        span: if row_type != BasicType::Numeric {
                            row_index.span()
                        } else {
                            col_index.span()
                        },
                        message: "2D array indices must be numeric".to_string(),
                    });
                }
                let array_type = if identifier.has_dollar() {
                    BasicType::String2DArray
                } else {
                    BasicType::Numeric2DArray
                };
                self.symbol_table.insert_variable(*identifier, array_type);

                let element_type = if identifier.has_dollar() {
                    BasicType::String
                } else {
                    BasicType::Numeric
                };
                Ok(element_type)
            }
            LValueInner::FixedMemoryAreaAccess { index, has_dollar } => {
                let index_type = self.analyze_expression(index)?;
                if index_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidArrayIndex {
                        span: index.span(),
                        message: "Memory area index must be numeric".to_string(),
                    });
                }
                let var_type = if *has_dollar {
                    BasicType::String
                } else {
                    BasicType::Numeric
                };
                Ok(var_type)
            }
        }
    }

    /// Analyze a dimension declaration
    fn analyze_dim_declaration(&mut self, dim: &DimInner) -> SemanticResult<()> {
        match dim {
            DimInner::DimInner1D {
                identifier,
                size,
                string_length,
                span: _,
            } => {
                let size_type = self.analyze_expression(size)?;
                if size_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: size_type,
                        span: size.span(),
                    });
                }

                if let Some(str_len) = string_length {
                    let str_len_type = self.analyze_expression(str_len)?;
                    if str_len_type != BasicType::Numeric {
                        return Err(SemanticError::TypeMismatch {
                            expected: BasicType::Numeric,
                            found: str_len_type,
                            span: str_len.span(),
                        });
                    }
                }

                let array_type = if identifier.has_dollar() {
                    BasicType::StringArray
                } else {
                    BasicType::NumericArray
                };
                self.symbol_table.insert_variable(*identifier, array_type);
            }
            DimInner::DimInner2D {
                identifier,
                rows,
                cols,
                string_length,
                span: _,
            } => {
                let rows_type = self.analyze_expression(rows)?;
                let cols_type = self.analyze_expression(cols)?;
                if rows_type != BasicType::Numeric || cols_type != BasicType::Numeric {
                    return Err(SemanticError::TypeMismatch {
                        expected: BasicType::Numeric,
                        found: if rows_type != BasicType::Numeric {
                            rows_type
                        } else {
                            cols_type
                        },
                        span: if rows_type != BasicType::Numeric {
                            rows.span()
                        } else {
                            cols.span()
                        },
                    });
                }

                if let Some(str_len) = string_length {
                    let str_len_type = self.analyze_expression(str_len)?;
                    if str_len_type != BasicType::Numeric {
                        return Err(SemanticError::TypeMismatch {
                            expected: BasicType::Numeric,
                            found: str_len_type,
                            span: str_len.span(),
                        });
                    }
                }

                let array_type = if identifier.has_dollar() {
                    BasicType::String2DArray
                } else {
                    BasicType::Numeric2DArray
                };
                self.symbol_table.insert_variable(*identifier, array_type);
            }
        }
        Ok(())
    }

    /// Analyze BEEP optional parameters
    fn analyze_beep_params(&mut self, params: &BeepParams) -> SemanticResult<()> {
        let freq_type = self.analyze_expression(&params.frequency)?;
        if freq_type != BasicType::Numeric {
            return Err(SemanticError::TypeMismatch {
                expected: BasicType::Numeric,
                found: freq_type,
                span: params.frequency.span(),
            });
        }

        if let Some(ref duration) = params.duration {
            let dur_type = self.analyze_expression(duration)?;
            if dur_type != BasicType::Numeric {
                return Err(SemanticError::TypeMismatch {
                    expected: BasicType::Numeric,
                    found: dur_type,
                    span: duration.span(),
                });
            }
        }

        Ok(())
    }

    /// Analyze LCURSOR clause
    fn analyze_lcursor_clause(&mut self, clause: &LCursorClause) -> SemanticResult<()> {
        let expr_type = self.analyze_expression(&clause.expr)?;
        if expr_type != BasicType::Numeric {
            return Err(SemanticError::TypeMismatch {
                expected: BasicType::Numeric,
                found: expr_type,
                span: clause.expr.span(),
            });
        }
        Ok(())
    }

    /// Analyze LINE/RLINE inner parameters
    fn analyze_line_inner(&mut self, inner: &LineInner) -> SemanticResult<()> {
        // Analyze start point if present
        if let Some((x_expr, y_expr)) = &inner.start_point {
            let x_type = self.analyze_expression(x_expr)?;
            let y_type = self.analyze_expression(y_expr)?;
            if x_type != BasicType::Numeric || y_type != BasicType::Numeric {
                return Err(SemanticError::TypeMismatch {
                    expected: BasicType::Numeric,
                    found: if x_type != BasicType::Numeric {
                        x_type
                    } else {
                        y_type
                    },
                    span: if x_type != BasicType::Numeric {
                        x_expr.span()
                    } else {
                        y_expr.span()
                    },
                });
            }
        }

        // Analyze all end points
        for (x_expr, y_expr) in &inner.end_points {
            let x_type = self.analyze_expression(x_expr)?;
            let y_type = self.analyze_expression(y_expr)?;
            if x_type != BasicType::Numeric || y_type != BasicType::Numeric {
                return Err(SemanticError::TypeMismatch {
                    expected: BasicType::Numeric,
                    found: if x_type != BasicType::Numeric {
                        x_type
                    } else {
                        y_type
                    },
                    span: if x_type != BasicType::Numeric {
                        x_expr.span()
                    } else {
                        y_expr.span()
                    },
                });
            }
        }

        // Analyze optional line type
        if let Some(line_type_expr) = &inner.line_type {
            let line_type_t = self.analyze_expression(line_type_expr)?;
            if line_type_t != BasicType::Numeric {
                return Err(SemanticError::TypeMismatch {
                    expected: BasicType::Numeric,
                    found: line_type_t,
                    span: line_type_expr.span(),
                });
            }
        }

        // Analyze optional color
        if let Some(color_expr) = &inner.color {
            let color_t = self.analyze_expression(color_expr)?;
            if color_t != BasicType::Numeric {
                return Err(SemanticError::TypeMismatch {
                    expected: BasicType::Numeric,
                    found: color_t,
                    span: color_expr.span(),
                });
            }
        }

        Ok(())
    }

    /// Analyze an expression and return its type
    fn analyze_expression(&mut self, expr: &Expr) -> SemanticResult<BasicType> {
        self.analyze_expr_inner(expr.inner())
    }

    /// Analyze expression inner and return its type
    fn analyze_expr_inner(&mut self, expr_inner: &ExprInner) -> SemanticResult<BasicType> {
        match expr_inner {
            ExprInner::Parentheses(expr) => self.analyze_expression(expr),
            ExprInner::DecimalNumber(_) => Ok(BasicType::Numeric),
            ExprInner::BinaryNumber(_) => Ok(BasicType::Numeric),
            ExprInner::StringLiteral { .. } => Ok(BasicType::String),
            ExprInner::LValue(lvalue) => {
                let lvalue_type = self.analyze_lvalue(lvalue)?;
                Ok(lvalue_type)
            }
            ExprInner::Unary(op, expr) => {
                let expr_type = self.analyze_expression(expr)?;
                match op {
                    UnaryOp::Plus | UnaryOp::Minus => {
                        if expr_type != BasicType::Numeric {
                            return Err(SemanticError::TypeMismatch {
                                expected: BasicType::Numeric,
                                found: expr_type,
                                span: expr.span(),
                            });
                        }
                        Ok(BasicType::Numeric)
                    }
                    UnaryOp::Not => {
                        if expr_type != BasicType::Numeric {
                            return Err(SemanticError::TypeMismatch {
                                expected: BasicType::Numeric,
                                found: expr_type,
                                span: expr.span(),
                            });
                        }
                        Ok(BasicType::Numeric)
                    }
                }
            }
            ExprInner::Binary(left, op, right) => {
                let left_type = self.analyze_expression(left)?;
                let right_type = self.analyze_expression(right)?;

                match op {
                    // Arithmetic operations require numeric operands and return numeric
                    BinaryOp::Add => {
                        if left_type != right_type {
                            return Err(SemanticError::TypeMismatch {
                                expected: left_type,
                                found: right_type,
                                span: right.span(),
                            });
                        }
                        Ok(left_type)
                    }
                    BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Exp => {
                        if left_type != BasicType::Numeric || right_type != BasicType::Numeric {
                            return Err(SemanticError::TypeMismatch {
                                expected: BasicType::Numeric,
                                found: if left_type != BasicType::Numeric {
                                    left_type
                                } else {
                                    right_type
                                },
                                span: if left_type != BasicType::Numeric {
                                    left.span()
                                } else {
                                    right.span()
                                },
                            });
                        }
                        Ok(BasicType::Numeric)
                    }
                    // Comparison operations can work on matching types and return numeric (boolean)
                    BinaryOp::Eq
                    | BinaryOp::Neq
                    | BinaryOp::Lt
                    | BinaryOp::Gt
                    | BinaryOp::Leq
                    | BinaryOp::Geq => {
                        if left_type != right_type {
                            return Err(SemanticError::TypeMismatch {
                                expected: left_type,
                                found: right_type,
                                span: right.span(),
                            });
                        }
                        Ok(BasicType::Numeric) // Comparisons return boolean (represented as numeric)
                    }
                    // Logical operations require numeric operands and return numeric
                    BinaryOp::And | BinaryOp::Or => {
                        if left_type != BasicType::Numeric || right_type != BasicType::Numeric {
                            return Err(SemanticError::TypeMismatch {
                                expected: BasicType::Numeric,
                                found: if left_type != BasicType::Numeric {
                                    left_type
                                } else {
                                    right_type
                                },
                                span: if left_type != BasicType::Numeric {
                                    left.span()
                                } else {
                                    right.span()
                                },
                            });
                        }
                        Ok(BasicType::Numeric)
                    }
                }
            }
            ExprInner::FunctionCall(function) => self.analyze_function_call(function),
        }
    }

    /// Analyze a function call and return its return type
    fn analyze_function_call(&mut self, function: &Function) -> SemanticResult<BasicType> {
        match &function.inner {
            FunctionInner::Mid {
                string,
                start,
                length,
            } => {
                let string_type = self.analyze_expression(string)?;
                let start_type = self.analyze_expression(start)?;
                let length_type = self.analyze_expression(length)?;

                if string_type != BasicType::String {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: string.span(),
                        function_name: "MID$".to_string(),
                        message: "first argument must be string".to_string(),
                    });
                }
                if start_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: start.span(),
                        function_name: "MID$".to_string(),
                        message: "start position must be numeric".to_string(),
                    });
                }
                if length_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: length.span(),
                        function_name: "MID$".to_string(),
                        message: "length must be numeric".to_string(),
                    });
                }
                Ok(BasicType::String)
            }
            FunctionInner::Left { string, length } => {
                let string_type = self.analyze_expression(string)?;
                let length_type = self.analyze_expression(length)?;

                if string_type != BasicType::String {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: string.span(),
                        function_name: "LEFT$".to_string(),
                        message: "first argument must be string".to_string(),
                    });
                }
                if length_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: length.span(),
                        function_name: "LEFT$".to_string(),
                        message: "length must be numeric".to_string(),
                    });
                }
                Ok(BasicType::String)
            }
            FunctionInner::Right { string, length } => {
                let string_type = self.analyze_expression(string)?;
                let length_type = self.analyze_expression(length)?;

                if string_type != BasicType::String {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: string.span(),
                        function_name: "RIGHT$".to_string(),
                        message: "first argument must be string".to_string(),
                    });
                }
                if length_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: length.span(),
                        function_name: "RIGHT$".to_string(),
                        message: "length must be numeric".to_string(),
                    });
                }
                Ok(BasicType::String)
            }
            FunctionInner::Asc { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::String {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: expr.span(),
                        function_name: "ASC".to_string(),
                        message: "argument must be string".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Point { position } => {
                let pos_type = self.analyze_expression(position)?;
                if pos_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: position.span(),
                        function_name: "POINT".to_string(),
                        message: "position must be numeric".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Rnd { range_end } => {
                let range_type = self.analyze_expression(range_end)?;
                if range_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: range_end.span(),
                        function_name: "RND".to_string(),
                        message: "range must be numeric".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Int { expr }
            | FunctionInner::Sgn { expr }
            | FunctionInner::Abs { expr }
            | FunctionInner::Ln { expr }
            | FunctionInner::Log { expr }
            | FunctionInner::Dms { expr }
            | FunctionInner::Deg { expr }
            | FunctionInner::Tan { expr }
            | FunctionInner::Cos { expr }
            | FunctionInner::Sin { expr }
            | FunctionInner::Sqr { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: expr.span(),
                        function_name: format!("{function:?}"),
                        message: "argument must be numeric".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Status { arg } => {
                let arg_type = self.analyze_expression(arg)?;
                if arg_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: arg.span(),
                        function_name: "STATUS".to_string(),
                        message: "argument must be numeric".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Val { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::String {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: expr.span(),
                        function_name: "VAL".to_string(),
                        message: "argument must be string".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Str { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: expr.span(),
                        function_name: "STR$".to_string(),
                        message: "argument must be numeric".to_string(),
                    });
                }
                Ok(BasicType::String)
            }
            FunctionInner::Chr { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: expr.span(),
                        function_name: "CHR$".to_string(),
                        message: "argument must be numeric".to_string(),
                    });
                }
                Ok(BasicType::String)
            }
            FunctionInner::Len { expr } => {
                let expr_type = self.analyze_expression(expr)?;
                if expr_type != BasicType::String {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: expr.span(),
                        function_name: "LEN".to_string(),
                        message: "argument must be string".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
            FunctionInner::Peek { address, .. } => {
                let addr_type = self.analyze_expression(address)?;
                if addr_type != BasicType::Numeric {
                    return Err(SemanticError::InvalidFunctionArgument {
                        span: address.span(),
                        function_name: "PEEK".to_string(),
                        message: "address must be numeric".to_string(),
                    });
                }
                Ok(BasicType::Numeric)
            }
        }
    }
}

/// Public function to analyze a program
pub fn analyze_program(program: &Program) -> SemanticResult<SymbolTable> {
    let mut analyzer = SemanticAnalyzer::new();
    analyzer.analyze_program(program)?;
    Ok(analyzer.symbol_table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::decimal_number::DecimalNumber;
    use crate::lex::identifier::Identifier;
    use crate::parse::expression::{Expr, expr_inner::ExprInner};

    #[cfg(test)]
    // Helper function to create a placeholder span for tests
    fn placeholder_span() -> crate::error::Span {
        crate::error::Span::new(0, 1)
    }

    #[test]
    fn test_numeric_assignment() {
        let mut analyzer = SemanticAnalyzer::new();

        // Create identifier "X" (numeric)
        let identifier = Identifier::new('X', None, false).unwrap();

        let assignment = Assignment::new(
            LValue::new(LValueInner::Identifier(identifier), placeholder_span()),
            Box::new(Expr::new(
                ExprInner::DecimalNumber(DecimalNumber::new(42.0)),
                placeholder_span(),
            )),
            placeholder_span(),
        );

        let result = analyzer.analyze_assignment(&assignment);
        assert!(result.is_ok());
    }

    #[test]
    fn test_type_mismatch() {
        let mut analyzer = SemanticAnalyzer::new();

        // Create identifier "X$" (string)
        let identifier = Identifier::new('X', None, true).unwrap();

        let assignment = Assignment::new(
            LValue::new(LValueInner::Identifier(identifier), placeholder_span()),
            Box::new(Expr::new(
                ExprInner::DecimalNumber(DecimalNumber::new(42.0)),
                placeholder_span(),
            )),
            placeholder_span(),
        );

        let result = analyzer.analyze_assignment(&assignment);
        assert!(result.is_err());
        if let Err(SemanticError::TypeMismatch {
            expected, found, ..
        }) = result
        {
            assert_eq!(expected, BasicType::String);
            assert_eq!(found, BasicType::Numeric);
        }
    }
}
