use std::{iter::Peekable, mem, vec};

use crate::{
    error::{ParseError, ParseResult, Span},
    lex::{
        SpannedToken, Token, binary_number::BinaryNumber, decimal_number::DecimalNumber,
        keyword::Keyword, symbol::Symbol,
    },
    parse::{
        code_line::{CodeLine, CodeLineLabel},
        expression::{
            Expr,
            binary_op::BinaryOp,
            expr_inner::ExprInner,
            function::{Function, FunctionInner},
            lvalue::{LValue, LValueInner},
            memory_area::MemoryArea,
            unary_op::UnaryOp,
        },
        program::Program,
        statement::{
            Statement, assignment::Assignment, beep_params::BeepParams, dim_inner::DimInner,
            lcursor_clause::LCursorClause, let_inner::LetInner, line_inner::LineInner,
            lprint_inner::LPrintInner, lprint_printable::LPrintPrintable, print_inner::PrintInner,
            print_separator::PrintSeparator, printable::Printable, statement_inner::StatementInner,
            using_clause::UsingClause,
        },
    },
};

pub mod code_line;
pub mod expression;
pub mod program;
pub mod statement;

pub struct Parser<I: Clone>
where
    I: Iterator<Item = SpannedToken>,
{
    tokens: Peekable<I>,
}

impl<I: Clone> Parser<I>
where
    I: Iterator<Item = SpannedToken>,
{
    pub fn new(tokens: I) -> Self {
        Self {
            tokens: tokens.peekable(),
        }
    }

    /// Helper method to peek at the token without the span
    fn peek_token(&mut self) -> &Token {
        self.tokens
            .peek()
            .map(|spanned| spanned.token())
            .unwrap_or(&Token::Symbol(Symbol::Eof))
    }

    fn peek_mut_token(&mut self) -> Option<&mut Token> {
        self.tokens.peek_mut().map(|spanned| spanned.token_mut())
    }

    /// Helper method to get the next token with span
    fn next_spanned(&mut self) -> Option<SpannedToken> {
        self.tokens.next()
    }

    /// Helper method to check if next token matches and consume it if so
    fn next_if_token_eq(&mut self, expected: &Token) -> Option<SpannedToken> {
        if self.peek_token() == expected {
            self.next_spanned()
        } else {
            None
        }
    }

    /// Helper method to conditionally consume token based on predicate  
    fn next_if_token(&mut self, predicate: impl Fn(&Token) -> bool) -> Option<SpannedToken> {
        if predicate(self.peek_token()) {
            self.next_spanned()
        } else {
            None
        }
    }

    /// Helper method to get the current span for error reporting
    fn current_span(&mut self) -> Span {
        self.tokens
            .peek()
            .map(|token| *token.span())
            .unwrap_or_else(|| unreachable!())
    }

    /// Helper method to expect a specific token and return an error if not found
    fn expect_token(&mut self, expected: &Token, context: &str) -> ParseResult<SpannedToken> {
        match self.next_spanned() {
            Some(token) if token.token() == expected => Ok(token),
            Some(token) => Err(ParseError::UnexpectedToken {
                expected: format!("{expected}"),
                found: format!("{}", token.token()),
                span: *token.span(),
            }),
            None => Err(ParseError::UnexpectedEndOfInput {
                expected: format!("{expected} ({context})"),
                span: self.current_span(),
            }),
        }
    }

    fn expect_expression(&mut self) -> ParseResult<Expr> {
        match self.parse_expression()? {
            Some(expr) => Ok(expr),
            None => Err(ParseError::ExpectedExpression {
                span: self.current_span(),
            }),
        }
    }

    fn expect_expression_factor(&mut self) -> ParseResult<Expr> {
        match self.parse_expression_factor()? {
            Some(expr) => Ok(expr),
            None => Err(ParseError::ExpectedExpression {
                span: self.current_span(),
            }),
        }
    }

    fn parse_exponent_expr(&mut self) -> ParseResult<Option<Expr>> {
        if let Some(mut left) = self.parse_expression_factor()? {
            while self.peek_token() == &Token::Symbol(Symbol::Exp) {
                self.next_spanned();
                let right = self.expect_expression_factor()?;
                let span = left.span().merge(right.span());
                left = Expr::new(
                    ExprInner::Binary(Box::new(left), BinaryOp::Exp, Box::new(right)),
                    span,
                );
            }

            Ok(Some(left))
        } else {
            Ok(None)
        }
    }

    fn parse_unary_op_expr(&mut self) -> ParseResult<Option<Expr>> {
        let maybe_op = self.next_if_token(|token| {
            matches!(
                token,
                Token::Symbol(Symbol::Add) | Token::Symbol(Symbol::Sub)
            )
        });
        match maybe_op.as_ref().map(|s| s.token()) {
            Some(Token::Symbol(Symbol::Add)) => {
                let op_span = *maybe_op.as_ref().unwrap().span();
                let right = self.expect_unary_op_expr()?;
                let span = op_span.merge(right.span());
                Ok(Some(Expr::new(
                    ExprInner::Unary(UnaryOp::Plus, Box::new(right)),
                    span,
                )))
            }
            Some(Token::Symbol(Symbol::Sub)) => {
                let op_span = *maybe_op.as_ref().unwrap().span();
                let right = self.expect_unary_op_expr()?;
                let span = op_span.merge(right.span());
                Ok(Some(Expr::new(
                    ExprInner::Unary(UnaryOp::Minus, Box::new(right)),
                    span,
                )))
            }
            _ => Ok(self.parse_exponent_expr()?),
        }
    }

    fn expect_unary_op_expr(&mut self) -> ParseResult<Expr> {
        match self.parse_unary_op_expr()? {
            Some(expr) => Ok(expr),
            None => Err(ParseError::ExpectedExpression {
                span: self.current_span(),
            }),
        }
    }

    fn parse_mul_div_expr(&mut self) -> ParseResult<Option<Expr>> {
        if let Some(mut left) = self.parse_unary_op_expr()? {
            while let Some(op) = self.next_if_token(|token| {
                matches!(
                    token,
                    Token::Symbol(Symbol::Mul) | Token::Symbol(Symbol::Div)
                )
            }) {
                let right = self.expect_unary_op_expr()?;
                let binary_op = match op.token() {
                    Token::Symbol(Symbol::Mul) => BinaryOp::Mul,
                    Token::Symbol(Symbol::Div) => BinaryOp::Div,
                    _ => unreachable!(),
                };
                let span = left.span().merge(right.span());
                left = Expr::new(
                    ExprInner::Binary(Box::new(left), binary_op, Box::new(right)),
                    span,
                );
            }

            Ok(Some(left))
        } else {
            Ok(None)
        }
    }

    fn expect_mul_div_expr(&mut self) -> ParseResult<Expr> {
        match self.parse_mul_div_expr()? {
            Some(expr) => Ok(expr),
            None => Err(ParseError::ExpectedExpression {
                span: self.current_span(),
            }),
        }
    }

    fn parse_add_sub_expr(&mut self) -> ParseResult<Option<Expr>> {
        if let Some(mut left) = self.parse_mul_div_expr()? {
            while let Some(op) = self.next_if_token(|token| {
                matches!(
                    token,
                    Token::Symbol(Symbol::Add) | Token::Symbol(Symbol::Sub)
                )
            }) {
                let right = self.expect_mul_div_expr()?;
                let binary_op = match op.token() {
                    Token::Symbol(Symbol::Add) => BinaryOp::Add,
                    Token::Symbol(Symbol::Sub) => BinaryOp::Sub,
                    _ => unreachable!(),
                };
                let span = left.span().merge(right.span());
                left = Expr::new(
                    ExprInner::Binary(Box::new(left), binary_op, Box::new(right)),
                    span,
                );
            }

            Ok(Some(left))
        } else {
            Ok(None)
        }
    }

    fn expect_add_sub_expr(&mut self) -> ParseResult<Expr> {
        match self.parse_add_sub_expr()? {
            Some(expr) => Ok(expr),
            None => Err(ParseError::ExpectedExpression {
                span: self.current_span(),
            }),
        }
    }

    fn parse_comparison_expr(&mut self) -> ParseResult<Option<Expr>> {
        if let Some(mut left) = self.parse_add_sub_expr()? {
            while let Some(op) = self.next_if_token(|token| {
                matches!(
                    token,
                    Token::Symbol(Symbol::Eq)
                        | Token::Symbol(Symbol::Neq)
                        | Token::Symbol(Symbol::Lt)
                        | Token::Symbol(Symbol::Leq)
                        | Token::Symbol(Symbol::Gt)
                        | Token::Symbol(Symbol::Geq)
                )
            }) {
                let right = self.expect_add_sub_expr()?;
                let binary_op = match op.token() {
                    Token::Symbol(Symbol::Eq) => BinaryOp::Eq,
                    Token::Symbol(Symbol::Neq) => BinaryOp::Neq,
                    Token::Symbol(Symbol::Lt) => BinaryOp::Lt,
                    Token::Symbol(Symbol::Leq) => BinaryOp::Leq,
                    Token::Symbol(Symbol::Gt) => BinaryOp::Gt,
                    Token::Symbol(Symbol::Geq) => BinaryOp::Geq,
                    _ => unreachable!(),
                };
                let span = left.span().merge(right.span());
                left = Expr::new(
                    ExprInner::Binary(Box::new(left), binary_op, Box::new(right)),
                    span,
                );
            }

            Ok(Some(left))
        } else {
            Ok(None)
        }
    }

    fn parse_unary_logical_expr(&mut self) -> ParseResult<Option<Expr>> {
        let maybe_op = self.next_if_token_eq(&Token::Keyword(Keyword::Not));
        match maybe_op {
            Some(op_token) => {
                let op_span = *op_token.span();
                let right = self.expect_unary_logical_expr()?;
                let span = op_span.merge(right.span());
                Ok(Some(Expr::new(
                    ExprInner::Unary(UnaryOp::Not, Box::new(right)),
                    span,
                )))
            }
            None => self.parse_comparison_expr(),
        }
    }

    fn expect_unary_logical_expr(&mut self) -> ParseResult<Expr> {
        match self.parse_unary_logical_expr()? {
            Some(expr) => Ok(expr),
            None => Err(ParseError::ExpectedExpression {
                span: self.current_span(),
            }),
        }
    }

    fn parse_and_or_expr(&mut self) -> ParseResult<Option<Expr>> {
        if let Some(mut left) = self.parse_unary_logical_expr()? {
            while let Some(op) = self.next_if_token(|token| {
                matches!(
                    token,
                    Token::Keyword(Keyword::Or) | Token::Keyword(Keyword::And)
                )
            }) {
                let right = self.expect_unary_logical_expr()?;
                let span = left.span().merge(right.span());
                left = Expr::new(
                    ExprInner::Binary(
                        Box::new(left),
                        match op.token() {
                            Token::Keyword(Keyword::Or) => BinaryOp::Or,
                            Token::Keyword(Keyword::And) => BinaryOp::And,
                            _ => unreachable!(),
                        },
                        Box::new(right),
                    ),
                    span,
                );
            }

            Ok(Some(left))
        } else {
            Ok(None)
        }
    }

    fn parse_expression(&mut self) -> ParseResult<Option<expression::Expr>> {
        self.parse_and_or_expr()
    }

    fn parse_binary_number(&mut self) -> Option<(BinaryNumber, Span)> {
        match self.tokens.peek() {
            Some(spanned_token) if matches!(spanned_token.token(), Token::BinaryNumber(_)) => {
                let spanned_token = self.tokens.next()?;
                if let Token::BinaryNumber(binary_number) = spanned_token.token() {
                    Some((*binary_number, *spanned_token.span()))
                } else {
                    unreachable!()
                }
            }
            _ => None,
        }
    }

    fn parse_decimal_number(&mut self) -> Option<(DecimalNumber, Span)> {
        match self.tokens.peek() {
            Some(spanned_token) if matches!(spanned_token.token(), Token::DecimalNumber(_)) => {
                let spanned_token = self.tokens.next()?;
                if let Token::DecimalNumber(decimal_number) = spanned_token.token() {
                    Some((*decimal_number, *spanned_token.span()))
                } else {
                    unreachable!()
                }
            }
            _ => None,
        }
    }

    fn parse_string_literal(&mut self) -> Option<Expr> {
        let peek = self.tokens.peek_mut()?;

        match peek.token_mut() {
            Token::StringLiteral {
                literal,
                is_quote_closed_in_source,
            } => {
                let literal = mem::take(literal);
                let is_quote_closed_in_source = *is_quote_closed_in_source;
                let span = *peek.span();
                self.tokens.next(); // Consume the string literal token

                Some(Expr::new(
                    ExprInner::StringLiteral {
                        value: literal,
                        is_quote_closed_in_source,
                    },
                    span,
                ))
            }
            _ => None,
        }
    }

    fn parse_expression_factor(&mut self) -> ParseResult<Option<Expr>> {
        if let Some(Token::Symbol(Symbol::LParen)) = self.peek_mut_token() {
            let start_span = self.current_span();
            self.tokens.next(); // Consume '('
            let expr = self.expect_expression()?;
            let end_token = self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "Expected closing parenthesis",
            )?;
            let span = start_span.merge(*end_token.span());
            return Ok(Some(Expr::new(
                ExprInner::Parentheses(Box::new(expr)),
                span,
            )));
        }

        if let Some((binary_number, span)) = self.parse_binary_number() {
            return Ok(Some(Expr::new(
                ExprInner::BinaryNumber(binary_number),
                span,
            )));
        }

        if let Some((decimal_number, span)) = self.parse_decimal_number() {
            return Ok(Some(Expr::new(
                ExprInner::DecimalNumber(decimal_number),
                span,
            )));
        }

        if let Some(expr) = self.parse_string_literal() {
            return Ok(Some(expr));
        }

        if let Some(lvalue) = self.parse_lvalue()? {
            let span = lvalue.span;
            return Ok(Some(Expr::new(ExprInner::LValue(lvalue), span)));
        }

        if let Some(function) = self.parse_function()? {
            let span = function.span;
            return Ok(Some(Expr::new(ExprInner::FunctionCall(function), span)));
        }

        Ok(None)
    }

    fn parse_function(&mut self) -> ParseResult<Option<Function>> {
        match self.peek_token() {
            Token::Keyword(Keyword::Int) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                // Create a span that merges from start to the end of the expression
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Int {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Sgn) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Sgn {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Status) => {
                let start_span = self.current_span();
                self.tokens.next();
                let arg = self.expect_expression_factor()?;
                let span = start_span.merge(arg.span());
                Ok(Some(Function::new(
                    FunctionInner::Status { arg: Box::new(arg) },
                    span,
                )))
            }
            Token::Keyword(Keyword::Val) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Val {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::StrDollar) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Str {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::ChrDollar) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Chr {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Abs) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Abs {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Len) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Len {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Peek) => {
                let start_span = self.current_span();
                self.tokens.next();
                let address = self.expect_expression_factor()?;
                let span = start_span.merge(address.span());
                Ok(Some(Function::new(
                    FunctionInner::Peek {
                        memory_area: MemoryArea::Me0,
                        address: Box::new(address),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::PeekHashTag) => {
                let start_span = self.current_span();
                self.tokens.next();
                let address = self.expect_expression_factor()?;
                let span = start_span.merge(address.span());
                Ok(Some(Function::new(
                    FunctionInner::Peek {
                        memory_area: MemoryArea::Me1,
                        address: Box::new(address),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Ln) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Ln {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Log) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Log {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Dms) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Dms {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Deg) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Deg {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Tan) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Tan {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Cos) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Cos {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Sin) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Sin {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Sqr) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Sqr {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::MidDollar) => {
                let start_span = self.current_span();
                self.tokens.next();
                // Parse '('
                self.expect_token(&Token::Symbol(Symbol::LParen), "Expected '(' after MID$")?;
                let string = self.expect_expression()?;

                self.expect_token(
                    &Token::Symbol(Symbol::Comma),
                    "Expected ',' after MID$ string",
                )?;
                let start_pos = self.expect_expression()?;

                self.expect_token(
                    &Token::Symbol(Symbol::Comma),
                    "Expected ',' after MID$ start",
                )?;
                let length = self.expect_expression()?;

                let end_token = self.expect_token(
                    &Token::Symbol(Symbol::RParen),
                    "Expected ')' after MID$ length",
                )?;
                let span = start_span.merge(*end_token.span());
                Ok(Some(Function::new(
                    FunctionInner::Mid {
                        string: Box::new(string),
                        start: Box::new(start_pos),
                        length: Box::new(length),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::LeftDollar) => {
                let start_span = self.current_span();
                self.tokens.next();
                // Parse '('
                self.expect_token(&Token::Symbol(Symbol::LParen), "Expected '(' after LEFT$")?;
                let string = self.expect_expression()?;
                // Parse ','
                self.expect_token(
                    &Token::Symbol(Symbol::Comma),
                    "Expected ',' after LEFT$ string",
                )?;
                let length = self.expect_expression()?;
                let end_token = self.expect_token(
                    &Token::Symbol(Symbol::RParen),
                    "Expected ')' after LEFT$ length",
                )?;
                let span = start_span.merge(*end_token.span());
                Ok(Some(Function::new(
                    FunctionInner::Left {
                        string: Box::new(string),
                        length: Box::new(length),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::RightDollar) => {
                let start_span = self.current_span();
                self.tokens.next();
                // Parse '('
                self.expect_token(&Token::Symbol(Symbol::LParen), "Expected '(' after RIGHT$")?;
                let string = self.expect_expression()?;
                self.expect_token(
                    &Token::Symbol(Symbol::Comma),
                    "Expected ',' after RIGHT$ string",
                )?;
                let length = self.expect_expression()?;
                let end_token = self.expect_token(
                    &Token::Symbol(Symbol::RParen),
                    "Expected ')' after RIGHT$ length",
                )?;
                let span = start_span.merge(*end_token.span());
                Ok(Some(Function::new(
                    FunctionInner::Right {
                        string: Box::new(string),
                        length: Box::new(length),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Asc) => {
                let start_span = self.current_span();
                self.tokens.next();
                let expr = self.expect_expression_factor()?;
                let span = start_span.merge(expr.span());
                Ok(Some(Function::new(
                    FunctionInner::Asc {
                        expr: Box::new(expr),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Point) => {
                let start_span = self.current_span();
                self.tokens.next();
                let position = self.expect_expression_factor()?;
                let span = start_span.merge(position.span());
                Ok(Some(Function::new(
                    FunctionInner::Point {
                        position: Box::new(position),
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Rnd) => {
                let start_span = self.current_span();
                self.tokens.next();
                let range_end = self.expect_expression_factor()?;
                let span = start_span.merge(range_end.span());
                Ok(Some(Function::new(
                    FunctionInner::Rnd {
                        range_end: Box::new(range_end),
                    },
                    span,
                )))
            }
            _ => Ok(None),
        }
    }

    fn parse_lvalue(&mut self) -> ParseResult<Option<LValue>> {
        match self.peek_token() {
            Token::Identifier(identifier) => {
                let identifier = *identifier;
                let start_span = self.current_span();
                self.tokens.next();

                // Check for array access
                if self.peek_token() == &Token::Symbol(Symbol::LParen) {
                    self.tokens.next(); // Consume '('
                    let index = self.expect_expression()?;

                    if self.peek_token() == &Token::Symbol(Symbol::Comma) {
                        self.tokens.next(); // Consume ','
                        let col_index = self.expect_expression()?;
                        let end_token = self.expect_token(
                            &Token::Symbol(Symbol::RParen),
                            "Expected ')' after 2D array indices",
                        )?;
                        let span = start_span.merge(*end_token.span());
                        return Ok(Some(LValue {
                            inner: LValueInner::Array2DAccess {
                                identifier,
                                row_index: Box::new(index),
                                col_index: Box::new(col_index),
                            },
                            span,
                        }));
                    }

                    let end_token = self.expect_token(
                        &Token::Symbol(Symbol::RParen),
                        "Expected ')' after 1D array index",
                    )?;
                    let span = start_span.merge(*end_token.span());
                    return Ok(Some(LValue {
                        inner: LValueInner::Array1DAccess {
                            identifier,
                            index: Box::new(index),
                        },
                        span,
                    }));
                }

                Ok(Some(LValue::new(
                    LValueInner::Identifier(identifier),
                    start_span,
                )))
            }
            Token::Symbol(Symbol::At) => {
                let start_span = self.current_span();
                self.tokens.next();

                let has_dollar = self
                    .next_if_token_eq(&Token::Symbol(Symbol::Dollar))
                    .is_some();

                self.expect_token(&Token::Symbol(Symbol::LParen), "Expected '(' after @")?;
                let index = self.expect_expression()?;
                let end_token = self.expect_token(
                    &Token::Symbol(Symbol::RParen),
                    "Expected ')' after memory area index",
                )?;
                let span = start_span.merge(*end_token.span());

                Ok(Some(LValue::new(
                    LValueInner::FixedMemoryAreaAccess {
                        index: Box::new(index),
                        has_dollar,
                    },
                    span,
                )))
            }
            Token::Keyword(Keyword::Time) => {
                let start_span = self.current_span();
                self.tokens.next();
                Ok(Some(LValue::new(
                    LValueInner::BuiltInIdentifier(Keyword::Time),
                    start_span,
                )))
            }
            Token::Keyword(Keyword::InkeyDollar) => {
                let start_span = self.current_span();
                self.tokens.next();
                Ok(Some(LValue::new(
                    LValueInner::BuiltInIdentifier(Keyword::InkeyDollar),
                    start_span,
                )))
            }
            Token::Keyword(Keyword::Pi) => {
                let start_span = self.current_span();
                self.tokens.next();
                Ok(Some(LValue::new(
                    LValueInner::BuiltInIdentifier(Keyword::Pi),
                    start_span,
                )))
            }
            _ => Ok(None),
        }
    }

    fn expect_lvalue(&mut self) -> ParseResult<LValue> {
        match self.parse_lvalue()? {
            Some(lvalue) => Ok(lvalue),
            None => Err(ParseError::ExpectedLValue {
                span: self.current_span(),
            }),
        }
    }

    fn parse_assignment(&mut self) -> ParseResult<Option<Assignment>> {
        if let Some(lhs) = self.parse_lvalue()? {
            let start_span = lhs.span;
            self.expect_token(&Token::Symbol(Symbol::Eq), "Expected '=' in assignment")?;
            let rhs = self.expect_expression()?;
            let end_span = rhs.span();
            let full_span = start_span.merge(end_span);
            Ok(Some(Assignment::new(lhs, Box::new(rhs), full_span)))
        } else {
            Ok(None)
        }
    }

    fn expect_assignment(&mut self) -> ParseResult<Assignment> {
        match self.parse_assignment()? {
            Some(assignment) => Ok(assignment),
            None => Err(ParseError::ExpectedAssignment {
                span: self.current_span(),
            }),
        }
    }

    fn parse_let_stmt(&mut self, is_let_mandatory: bool) -> ParseResult<Option<Statement>> {
        let let_kw = self.next_if_token_eq(&Token::Keyword(Keyword::Let));

        if is_let_mandatory {
            let start_span = if let Some(let_token) = let_kw {
                *let_token.span()
            } else {
                return Ok(None);
            };

            let mut assignments = vec![];

            loop {
                let assignment = self.expect_assignment()?;
                assignments.push(assignment);
                if self.peek_token() == &Token::Symbol(Symbol::Comma) {
                    self.tokens.next(); // Consume ','
                } else {
                    break;
                }
            }

            let end_span = assignments.last().map(|a| a.span()).unwrap_or(start_span);
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Let {
                    inner: LetInner::new(assignments, full_span),
                    is_let_kw_present_in_source: true,
                },
                full_span,
            )))
        } else if let Some(first_assignment) = self.parse_assignment()? {
            let start_span = first_assignment.span();
            let mut assignments = vec![first_assignment];

            if self.peek_token() == &Token::Symbol(Symbol::Comma) {
                self.tokens.next(); // Consume ','
                loop {
                    let assignment = self.expect_assignment()?;
                    assignments.push(assignment);
                    if self.peek_token() == &Token::Symbol(Symbol::Comma) {
                        self.tokens.next(); // Consume ','
                    } else {
                        break;
                    }
                }
            }

            let end_span = assignments.last().map(|a| a.span()).unwrap_or(start_span);
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Let {
                    inner: LetInner::new(assignments, full_span),
                    is_let_kw_present_in_source: let_kw.is_some(),
                },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_using_clause(&mut self) -> ParseResult<Option<UsingClause>> {
        if let Some(start_token) = self.next_if_token_eq(&Token::Keyword(Keyword::Using)) {
            let start_span = *start_token.span();
            let format = self.parse_expression()?;
            if let Some(format) = format {
                let full_span = start_span.merge(format.span());
                Ok(Some(UsingClause::new(Some(format), full_span)))
            } else {
                Ok(Some(UsingClause::new(None, start_span)))
            }
        } else {
            Ok(None)
        }
    }

    fn parse_print_separator(&mut self) -> PrintSeparator {
        match self.peek_token() {
            Token::Symbol(Symbol::Comma) => {
                self.tokens.next();
                PrintSeparator::Comma
            }
            Token::Symbol(Symbol::Semicolon) => {
                self.tokens.next();
                PrintSeparator::Semicolon
            }
            _ => PrintSeparator::None,
        }
    }

    fn parse_print_inner(&mut self, start_span: Span) -> ParseResult<PrintInner> {
        let mut exprs = Vec::new();

        loop {
            if let Some(expr) = self.parse_expression()? {
                let print_separator = self.parse_print_separator();
                exprs.push((Printable::Expr(expr), print_separator));
            } else if let Some(using_clause) = self.parse_using_clause()? {
                let print_separator = self.parse_print_separator();
                exprs.push((Printable::UsingClause(using_clause), print_separator));
            } else {
                break;
            }
        }

        let end_span = self.current_span();
        let full_span = start_span.merge(end_span);
        Ok(PrintInner::new(exprs, full_span))
    }

    fn parse_print_pause_stmt(&mut self) -> ParseResult<Option<Statement>> {
        match self.peek_token() {
            Token::Keyword(Keyword::Print) => {
                let start_token = self.tokens.next().unwrap();
                let start_span = *start_token.span();
                let print_inner = self.parse_print_inner(start_span)?;
                let end_span = self.current_span();
                let full_span = start_span.merge(end_span);
                Ok(Some(Statement::new(
                    StatementInner::Print { inner: print_inner },
                    full_span,
                )))
            }
            Token::Keyword(Keyword::Pause) => {
                let start_token = self.tokens.next().unwrap();
                let start_span = *start_token.span();
                let pause_inner = self.parse_print_inner(start_span)?;
                let end_span = self.current_span();
                let full_span = start_span.merge(end_span);
                Ok(Some(Statement::new(
                    StatementInner::Pause { inner: pause_inner },
                    full_span,
                )))
            }
            _ => Ok(None),
        }
    }

    fn parse_end_statement(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::End))?;
        Some(Statement::new(StatementInner::End, *start_token.span()))
    }

    fn parse_dim_inner(&mut self) -> ParseResult<Option<DimInner>> {
        match self.peek_token() {
            Token::Identifier(identifier) => {
                let identifier = *identifier;
                let start_token = self.tokens.next().unwrap();
                let start_span = *start_token.span();
                self.expect_token(
                    &Token::Symbol(Symbol::LParen),
                    "Expected '(' after identifier in DIM statement",
                )?;
                let rows = self.expect_expression()?;
                let maybe_cols = if self
                    .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                    .is_some()
                {
                    Some(self.expect_expression()?)
                } else {
                    None
                };

                self.expect_token(
                    &Token::Symbol(Symbol::RParen),
                    "Expected ')' after dimensions in DIM statement",
                )?;

                let string_length = if self.next_if_token_eq(&Token::Symbol(Symbol::Mul)).is_some()
                {
                    Some(self.expect_expression()?)
                } else {
                    None
                };

                let end_span = string_length
                    .as_ref()
                    .map(|e| e.span())
                    .unwrap_or_else(|| self.current_span());
                let full_span = start_span.merge(end_span);

                match maybe_cols {
                    Some(cols) => Ok(Some(DimInner::DimInner2D {
                        identifier,
                        rows,
                        cols,
                        string_length,
                        span: full_span,
                    })),
                    None => Ok(Some(DimInner::DimInner1D {
                        identifier,
                        size: rows,
                        string_length,
                        span: full_span,
                    })),
                }
            }
            _ => Ok(None),
        }
    }

    fn expect_dim_inner(&mut self) -> ParseResult<DimInner> {
        match self.parse_dim_inner()? {
            Some(dim_inner) => Ok(dim_inner),
            None => Err(ParseError::ExpectedDimInner {
                span: self.current_span(),
            }),
        }
    }

    fn parse_dim_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Dim))
            .is_some()
        {
            let start_span = self.current_span();
            let mut decls = vec![];

            loop {
                let decl = self.expect_dim_inner()?;
                decls.push(decl);

                if self
                    .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                    .is_none()
                {
                    break;
                }
            }

            let end_span = self.current_span();
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Dim { decls },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_rotate_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Rotate))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let end_span = self.current_span();
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Rotate { expr },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_degree_stmt(&mut self) -> Option<Statement> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Degree))
            .is_some()
        {
            let start_token = self.current_span();
            Some(Statement::new(StatementInner::Degree, start_token))
        } else {
            None
        }
    }

    fn parse_statement(&mut self, is_let_mandatory: bool) -> ParseResult<Option<Statement>> {
        if let Some(stmt) = self.parse_print_pause_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_using_clause()? {
            let span = stmt.span();
            return Ok(Some(Statement::new(
                StatementInner::Using { using_clause: stmt },
                span,
            )));
        }

        if let Some(stmt) = self.parse_end_statement() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_if_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_input_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_remark_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_for_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_next_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_clear_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_goto_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_gosub_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_on_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_wait_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_cls_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_random_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_gprint_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_gcursor_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_cursor_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_beep_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_return_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_poke_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_dim_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_degree_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_read_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_data_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_restore_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_arun_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_lock_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_unlock_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_call_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_text_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_graph_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_line_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_rline_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_color_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_csize_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_lf_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_radian_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_lcursor_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_lprint_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_glcursor_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_sorgn_stmt() {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_rotate_stmt()? {
            return Ok(Some(stmt));
        }

        if let Some(stmt) = self.parse_let_stmt(is_let_mandatory)? {
            return Ok(Some(stmt));
        }

        Ok(None)
    }

    fn parse_sorgn_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Sorgn))?;
        Some(Statement::new(StatementInner::Sorgn, *start_token.span()))
    }

    fn parse_if_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::If))
            .is_some()
        {
            let start_span = self.current_span();
            let condition = self.expect_expression()?;
            let then_kw = self.next_if_token_eq(&Token::Keyword(Keyword::Then)); // optional
            if let Some(first_stmt) = self.parse_statement(false)? {
                // En BASIC clásico (incluido este dialecto Sharp PC-1500),
                // un IF sin bloque explícito hace TODAS las sentencias
                // separadas por ':' hasta el final de la línea
                // condicionales, no solo la primera — p.ej. `IF cond
                // stmt1:stmt2:stmt3` equivale a `IF cond THEN
                // (stmt1:stmt2:stmt3)`, no a `(IF cond THEN stmt1):stmt2:
                // stmt3`. El bucle de `parse_code_line_with_recovery` que
                // llama a este método no sabe distinguir esto (trata cada
                // sentencia separada por ':' como independiente), así que
                // hay que consumir aquí mismo el resto de la línea.
                let mut then_statements = vec![first_stmt];
                while self.next_if_token_eq(&Token::Symbol(Symbol::Colon)).is_some() {
                    match self.parse_statement(false)? {
                        Some(next_stmt) => then_statements.push(next_stmt),
                        None => break,
                    }
                }

                let last_span = then_statements.last().expect("then_statements no puede estar vacío").span;
                let full_span = start_span.merge(last_span);

                let then_stmt = if then_statements.len() == 1 {
                    then_statements.into_iter().next().expect("comprobado len == 1")
                } else {
                    Statement::new(StatementInner::Multi(then_statements), full_span)
                };
                let is_goto_stmt = matches!(then_stmt.inner, StatementInner::Goto { .. });

                Ok(Some(Statement::new(
                    StatementInner::If {
                        condition,
                        then_stmt: Box::new(then_stmt),
                        is_then_kw_present_in_source: then_kw.is_some(),
                        is_goto_kw_present_in_source: is_goto_stmt,
                    },
                    full_span,
                )))
            } else {
                if then_kw.is_none() {
                    return Err(ParseError::ExpectedThenClause { span: start_span });
                }

                let goto_expr = self.expect_expression()?;
                let full_span = start_span.merge(goto_expr.span());
                let goto_stmt =
                    Statement::new(StatementInner::Goto { target: goto_expr }, full_span);
                Ok(Some(Statement::new(
                    StatementInner::If {
                        condition,
                        then_stmt: Box::new(goto_stmt),
                        is_then_kw_present_in_source: then_kw.is_some(),
                        is_goto_kw_present_in_source: false,
                    },
                    full_span,
                )))
            }
        } else {
            Ok(None)
        }
    }

    fn parse_input_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Input))
            .is_some()
        {
            let start_span = self.current_span();
            let mut input_exprs = vec![];

            loop {
                let lvalue = self.parse_lvalue()?;

                if let Some(lvalue) = lvalue {
                    input_exprs.push((None, lvalue)); // No prompt
                } else {
                    // Try to parse a prompt
                    let prompt = self.expect_expression()?;
                    // Expect ;
                    self.expect_token(
                        &Token::Symbol(Symbol::Semicolon),
                        "Expected ';' after prompt",
                    )?;

                    let lvalue = self.expect_lvalue()?;
                    input_exprs.push((Some(prompt), lvalue));
                }

                if self
                    .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                    .is_none()
                {
                    break;
                }
            }

            let end_span = self.current_span();
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Input { input_exprs },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_remark_stmt(&mut self) -> Option<Statement> {
        match self.peek_mut_token()? {
            Token::Remark(s) => {
                let s = mem::take(s);
                let start_span = self.current_span();
                self.tokens.next();
                Some(Statement::new(
                    StatementInner::Remark {
                        text: s.to_string(),
                    },
                    start_span,
                ))
            }
            _ => None,
        }
    }

    fn parse_for_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::For))
            .is_some()
        {
            let start_span = self.current_span();
            let assignment = self.expect_assignment()?;
            self.expect_token(
                &Token::Keyword(Keyword::To),
                "Expected 'TO' after FOR assignment",
            )?;
            let to_expr = self.expect_expression()?;
            let step_expr = if self
                .next_if_token_eq(&Token::Keyword(Keyword::Step))
                .is_some()
            {
                Some(self.expect_expression()?)
            } else {
                None
            };

            let end_span = step_expr
                .as_ref()
                .map(|e| e.span())
                .unwrap_or(to_expr.span());
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::For {
                    assignment,
                    to_expr,
                    step_expr,
                },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_next_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Next))
            .is_some()
        {
            let start_span = self.current_span();
            let lvalue = self.expect_lvalue()?;
            let full_span = start_span.merge(lvalue.span);
            Ok(Some(Statement::new(
                StatementInner::Next { lvalue },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_clear_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Clear))?;
        Some(Statement::new(StatementInner::Clear, *start_token.span()))
    }

    fn parse_goto_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Goto))
            .is_some()
        {
            let start_span = self.current_span();
            let target = self.expect_expression()?;
            let full_span = start_span.merge(target.span());
            Ok(Some(Statement::new(
                StatementInner::Goto { target },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_gosub_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Gosub))
            .is_some()
        {
            let start_span = self.current_span();
            let target = self.expect_expression()?;
            let full_span = start_span.merge(target.span());
            Ok(Some(Statement::new(
                StatementInner::Gosub { target },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_on_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::On))
            .is_some()
        {
            let start_span = self.current_span();
            if self
                .next_if_token_eq(&Token::Keyword(Keyword::Error))
                .is_some()
            {
                self.expect_token(
                    &Token::Keyword(Keyword::Goto),
                    "Expected 'GOTO' after 'ON ERROR'",
                )?;

                let expr = self.expect_expression()?;
                let full_span = start_span.merge(expr.span());
                return Ok(Some(Statement::new(
                    StatementInner::OnErrorGoto { target: expr },
                    full_span,
                )));
            }

            let expr = self.expect_expression()?;

            if self
                .next_if_token_eq(&Token::Keyword(Keyword::Goto))
                .is_some()
            {
                let mut targets = vec![self.expect_expression()?];
                while self
                    .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                    .is_some()
                {
                    targets.push(self.expect_expression()?);
                }
                let end_span = targets.last().unwrap().span();
                let full_span = start_span.merge(end_span);
                Ok(Some(Statement::new(
                    StatementInner::OnGoto { expr, targets },
                    full_span,
                )))
            } else if self
                .next_if_token_eq(&Token::Keyword(Keyword::Gosub))
                .is_some()
            {
                let mut targets = vec![self.expect_expression()?];
                while self
                    .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                    .is_some()
                {
                    targets.push(self.expect_expression()?);
                }
                let end_span = targets.last().unwrap().span();
                let full_span = start_span.merge(end_span);
                Ok(Some(Statement::new(
                    StatementInner::OnGosub { expr, targets },
                    full_span,
                )))
            } else {
                Err(ParseError::ExpectedGotoOrGosub {
                    span: self.current_span(),
                })
            }
        } else {
            Ok(None)
        }
    }

    fn parse_wait_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Wait))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.parse_expression()?;
            let end_span = expr.as_ref().map(|e| e.span()).unwrap_or(start_span);
            let full_span = start_span.merge(end_span);
            return Ok(Some(Statement::new(
                StatementInner::Wait { expr },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_cls_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Cls))?;
        Some(Statement::new(StatementInner::Cls, *start_token.span()))
    }

    fn parse_random_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Random))?;
        Some(Statement::new(StatementInner::Random, *start_token.span()))
    }

    fn parse_gprint_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Gprint))
            .is_some()
        {
            let start_span = self.current_span();
            let mut exprs = vec![];

            while let Some(expr) = self.parse_expression()? {
                let print_separator = self.parse_print_separator();
                exprs.push((expr, print_separator));
            }

            let end_span = self.current_span();
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Gprint { exprs },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_gcursor_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Gcursor))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let full_span = start_span.merge(expr.span());
            return Ok(Some(Statement::new(
                StatementInner::GCursor { expr },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_cursor_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Cursor))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let full_span = start_span.merge(expr.span());
            return Ok(Some(Statement::new(
                StatementInner::Cursor { expr },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_beep_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Beep))
            .is_some()
        {
            let start_span = self.current_span();
            // Check for BEEP ON/OFF
            if self.peek_token() == &Token::Keyword(Keyword::On) {
                let on_token = self.tokens.next().unwrap();
                let full_span = start_span.merge(*on_token.span());
                return Ok(Some(Statement::new(
                    StatementInner::BeepOnOff {
                        switch_beep_on: true,
                    },
                    full_span,
                )));
            }
            if self.peek_token() == &Token::Keyword(Keyword::Off) {
                let off_token = self.tokens.next().unwrap();
                let full_span = start_span.merge(*off_token.span());
                return Ok(Some(Statement::new(
                    StatementInner::BeepOnOff {
                        switch_beep_on: false,
                    },
                    full_span,
                )));
            }

            // Parse BEEP repetitions[,frequency[,duration]]
            let repetitions_expr = self.expect_expression()?;

            let optional_params = if self
                .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                .is_some()
            {
                let frequency = self.expect_expression()?;
                let duration = if self
                    .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                    .is_some()
                {
                    Some(self.expect_expression()?)
                } else {
                    None
                };
                let param_start_span = frequency.span();
                let param_end_span = duration
                    .as_ref()
                    .map(|d| d.span())
                    .unwrap_or(frequency.span());
                let param_full_span = param_start_span.merge(param_end_span);
                Some(BeepParams::new(frequency, duration, param_full_span))
            } else {
                None
            };

            let end_span = optional_params
                .as_ref()
                .and_then(|p| p.duration.as_ref())
                .map(|d| d.span())
                .or_else(|| optional_params.as_ref().map(|p| p.frequency.span()))
                .unwrap_or(repetitions_expr.span());
            let full_span = start_span.merge(end_span);
            Ok(Some(Statement::new(
                StatementInner::Beep {
                    repetitions_expr,
                    optional_params,
                },
                full_span,
            )))
        } else {
            Ok(None)
        }
    }

    fn parse_return_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Return))?;
        Some(Statement::new(StatementInner::Return, *start_token.span()))
    }

    fn parse_poke_stmt(&mut self) -> ParseResult<Option<Statement>> {
        let start_span = self.current_span();
        let memory_area = match self.peek_token() {
            Token::Keyword(Keyword::Poke) => {
                self.tokens.next();
                MemoryArea::Me0
            }
            Token::Keyword(Keyword::PokeHashTag) => {
                self.tokens.next();
                MemoryArea::Me1
            }
            _ => return Ok(None),
        };

        let mut exprs = vec![self.expect_expression()?];
        while self
            .next_if_token_eq(&Token::Symbol(Symbol::Comma))
            .is_some()
        {
            exprs.push(self.expect_expression()?);
        }

        let end_span = exprs.last().unwrap().span();
        let full_span = start_span.merge(end_span);
        Ok(Some(Statement::new(
            StatementInner::Poke { memory_area, exprs },
            full_span,
        )))
    }

    fn parse_read_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Read))
            .is_some()
        {
            let start_span = self.current_span();
            let mut destinations = vec![self.expect_lvalue()?];
            while self
                .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                .is_some()
            {
                destinations.push(self.expect_lvalue()?);
            }
            let end_span = destinations.last().unwrap().span;
            let full_span = start_span.merge(end_span);
            return Ok(Some(Statement::new(
                StatementInner::Read { destinations },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_data_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Data))
            .is_some()
        {
            let start_span = self.current_span();
            let mut exprs = vec![self.expect_expression()?];
            while self
                .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                .is_some()
            {
                exprs.push(self.expect_expression()?);
            }
            let end_span = exprs.last().unwrap().span();
            let full_span = start_span.merge(end_span);
            return Ok(Some(Statement::new(StatementInner::Data(exprs), full_span)));
        }
        Ok(None)
    }

    fn parse_restore_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Restore))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.parse_expression()?;
            let end_span = expr.as_ref().map(|e| e.span()).unwrap_or(start_span);
            let full_span = start_span.merge(end_span);
            return Ok(Some(Statement::new(
                StatementInner::Restore { expr },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_arun_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Arun))?;
        Some(Statement::new(StatementInner::Arun, *start_token.span()))
    }

    fn parse_lock_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Lock))?;
        Some(Statement::new(StatementInner::Lock, *start_token.span()))
    }

    fn parse_unlock_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Unlock))?;
        Some(Statement::new(StatementInner::Unlock, *start_token.span()))
    }

    fn parse_call_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Call))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let variable = if self
                .next_if_token_eq(&Token::Symbol(Symbol::Comma))
                .is_some()
            {
                Some(self.expect_lvalue()?)
            } else {
                None
            };
            let end_span = variable.as_ref().map(|v| v.span).unwrap_or(expr.span());
            let full_span = start_span.merge(end_span);
            return Ok(Some(Statement::new(
                StatementInner::Call { expr, variable },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_text_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Text))?;
        Some(Statement::new(StatementInner::Text, *start_token.span()))
    }

    fn parse_graph_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Graph))?;
        Some(Statement::new(StatementInner::Graph, *start_token.span()))
    }

    fn parse_line_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Line))
            .is_none()
        {
            return Ok(None);
        }

        let start_span = self.current_span();

        let mut start_point = None;
        let mut end_points = Vec::new();

        // Check if it starts with '-' (omit first point, use current position)
        if self.peek_token() == &Token::Symbol(Symbol::Sub) {
            self.next_spanned(); // consume the '-'

            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "expected '(' for coordinates",
            )?;
            let x = self.expect_expression()?;
            self.expect_token(&Token::Symbol(Symbol::Comma), "expected ',' in coordinates")?;
            let y = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "expected ')' after coordinates",
            )?;
            end_points.push((x, y));
        } else if self.peek_token() == &Token::Symbol(Symbol::LParen) {
            // Parse the first point explicitly
            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "expected '(' for coordinates",
            )?;
            let x = self.expect_expression()?;
            self.expect_token(&Token::Symbol(Symbol::Comma), "expected ',' in coordinates")?;
            let y = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "expected ')' after coordinates",
            )?;
            start_point = Some((x, y));
        }

        // Parse subsequent points preceded by '-'
        while self.peek_token() == &Token::Symbol(Symbol::Sub) {
            self.next_spanned(); // consume the '-'

            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "expected '(' for coordinates",
            )?;
            let x = self.expect_expression()?;
            self.expect_token(&Token::Symbol(Symbol::Comma), "expected ',' in coordinates")?;
            let y = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "expected ')' after coordinates",
            )?;
            end_points.push((x, y));
        }

        // Parse optional line-type parameter
        let line_type = if self.peek_token() == &Token::Symbol(Symbol::Comma) {
            self.next_spanned(); // consume comma
            if self.peek_token() == &Token::Symbol(Symbol::Comma) {
                // Empty line-type parameter
                None
            } else {
                Some(self.expect_expression()?)
            }
        } else {
            None
        };

        // Parse optional color parameter
        let color = if self.peek_token() == &Token::Symbol(Symbol::Comma) {
            self.next_spanned(); // consume comma
            if self.peek_token() == &Token::Symbol(Symbol::Comma)
                || matches!(self.peek_token(), Token::Identifier(_))
                || matches!(
                    self.peek_token(),
                    Token::Symbol(Symbol::Newline) | Token::Symbol(Symbol::Eof)
                )
            {
                // Empty color parameter or end of statement
                None
            } else {
                Some(self.expect_expression()?)
            }
        } else {
            None
        };

        // Parse optional box parameter (B)
        let is_box = if self.peek_token() == &Token::Symbol(Symbol::Comma) {
            self.next_spanned(); // consume comma
            if let Some(Token::Identifier(id)) = self.peek_mut_token() {
                if id.to_string().to_uppercase() == "B" {
                    self.next_spanned(); // consume B
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        let end_span = self.current_span();
        let full_span = start_span.merge(end_span);
        let inner = LineInner::new(start_point, end_points, line_type, color, is_box, full_span);

        Ok(Some(Statement::new(
            StatementInner::Line { inner },
            full_span,
        )))
    }

    fn parse_rline_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Rline))
            .is_none()
        {
            return Ok(None);
        }

        let start_span = self.current_span();

        let mut start_point = None;
        let mut end_points = Vec::new();

        // Check if it starts with '-' (omit first point, use current position)
        if self.peek_token() == &Token::Symbol(Symbol::Sub) {
            self.next_spanned(); // consume the '-'

            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "expected '(' for coordinates",
            )?;
            let x = self.expect_expression()?;
            self.expect_token(&Token::Symbol(Symbol::Comma), "expected ',' in coordinates")?;
            let y = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "expected ')' after coordinates",
            )?;
            end_points.push((x, y));
        } else if self.peek_token() == &Token::Symbol(Symbol::LParen) {
            // Parse the first point explicitly
            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "expected '(' for coordinates",
            )?;
            let x = self.expect_expression()?;
            self.expect_token(&Token::Symbol(Symbol::Comma), "expected ',' in coordinates")?;
            let y = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "expected ')' after coordinates",
            )?;
            start_point = Some((x, y));
        }

        // Parse subsequent points preceded by '-'
        while self.peek_token() == &Token::Symbol(Symbol::Sub) {
            self.next_spanned(); // consume the '-'

            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "expected '(' for coordinates",
            )?;
            let x = self.expect_expression()?;
            self.expect_token(&Token::Symbol(Symbol::Comma), "expected ',' in coordinates")?;
            let y = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "expected ')' after coordinates",
            )?;
            end_points.push((x, y));
        }

        // Parse optional line-type parameter
        let line_type = if self.peek_token() == &Token::Symbol(Symbol::Comma) {
            self.next_spanned(); // consume comma
            if self.peek_token() == &Token::Symbol(Symbol::Comma) {
                // Empty line-type parameter
                None
            } else {
                Some(self.expect_expression()?)
            }
        } else {
            None
        };

        // Parse optional color parameter
        let color = if self.peek_token() == &Token::Symbol(Symbol::Comma) {
            self.next_spanned(); // consume comma
            if self.peek_token() == &Token::Symbol(Symbol::Comma)
                || matches!(self.peek_token(), Token::Identifier(_))
                || matches!(
                    self.peek_token(),
                    Token::Symbol(Symbol::Newline) | Token::Symbol(Symbol::Eof)
                )
            {
                // Empty color parameter or end of statement
                None
            } else {
                Some(self.expect_expression()?)
            }
        } else {
            None
        };

        // Parse optional box parameter (B)
        let is_box = if self.peek_token() == &Token::Symbol(Symbol::Comma) {
            self.next_spanned(); // consume comma
            if let Some(Token::Identifier(id)) = self.peek_mut_token() {
                if id.to_string().to_uppercase() == "B" {
                    self.next_spanned(); // consume B
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        let end_span = self.current_span();
        let full_span = start_span.merge(end_span);
        let inner = LineInner::new(start_point, end_points, line_type, color, is_box, full_span);

        Ok(Some(Statement::new(
            StatementInner::RLine { inner },
            full_span,
        )))
    }

    fn parse_color_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Color))
            .is_some()
        {
            let start_span = self.current_span();
            let color = self.expect_expression()?;
            let full_span = start_span.merge(color.span());
            return Ok(Some(Statement::new(
                StatementInner::Color { expr: color },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_csize_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Csize))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let full_span = start_span.merge(expr.span());
            return Ok(Some(Statement::new(
                StatementInner::CSize { expr },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_lf_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Lf))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let full_span = start_span.merge(expr.span());
            return Ok(Some(Statement::new(StatementInner::Lf { expr }, full_span)));
        }
        Ok(None)
    }

    fn parse_radian_stmt(&mut self) -> Option<Statement> {
        let start_token = self.next_if_token_eq(&Token::Keyword(Keyword::Radian))?;
        Some(Statement::new(StatementInner::Radian, *start_token.span()))
    }

    fn parse_lcursor_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Lcursor))
            .is_some()
        {
            let start_span = self.current_span();
            let expr = self.expect_expression()?;
            let full_span = start_span.merge(expr.span());
            return Ok(Some(Statement::new(
                StatementInner::LCursor(LCursorClause::new(expr, full_span)),
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_lprint_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Lprint))
            .is_some()
        {
            let start_span = self.current_span();
            let inner = self.parse_lprint_inner(start_span)?;
            let end_span = self.current_span();
            let full_span = start_span.merge(end_span);
            return Ok(Some(Statement::new(
                StatementInner::LPrint { inner },
                full_span,
            )));
        }
        Ok(None)
    }

    fn parse_lprint_inner(&mut self, start_span: Span) -> ParseResult<LPrintInner> {
        let mut exprs = vec![];

        loop {
            let printable = if self.peek_token() == (&Token::Keyword(Keyword::Tab)) {
                let start_token = self.tokens.next().unwrap();
                let start_span = *start_token.span();
                let expr = self.expect_expression()?;
                let full_span = start_span.merge(expr.span());
                LPrintPrintable::LCursorClause(LCursorClause::new(expr, full_span))
            } else if let Some(expr) = self.parse_expression()? {
                LPrintPrintable::Expr(expr)
            } else {
                break;
            };

            let print_separator = self.parse_print_separator();
            let is_empty = matches!(print_separator, PrintSeparator::None);
            exprs.push((printable, print_separator));

            if is_empty {
                break;
            }
        }

        let end_span = self.current_span();
        let full_span = start_span.merge(end_span);
        Ok(LPrintInner::new(exprs, full_span))
    }

    fn parse_glcursor_stmt(&mut self) -> ParseResult<Option<Statement>> {
        if self
            .next_if_token_eq(&Token::Keyword(Keyword::Glcursor))
            .is_some()
        {
            let start_span = self.current_span();
            self.expect_token(
                &Token::Symbol(Symbol::LParen),
                "Expected '(' after GLCURSOR",
            )?;
            let x_expr = self.expect_expression()?;
            self.expect_token(
                &Token::Symbol(Symbol::Comma),
                "Expected ',' between GLCURSOR coordinates",
            )?;
            let y_expr = self.expect_expression()?;
            let rparen = self.expect_token(
                &Token::Symbol(Symbol::RParen),
                "Expected ')' after GLCURSOR coordinates",
            )?;
            let full_span = start_span.merge(*rparen.span());
            return Ok(Some(Statement::new(
                StatementInner::GlCursor { x_expr, y_expr },
                full_span,
            )));
        }
        Ok(None)
    }

    /// Parse with error collection - collects all parsing errors instead of stopping at first
    pub fn parse_with_error_recovery(&mut self) -> (Program, Vec<ParseError>) {
        let mut errors = Vec::new();
        let mut program = Program::new();

        while self.tokens.peek().is_some() {
            match self.parse_code_line_with_recovery() {
                Ok(Some(line)) => {
                    program.add_line(line);
                }
                Ok(None) => {
                    // Skip to next line on parse failure
                    self.skip_to_next_code_line();
                }
                Err(error) => {
                    errors.push(error);
                    // Skip to next line and continue parsing
                    self.skip_to_next_code_line();
                }
            }
        }

        (program, errors)
    }

    /// Skip tokens until we reach a newline or EOF
    fn skip_to_next_code_line(&mut self) {
        loop {
            match self.peek_token() {
                Token::Symbol(Symbol::Newline) => {
                    // Skip empty lines, skip newlines until we reach something different
                    while self
                        .next_if_token_eq(&Token::Symbol(Symbol::Newline))
                        .is_some()
                    {}

                    // If we reached EOF consume it
                    self.next_if_token_eq(&Token::Symbol(Symbol::Eof));
                }
                Token::Symbol(Symbol::Eof) => {
                    self.next_spanned(); // consume the eof
                    break;
                }
                _ => {
                    self.next_spanned(); // skip this token
                }
            }
        }
    }

    /// Parse a line with proper error reporting
    fn parse_code_line_with_recovery(&mut self) -> ParseResult<Option<CodeLine>> {
        let start_span = self.current_span();

        match self.peek_token() {
            Token::DecimalNumber(line_number) => {
                let line_number = line_number
                    .as_integer()
                    .and_then(|line_number| u16::try_from(line_number).ok())
                    .ok_or_else(|| ParseError::InvalidExpression {
                        message: format!(
                            "Invalid line number: expected a valid integer between 0 and 65535, got {}",
                            line_number
                        ),
                        span: start_span,
                    })?;

                self.tokens.next(); // Consume line number

                // Try to parse a line label: a string literal optionally followed by ':'
                let label = if let Some(Token::StringLiteral {
                    literal,
                    is_quote_closed_in_source,
                }) = self.peek_mut_token()
                {
                    let label = mem::take(literal);
                    let is_quote_closed_in_source = *is_quote_closed_in_source;
                    self.tokens.next(); // Consume label
                    let has_colon = self.next_if_token_eq(&Token::Symbol(Symbol::Colon)); // Consume ':'

                    Some(CodeLineLabel::new(
                        label,
                        is_quote_closed_in_source,
                        has_colon.is_some(),
                    ))
                } else {
                    None
                };

                let mut statements = vec![];

                // Parse statements with error recovery
                loop {
                    match self.parse_statement(false) {
                        Ok(Some(statement)) => {
                            statements.push(statement);
                            // Statements are separated by ':'
                            if self
                                .next_if_token_eq(&Token::Symbol(Symbol::Colon))
                                .is_none()
                            {
                                break;
                            }
                        }
                        Ok(None) => {
                            // No more statements to parse
                            break;
                        }
                        Err(parse_error) => {
                            // Return the error for higher-level error recovery
                            return Err(parse_error);
                        }
                    }
                }

                // Parse line end or EOF
                let newline = self.next_if_token_eq(&Token::Symbol(Symbol::Newline));

                // Parse as many newlines as possible
                while self
                    .next_if_token_eq(&Token::Symbol(Symbol::Newline))
                    .is_some()
                {}

                let eof = self.next_if_token_eq(&Token::Symbol(Symbol::Eof));

                if newline.is_none() && eof.is_none() {
                    return Err(ParseError::MissingToken {
                        token: "newline or end of file".to_string(),
                        span: self.current_span(),
                    });
                }

                // Calculate the full line span from start to end of line
                // let end_span = newline
                //     .map(|t| *t.span())
                //     .or_else(|| eof.map(|t| *t.span()))
                //     .unwrap_or_else(|| self.current_span());
                // let line_span = start_span.merge(end_span);

                Ok(Some(CodeLine::new(line_number, label, statements)))
            }
            _ => {
                // Check if we're at EOF - this is not an error, just no more lines
                if matches!(self.peek_token(), Token::Symbol(Symbol::Eof)) {
                    return Ok(None);
                }

                // Unexpected token at start of line
                let token = self.next_spanned().unwrap();
                Err(ParseError::UnexpectedToken {
                    expected: "line number".to_string(),
                    found: format!("{}", token.token()),
                    span: *token.span(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::Lexer;
    use crate::lex::RemarkLexOption;

    fn parse_expression_from_str(input: &str) -> Option<Expr> {
        let lexer = Lexer::new(input, RemarkLexOption::TrimWhitespace);
        let tokens: Vec<SpannedToken> = lexer.filter_map(|result| result.ok()).collect();
        let mut parser = Parser::new(tokens.into_iter());
        parser.parse_expression().ok().flatten()
    }

    #[test]
    fn test_basic_arithmetic_precedence() {
        // Test that multiplication has higher precedence than addition
        let expr = parse_expression_from_str("2 + 3 * 4").unwrap();

        // Should parse as 2 + (3 * 4), not (2 + 3) * 4
        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                match (left.inner(), right.inner()) {
                    (
                        ExprInner::DecimalNumber(_),
                        ExprInner::Binary(mul_left, BinaryOp::Mul, mul_right),
                    ) => {
                        // This is the expected structure
                        assert!(matches!(mul_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(mul_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected 2 + (3 * 4) structure, got {expr:?}"),
                }
            }
            _ => panic!("Expected addition at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_addition() {
        // Test that addition is left associative: 1 + 2 + 3 should parse as (1 + 2) + 3
        let expr = parse_expression_from_str("1 + 2 + 3").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                // Right operand should be 3
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));

                // Left operand should be (1 + 2)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Add, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }
            }
            _ => panic!("Expected addition at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_subtraction() {
        // Test that subtraction is left associative: 10 - 5 - 2 should parse as (10 - 5) - 2
        let expr = parse_expression_from_str("10 - 5 - 2").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Sub, right) => {
                // Right operand should be 2
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));

                // Left operand should be (10 - 5)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Sub, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }
            }
            _ => panic!("Expected subtraction at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_multiplication() {
        // Test that multiplication is left associative: 2 * 3 * 4 should parse as (2 * 3) * 4
        let expr = parse_expression_from_str("2 * 3 * 4").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Mul, right) => {
                // Right operand should be 4
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));

                // Left operand should be (2 * 3)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Mul, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }
            }
            _ => panic!("Expected multiplication at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_division() {
        // Test that division is left associative: 12 / 3 / 2 should parse as (12 / 3) / 2
        let expr = parse_expression_from_str("12 / 3 / 2").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Div, right) => {
                // Right operand should be 2
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));

                // Left operand should be (12 / 3)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Div, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }
            }
            _ => panic!("Expected division at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_comparison() {
        // Test that comparison operators are left associative: 1 < 2 < 3 should parse as (1 < 2) < 3
        let expr = parse_expression_from_str("1 < 2 < 3").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Lt, right) => {
                // Right operand should be 3
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));

                // Left operand should be (1 < 2)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Lt, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }
            }
            _ => panic!("Expected less-than at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_exponentiation() {
        // Test that exponentiation is left associative: 2 ^ 3 ^ 4 should parse as (2 ^ 3) ^ 4
        let expr = parse_expression_from_str("2 ^ 3 ^ 4").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Exp, right) => {
                // Left operand should be (2 ^ 3)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Exp, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::DecimalNumber(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }

                // Right operand should be 4
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));
            }
            _ => panic!("Expected exponentiation at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_and() {
        // Test that AND is left associative: A AND B AND C should parse as (A AND B) AND C
        let expr = parse_expression_from_str("A AND B AND C").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::And, right) => {
                // Left operand should be (A AND B)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::And, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::LValue(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::LValue(_)));
                    }
                    _ => panic!("Expected binary operation on left side, got {left:?}"),
                }

                // Right operand should be C
                assert!(matches!(right.inner(), ExprInner::LValue(_)));
            }
            _ => panic!("Expected AND at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_left_associativity_or() {
        // Test that OR is left associative: A OR B OR C should parse as (A OR B) OR C
        let expr = parse_expression_from_str("A OR B OR C").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Or, right) => {
                // Left operand should be (A OR B)
                assert!(matches!(right.inner(), ExprInner::LValue(_)));

                // Right operand should be (B OR C)
                match left.inner() {
                    ExprInner::Binary(inner_left, BinaryOp::Or, inner_right) => {
                        assert!(matches!(inner_left.inner(), ExprInner::LValue(_)));
                        assert!(matches!(inner_right.inner(), ExprInner::LValue(_)));
                    }
                    _ => panic!("Expected binary operation on right side, got {right:?}"),
                }
            }
            _ => panic!("Expected OR at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_operator_precedence_chain() {
        // Test complex precedence: 1 + 2 * 3 ^ 4 should parse as 1 + (2 * (3 ^ 4))
        let expr = parse_expression_from_str("1 + 2 * 3 ^ 4").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Add, right) => {
                // Left should be 1
                assert!(matches!(left.inner(), ExprInner::DecimalNumber(_)));

                // Right should be (2 * (3 ^ 4))
                match right.inner() {
                    ExprInner::Binary(mul_left, BinaryOp::Mul, mul_right) => {
                        // mul_left should be 2
                        assert!(matches!(mul_left.inner(), ExprInner::DecimalNumber(_)));

                        // mul_right should be (3 ^ 4)
                        match mul_right.inner() {
                            ExprInner::Binary(exp_left, BinaryOp::Exp, exp_right) => {
                                assert!(matches!(exp_left.inner(), ExprInner::DecimalNumber(_)));
                                assert!(matches!(exp_right.inner(), ExprInner::DecimalNumber(_)));
                            }
                            _ => panic!(
                                "Expected exponentiation on right of multiplication, got {mul_right:?}"
                            ),
                        }
                    }
                    _ => panic!("Expected multiplication on right of addition, got {right:?}"),
                }
            }
            _ => panic!("Expected addition at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_unary_minus_precedence() {
        // Test that unary minus has low precedence: -2 ^ 3 should parse as -(2 ^ 3)
        let expr = parse_expression_from_str("-2 ^ 3").unwrap();

        match expr.inner() {
            ExprInner::Unary(UnaryOp::Minus, unary_operand) => {
                let inner_expr = &unary_operand.inner();
                assert!(
                    matches!(inner_expr, ExprInner::Binary(_, BinaryOp::Exp, _)),
                    "Expected exponentiation on right of unary minus, got {inner_expr:?}"
                );
            }
            expr => panic!("Expected unary minus, got {expr:?}"),
        }
    }

    #[test]
    fn test_unary_plus_precedence() {
        // Test that unary plus has high precedence: +2 * 3 should parse as (+2) * 3
        let expr = parse_expression_from_str("+2 * 3").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::Mul, right) => {
                // Left should be (+2)
                match left.inner() {
                    ExprInner::Unary(UnaryOp::Plus, unary_operand) => {
                        assert!(matches!(unary_operand.inner(), ExprInner::DecimalNumber(_)));
                    }
                    _ => panic!("Expected unary plus, got {left:?}"),
                }

                // Right should be 3
                assert!(matches!(right.inner(), ExprInner::DecimalNumber(_)));
            }
            _ => panic!("Expected multiplication at top level, got {expr:?}"),
        }
    }

    #[test]
    fn test_not_precedence() {
        // Test that NOT has high precedence: NOT A AND B should parse as (NOT A) AND B
        let expr = parse_expression_from_str("NOT A AND B").unwrap();

        match expr.inner() {
            ExprInner::Binary(left, BinaryOp::And, right) => {
                // Left should be (NOT A)
                match left.inner() {
                    ExprInner::Unary(UnaryOp::Not, unary_operand) => {
                        assert!(matches!(unary_operand.inner(), ExprInner::LValue(_)));
                    }
                    _ => panic!("Expected NOT operation, got {left:?}"),
                }

                // Right should be B
                assert!(matches!(right.inner(), ExprInner::LValue(_)));
            }
            _ => panic!("Expected AND at top level, got {expr:?}"),
        }
    }
}
