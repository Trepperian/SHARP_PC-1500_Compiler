use crate::parse::statement::Statement;

pub struct CodeLineLabel {
    name: String,
    are_quotes_closed_in_source: bool,
    has_colon_in_source: bool,
}

impl CodeLineLabel {
    pub fn new(name: String, are_quotes_closed_in_source: bool, has_colon_in_source: bool) -> Self {
        Self {
            name,
            are_quotes_closed_in_source,
            has_colon_in_source,
        }
    }
}

pub struct CodeLine {
    number: u16,
    label: Option<CodeLineLabel>,
    statements: Vec<Statement>,
}

impl CodeLine {
    pub fn new(number: u16, label: Option<CodeLineLabel>, statements: Vec<Statement>) -> Self {
        Self {
            number,
            label,
            statements,
        }
    }

    pub fn statements(&self) -> &Vec<Statement> {
        &self.statements
    }

    pub fn number(&self) -> u16 {
        self.number
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_ref().map(|label| label.name.as_str())
    }

    pub fn write_bytes(&self, bytes: &mut Vec<u8>, preserve_source_wording: bool) {
        // Line number in big-endian (PC-1500 format)
        bytes.extend_from_slice(self.number.to_be_bytes().as_slice());
        // Add placeholder bytes for line size
        let line_size_pos = bytes.len();
        bytes.push(0);

        if let Some(label) = &self.label {
            bytes.push(b'"');
            bytes.extend_from_slice(label.name.as_bytes());
            if label.are_quotes_closed_in_source {
                bytes.push(b'"');
            }
            if label.has_colon_in_source {
                bytes.push(b':');
            }
        }
        for (i, statement) in self.statements.iter().enumerate() {
            statement.write_bytes(bytes, preserve_source_wording);
            if i < self.statements.len() - 1 {
                bytes.push(b':');
            }
        }
        // Update the line size placeholder
        let line_size = (bytes.len() - line_size_pos) as u8;
        bytes[line_size_pos] = line_size;
    }

    pub fn show(&self, preserve_source_wording: bool) -> String {
        let mut result = format!("{} ", self.number);
        if let Some(label) = &self.label {
            result.push_str(&format!("\"{}", label.name));

            if !preserve_source_wording || label.are_quotes_closed_in_source {
                result.push('\"');
            }

            if label.has_colon_in_source {
                result.push(':');
            }
        }
        if !self.statements.is_empty() {
            result.push_str(
                &self
                    .statements
                    .iter()
                    .map(|stmt| stmt.show(preserve_source_wording))
                    .collect::<Vec<_>>()
                    .join(": ")
                    .to_string(),
            );
        }
        result
    }
}
