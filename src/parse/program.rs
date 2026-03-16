use crate::parse::code_line::CodeLine;
use std::collections::BTreeMap;

pub struct Program {
    lines: BTreeMap<u16, CodeLine>,
}

impl Program {
    pub fn new() -> Self {
        Self {
            lines: BTreeMap::new(),
        }
    }

    pub fn write_bytes(&self, bytes: &mut Vec<u8>, preserve_source_wording: bool) {
        for line in self.lines.values() {
            line.write_bytes(bytes, preserve_source_wording);

            // Carriage return at the end of every line
            bytes.push(b'\r');
        }
    }

    pub fn add_line(&mut self, line: CodeLine) {
        self.lines.insert(line.number(), line);
    }

    pub fn lines(&self) -> impl Iterator<Item = &CodeLine> {
        self.lines.values()
    }

    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn show(&self, preserve_source_wording: bool) -> String {
        self.lines
            .values()
            .map(|line| line.show(preserve_source_wording))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
