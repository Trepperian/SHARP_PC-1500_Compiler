/// Generador de archivos BASIC tokenizados para PC-1500
/// 
/// Formato de línea: [NumLínea: 2 bytes LE] [Longitud: 1 byte] [Contenido] [0x0D]
/// 
/// Estructura:
/// - Número de línea: 2 bytes little-endian (ej: 0x00 0x0A = línea 10)
/// - Longitud: 1 byte = bytes desde aquí hasta 0x0D inclusive
/// - Contenido tokenizado:
///   - Strings: 0x22 + ASCII + 0x22
///   - Comandos: Tokens 2 bytes 0xF0xx o 0xF1xx
///   - Números/Variables: ASCII directo
///   - POKE múltiple: POKE addr,v1,v2,v3,... (addr inicial + valores consecutivos)
///   - Operadores: ASCII (: = + - * ( ))
/// - Fin de línea: 0x0D
///
/// Carga en memoria:
/// - Archivo completo → dirección 0x40C5
/// - Configurar punteros en 0x7865-0x7866 (inicio), 0x7867-0x7868 (fin)

use std::collections::HashMap;

/// Tokens BASIC del PC-1500
#[derive(Debug, Clone, Copy)]
pub enum BasicToken {
    // Comandos básicos
    Print = 0xF097,
    Input = 0xF091,
    Let = 0xF198,
    Rem = 0xF1AB,
    Dim = 0xF18B,
    Clear = 0xF187,
    Wait = 0xF1B3,

    // Control de flujo
    If = 0xF196,
    Then = 0xF1AE,
    For = 0xF1A5,
    To = 0xF1B1,
    Step = 0xF1AD,
    Next = 0xF19A,
    Goto = 0xF192,
    Gosub = 0xF194,
    Return = 0xF199,
    End = 0xF18E,

    // Gráficos
    Line = 0xF0B7,
    Gcursor = 0xF093,
    Gprint = 0xF09F,
    Point = 0xF168,

    // Memoria y sistema
    Poke = 0xF1A1,
    Peek = 0xF16F,
    Call = 0xF18A,
}

impl BasicToken {
    /// Obtener los dos bytes del token en big-endian (como se almacenan)
    pub fn to_bytes(self) -> [u8; 2] {
        let val = self as u16;
        [(val >> 8) as u8, (val & 0xFF) as u8]
    }
}

/// Generador de programas BASIC tokenizados para PC-1500
pub struct Pc1500Tokenizer {
    /// Contenido tokenizado completo
    output: Vec<u8>,
    
    /// Dirección de inicio en memoria (típicamente 0x40C5)
    start_address: u16,
    
    /// Punteros de inicio/fin del programa
    ptr_start: u16,  // 0x7865-0x7866
    ptr_end: u16,    // 0x7867-0x7868
}

impl Pc1500Tokenizer {
    /// Crear nuevo tokenizador
    pub fn new() -> Self {
        Self {
            output: Vec::new(),
            start_address: 0x40C5,
            ptr_start: 0x7865,
            ptr_end: 0x7867,
        }
    }
    
    /// Añadir una línea BASIC tokenizada
    /// 
    /// # Argumentos
    /// * `line_num` - Número de línea (10, 20, 30, ...)
    /// * `content` - Contenido tokenizado de la línea (sin el 0x0D final)
    pub fn add_line(&mut self, line_num: u16, content: &[u8]) {
        // Número de línea (2 bytes little-endian)
        self.output.push((line_num & 0xFF) as u8);
        self.output.push((line_num >> 8) as u8);
        
        // Longitud: bytes desde aquí hasta 0x0D inclusive
        let length = (content.len() + 2) as u8; // +1 para longitud, +1 para 0x0D
        self.output.push(length);
        
        // Contenido
        self.output.extend_from_slice(content);
        
        // Fin de línea
        self.output.push(0x0D);
    }
    
    /// Añadir una línea REM (comentario)
    pub fn add_rem_line(&mut self, line_num: u16, comment: &str) {
        let mut content = Vec::new();
        
        // Token REM
        content.extend_from_slice(&BasicToken::Rem.to_bytes());
        
        // Texto del comentario en ASCII
        content.extend_from_slice(comment.as_bytes());
        
        self.add_line(line_num, &content);
    }
    
    /// Añadir una línea CLEAR
    pub fn add_clear_line(&mut self, line_num: u16) {
        let content = BasicToken::Clear.to_bytes();
        self.add_line(line_num, &content);
    }
    
    /// Añadir una línea END
    pub fn add_end_line(&mut self, line_num: u16) {
        let content = BasicToken::End.to_bytes();
        self.add_line(line_num, &content);
    }
    
    /// Añadir línea POKE para escribir un byte en memoria
    /// Formato: POKE dirección, valor
    pub fn add_poke_line(&mut self, line_num: u16, address: u16, value: u8) {
        let mut content = Vec::new();
        
        // Token POKE
        content.extend_from_slice(&BasicToken::Poke.to_bytes());
        
        // Espacio
        content.push(b' ');
        
        // Dirección en ASCII
        let addr_str = format!("{}", address);
        content.extend_from_slice(addr_str.as_bytes());
        
        // Coma
        content.push(b',');
        
        // Valor en ASCII
        let val_str = format!("{}", value);
        content.extend_from_slice(val_str.as_bytes());
        
        self.add_line(line_num, &content);
    }
    
    /// Añadir línea con POKE múltiple (sintaxis nativa PC-1500)
    /// Formato: POKE addr,val1,val2,val3,...
    /// La calculadora almacena val1 en addr, val2 en addr+1, etc.
    pub fn add_multi_poke_line(&mut self, line_num: u16, pokes: &[(u16, u8)]) {
        if pokes.is_empty() {
            return;
        }

        let mut content = Vec::new();

        // Token POKE
        content.extend_from_slice(&BasicToken::Poke.to_bytes());

        // Espacio
        content.push(b' ');

        // Solo la dirección inicial
        let addr_str = format!("{}", pokes[0].0);
        content.extend_from_slice(addr_str.as_bytes());

        // Todos los valores consecutivos separados por comas
        for &(_, value) in pokes.iter() {
            content.push(b',');
            let val_str = format!("{}", value);
            content.extend_from_slice(val_str.as_bytes());
        }

        self.add_line(line_num, &content);
    }
    
    /// Añadir línea CALL para ejecutar código en una dirección
    pub fn add_call_line(&mut self, line_num: u16, address: u16) {
        let mut content = Vec::new();
        
        // Token CALL
        content.extend_from_slice(&BasicToken::Call.to_bytes());
        
        // Espacio
        content.push(b' ');
        
        // Dirección en ASCII
        let addr_str = format!("{}", address);
        content.extend_from_slice(addr_str.as_bytes());
        
        self.add_line(line_num, &content);
    }
    
    /// Generar programa loader para código máquina LH5801
    ///
    /// Crea un programa BASIC tokenizado que:
    /// 1. Hace CLEAR para limpiar memoria
    /// 2. Usa POKE addr,v1,v2,... para cargar el código máquina
    /// 3. Ejecuta el código con CALL
    pub fn generate_loader(&mut self, machine_code: &[u8], load_address: u16) {
        const POKES_PER_LINE: usize = 16; // Agrupar 16 POKEs por línea para minimizar tamaño
        
        let mut line_num = 10;
        
        // Línea de comentario inicial
        self.add_rem_line(line_num, "Native code loader");
        line_num += 10;
        
        // CLEAR para limpiar variables
        self.add_clear_line(line_num);
        line_num += 10;
        
        // POKEs agrupados para cargar el código máquina
        for (chunk_idx, chunk) in machine_code.chunks(POKES_PER_LINE).enumerate() {
            let mut pokes = Vec::new();
            
            for (byte_idx, &byte) in chunk.iter().enumerate() {
                let addr = load_address + ((chunk_idx * POKES_PER_LINE) + byte_idx) as u16;
                pokes.push((addr, byte));
            }
            
            self.add_multi_poke_line(line_num, &pokes);
            line_num += 10;
            
            // Evitar números de línea muy grandes
            if line_num > 60000 {
                eprintln!("WARNING: Código demasiado grande, alcanzado límite de líneas BASIC");
                break;
            }
        }
        
        // REM antes de ejecutar
        self.add_rem_line(line_num, "Execute native code");
        line_num += 10;
        
        // CALL para ejecutar el código
        self.add_call_line(line_num, load_address);
        line_num += 10;
        
        // END
        self.add_end_line(line_num);
    }
    
    /// Obtener el programa tokenizado completo
    pub fn finalize(&self) -> Vec<u8> {
        self.output.clone()
    }
    
    /// Obtener información sobre el programa generado
    pub fn info(&self) -> TokenizedProgramInfo {
        TokenizedProgramInfo {
            total_bytes: self.output.len(),
            start_address: self.start_address,
            end_address: self.start_address + self.output.len() as u16,
            ptr_start: self.ptr_start,
            ptr_end: self.ptr_end,
        }
    }
}

/// Información sobre el programa tokenizado
#[derive(Debug)]
pub struct TokenizedProgramInfo {
    pub total_bytes: usize,
    pub start_address: u16,
    pub end_address: u16,
    pub ptr_start: u16,
    pub ptr_end: u16,
}

impl Default for Pc1500Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simple_program() {
        let mut tokenizer = Pc1500Tokenizer::new();
        
        // 10 REM TEST
        tokenizer.add_rem_line(10, "TEST");
        
        // 20 END
        tokenizer.add_end_line(20);
        
        let output = tokenizer.finalize();
        
        // Verificar estructura básica
        assert!(!output.is_empty());
        
        // Línea 10: número (0x0A, 0x00), longitud, REM token, texto, 0x0D
        assert_eq!(output[0], 0x0A); // Línea 10 low byte
        assert_eq!(output[1], 0x00); // Línea 10 high byte
        
        // El tercer byte es la longitud
        let len1 = output[2];
        assert!(len1 > 0);
    }
    
    #[test]
    fn test_poke_line() {
        let mut tokenizer = Pc1500Tokenizer::new();
        
        // 10 POKE 14336,255
        tokenizer.add_poke_line(10, 14336, 255);
        
        let output = tokenizer.finalize();
        
        // Verificar que contiene el token POKE (0xF1, 0xA1)
        assert!(output.contains(&0xF1));
        assert!(output.iter().position(|&x| x == 0xF1).map(|i| output[i+1]) == Some(0xA1));
    }
}
