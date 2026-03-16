pub use codespan::Span;
use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
use std::ops::Range;

/// Error types that can occur during lexical analysis
#[derive(Debug, Clone)]
pub enum LexError {
    UnexpectedCharacter { ch: char, span: Span },
    UnterminatedStringLiteral { span: Span },
    InvalidBinaryNumber { text: String, span: Span },
    InvalidRemark { span: Span },
}

/// Error types that can occur during parsing
#[derive(Debug, Clone)]
pub enum ParseError {
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },
    UnexpectedEndOfInput {
        expected: String,
        span: Span,
    },
    InvalidExpression {
        message: String,
        span: Span,
    },
    MissingToken {
        token: String,
        span: Span,
    },
    ExpectedExpression {
        span: Span,
    },
    ExpectedExplicitLet {
        span: Span,
    },
    ExpectedAssignment {
        span: Span,
    },
    ExpectedDimInner {
        span: Span,
    },
    ExpectedStatement {
        span: Span,
    },
    ExpectedLValue {
        span: Span,
    },
    ExpectedGotoOrGosub {
        span: Span,
    },
    ExpectedThenClause {
        span: Span,
    },
}

/// Error types that can occur during semantic analysis
#[derive(Debug, Clone)]
pub enum SemanticError {
    TypeMismatch {
        expected: crate::semantic_analysis::BasicType,
        found: crate::semantic_analysis::BasicType,
        span: Span,
    },
    InvalidArrayIndex {
        message: String,
        span: Span,
    },
    InvalidFunctionArgument {
        function_name: String,
        message: String,
        span: Span,
    },
    InvalidExpression {
        message: String,
        span: Span,
    },
}

/// Combined error type for all compilation phases
#[derive(Debug, Clone)]
pub enum CompileError {
    Lex(LexError),
    Parse(ParseError),
    Semantic(SemanticError),
}

impl From<LexError> for CompileError {
    fn from(err: LexError) -> Self {
        CompileError::Lex(err)
    }
}

impl From<ParseError> for CompileError {
    fn from(err: ParseError) -> Self {
        CompileError::Parse(err)
    }
}

impl From<SemanticError> for CompileError {
    fn from(err: SemanticError) -> Self {
        CompileError::Semantic(err)
    }
}

/// Pretty-print errors using codespan
pub fn print_error(error: &CompileError, filename: &str, source: &str) {
    let file = SimpleFile::new(filename, source);

    let diagnostic = match error {
        CompileError::Lex(lex_err) => match lex_err {
            LexError::UnexpectedCharacter { ch, span } => Diagnostic::error()
                .with_code("L001")
                .with_message(format!("Unexpected character '{ch}'"))
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message(format!("unexpected character '{ch}'")),
                ]),
            LexError::UnterminatedStringLiteral { span } => Diagnostic::error()
                .with_code("L002")
                .with_message("Unterminated string literal")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message("string literal is not closed"),
                ]),
            LexError::InvalidBinaryNumber { text, span } => Diagnostic::error()
                .with_code("L003")
                .with_message("Invalid binary number format")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message(format!("invalid binary number '{text}'")),
                ]),
            LexError::InvalidRemark { span } => Diagnostic::error()
                .with_code("L004")
                .with_message("Invalid REM statement")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("invalid REM syntax"),
                ]),
        },
        CompileError::Parse(parse_err) => match parse_err {
            ParseError::UnexpectedToken {
                expected,
                found,
                span,
            } => Diagnostic::error()
                .with_code("P001")
                .with_message(format!("Expected {expected}, found {found}"))
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message(format!("expected {expected}")),
                ]),
            ParseError::UnexpectedEndOfInput { expected, span } => Diagnostic::error()
                .with_code("P002")
                .with_message(format!("Unexpected end of input, expected {expected}"))
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message(format!("expected {expected}")),
                ]),
            ParseError::InvalidExpression { message, span } => Diagnostic::error()
                .with_code("P003")
                .with_message("Invalid expression")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message(message),
                ]),
            ParseError::MissingToken { token, span } => Diagnostic::error()
                .with_code("P004")
                .with_message(format!("Missing {token}"))
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message(format!("expected {token}")),
                ]),
            ParseError::ExpectedExpression { span } => Diagnostic::error()
                .with_code("P005")
                .with_message("Expected expression")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("expected expression"),
                ]),
            ParseError::ExpectedExplicitLet { span } => Diagnostic::error()
                .with_code("P006")
                .with_message("Expected explicit 'LET' keyword for let binding")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("expected 'LET' keyword"),
                ]),
            ParseError::ExpectedAssignment { span } => Diagnostic::error()
                .with_code("P007")
                .with_message("Expected assignment")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("expected assignment"),
                ]),
            ParseError::ExpectedDimInner { span } => Diagnostic::error()
                .with_code("P008")
                .with_message("Expected dimension specification")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message("expected dimension specification"),
                ]),
            ParseError::ExpectedStatement { span } => Diagnostic::error()
                .with_code("P009")
                .with_message("Expected statement")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("expected statement"),
                ]),
            ParseError::ExpectedLValue { span } => Diagnostic::error()
                .with_code("P010")
                .with_message("Expected lvalue")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("expected lvalue"),
                ]),
            ParseError::ExpectedGotoOrGosub { span } => Diagnostic::error()
                .with_code("P011")
                .with_message("Expected 'GOTO' or 'GOSUB'")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message("expected 'GOTO' or 'GOSUB'"),
                ]),
            ParseError::ExpectedThenClause { span } => Diagnostic::error()
                .with_code("P012")
                .with_message("Expected 'THEN' clause")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message("expected 'THEN' clause"),
                ]),
        },
        CompileError::Semantic(semantic_err) => match semantic_err {
            SemanticError::TypeMismatch {
                expected,
                found,
                span,
            } => Diagnostic::error()
                .with_code("S001")
                .with_message("Type mismatch")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span))
                        .with_message(format!("expected {expected:?}, found {found:?}")),
                ]),
            SemanticError::InvalidArrayIndex { message, span } => Diagnostic::error()
                .with_code("S002")
                .with_message("Invalid array index")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message(message),
                ]),
            SemanticError::InvalidFunctionArgument {
                function_name,
                message,
                span,
            } => Diagnostic::error()
                .with_code("S003")
                .with_message(format!("Invalid argument to function '{function_name}'"))
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message(message),
                ]),
            SemanticError::InvalidExpression { message, span } => Diagnostic::error()
                .with_code("S004")
                .with_message("Invalid expression")
                .with_labels(vec![
                    Label::primary((), span_to_range(*span)).with_message(message),
                ]),
        },
    };

    let writer = StandardStream::stderr(ColorChoice::Always);
    let config = codespan_reporting::term::Config::default();

    if let Err(e) = term::emit(&mut writer.lock(), &config, &file, &diagnostic) {
        eprintln!("Error printing diagnostic: {e}");
    };
}

/// Convert a codespan::Span to a Range<usize> for codespan-reporting
fn span_to_range(span: Span) -> Range<usize> {
    let start: usize = span.start().into();
    let end: usize = span.end().into();
    start..end
}

pub type LexResult<T> = Result<T, LexError>;
pub type ParseResult<T> = Result<T, ParseError>;
pub type SemanticResult<T> = Result<T, SemanticError>;
pub type CompileResult<T> = Result<T, CompileError>;
