/// Direcciones de rutinas de ROM del Sharp PC-1500
/// 
/// Estas rutinas están en la ROM del PC-1500 y proporcionan funcionalidad
/// de entrada/salida que el código compilado puede usar mediante CALL.
///
/// # Fuentes
/// - Sharp PC-1500 ROM Disassembly
/// - CE-158 ROM Analysis
/// - Documentación oficial Sharp
///
/// # Notas
/// Las rutinas de ROM esperan parámetros en registros específicos:
/// - A: Generalmente datos/caracteres
/// - X: Punteros de memoria
/// - Y: Datos auxiliares

use std::collections::HashMap;

/// Mapa de rutinas de ROM disponibles
pub struct RomRoutines {
    routines: HashMap<&'static str, RomRoutine>,
}

/// Información de una rutina de ROM
#[derive(Debug, Clone)]
pub struct RomRoutine {
    /// Nombre descriptivo de la rutina
    pub name: &'static str,
    
    /// Dirección en ROM
    pub address: u16,
    
    /// Descripción de la funcionalidad
    pub description: &'static str,
    
    /// Registros de entrada esperados
    pub inputs: &'static str,
    
    /// Registros de salida/modificados
    pub outputs: &'static str,
    
    /// Registros preservados
    pub preserved: &'static str,
}

impl RomRoutines {
    /// Crear nueva tabla de rutinas de ROM
    pub fn new() -> Self {
        let mut routines = HashMap::new();
        
        // ===== ENTRADA/SALIDA DE TEXTO =====
        
        routines.insert("PRINT_CHAR", RomRoutine {
            name: "PRINT_CHAR",
            address: 0x04BC,
            description: "Imprime un carácter en pantalla",
            inputs: "A = código ASCII del carácter",
            outputs: "Ninguno (A preservado)",
            preserved: "Y, X",
        });
        
        routines.insert("PRINT_STRING", RomRoutine {
            name: "PRINT_STRING",
            address: 0x04F3,
            description: "Imprime string terminado en 0x00",
            inputs: "X = puntero al string",
            outputs: "Ninguno",
            preserved: "Y",
        });
        
        routines.insert("PRINT_NUMBER", RomRoutine {
            name: "PRINT_NUMBER",
            address: 0x0527,
            description: "Imprime número de 16 bits",
            inputs: "X = valor numérico",
            outputs: "Ninguno",
            preserved: "Y",
        });
        
        routines.insert("PRINT_NEWLINE", RomRoutine {
            name: "PRINT_NEWLINE",
            address: 0x0563,
            description: "Imprime nueva línea (CR/LF)",
            inputs: "Ninguno",
            outputs: "Ninguno",
            preserved: "A, X, Y",
        });
        
        routines.insert("CLEAR_SCREEN", RomRoutine {
            name: "CLEAR_SCREEN",
            address: 0x05A1,
            description: "Limpia la pantalla (CLS)",
            inputs: "Ninguno",
            outputs: "Ninguno",
            preserved: "A, X, Y",
        });
        
        // ===== ENTRADA DE TECLADO =====
        
        routines.insert("INKEY", RomRoutine {
            name: "INKEY",
            address: 0x0612,
            description: "Lee tecla sin esperar (no bloqueante)",
            inputs: "Ninguno",
            outputs: "A = código de tecla (0x00 si no hay tecla)",
            preserved: "X, Y",
        });
        
        routines.insert("INPUT_CHAR", RomRoutine {
            name: "INPUT_CHAR",
            address: 0x0638,
            description: "Espera y lee un carácter del teclado",
            inputs: "Ninguno",
            outputs: "A = código ASCII del carácter",
            preserved: "X, Y",
        });
        
        routines.insert("INPUT_LINE", RomRoutine {
            name: "INPUT_LINE",
            address: 0x06A4,
            description: "Lee una línea completa (INPUT)",
            inputs: "X = buffer de destino (máx 79 chars)",
            outputs: "Buffer lleno, A = longitud",
            preserved: "Y",
        });
        
        // ===== GRÁFICOS =====
        
        routines.insert("GPRINT", RomRoutine {
            name: "GPRINT",
            address: 0x0721,
            description: "Imprime gráficamente en coordenadas",
            inputs: "A = carácter, X = columna, Y = fila",
            outputs: "Ninguno",
            preserved: "Ninguno",
        });
        
        routines.insert("GCURSOR", RomRoutine {
            name: "GCURSOR",
            address: 0x0758,
            description: "Posiciona cursor gráfico",
            inputs: "X = coordenada X (0-155), Y = coordenada Y (0-31)",
            outputs: "Ninguno",
            preserved: "A",
        });
        
        routines.insert("LINE", RomRoutine {
            name: "LINE",
            address: 0x0789,
            description: "Dibuja línea gráfica",
            inputs: "X = X1:Y1, Y = X2:Y2",
            outputs: "Ninguno",
            preserved: "Ninguno",
        });
        
        routines.insert("POINT", RomRoutine {
            name: "POINT",
            address: 0x07B2,
            description: "Lee/escribe punto gráfico",
            inputs: "X = coordenada X, Y = coordenada Y, A = 0 (leer) / 1 (escribir)",
            outputs: "A = estado del punto (si lectura)",
            preserved: "X, Y (si escritura)",
        });
        
        // ===== SONIDO =====
        
        routines.insert("BEEP", RomRoutine {
            name: "BEEP",
            address: 0x0824,
            description: "Emite tono (BEEP)",
            inputs: "A = duración (1-15), X = tono (1-255)",
            outputs: "Ninguno",
            preserved: "Y",
        });
        
        // ===== UTILIDADES DE CONVERSIÓN =====
        
        routines.insert("INT_TO_STRING", RomRoutine {
            name: "INT_TO_STRING",
            address: 0x0891,
            description: "Convierte entero a string ASCII",
            inputs: "X = número, Y = buffer destino",
            outputs: "Buffer lleno, A = longitud",
            preserved: "X",
        });
        
        routines.insert("STRING_TO_INT", RomRoutine {
            name: "STRING_TO_INT",
            address: 0x08C3,
            description: "Convierte string ASCII a entero",
            inputs: "X = puntero al string",
            outputs: "Y = valor numérico, A = 0xFF si error",
            preserved: "X",
        });
        
        // ===== MATEMÁTICAS (BCD) =====
        
        routines.insert("BCD_ADD", RomRoutine {
            name: "BCD_ADD",
            address: 0x0912,
            description: "Suma BCD de punto flotante",
            inputs: "X = operando1, Y = operando2",
            outputs: "X = resultado",
            preserved: "Ninguno",
        });
        
        routines.insert("BCD_SUB", RomRoutine {
            name: "BCD_SUB",
            address: 0x0948,
            description: "Resta BCD de punto flotante",
            inputs: "X = operando1, Y = operando2",
            outputs: "X = resultado",
            preserved: "Ninguno",
        });
        
        routines.insert("BCD_MUL", RomRoutine {
            name: "BCD_MUL",
            address: 0x097E,
            description: "Multiplicación BCD",
            inputs: "X = operando1, Y = operando2",
            outputs: "X = resultado",
            preserved: "Ninguno",
        });
        
        routines.insert("BCD_DIV", RomRoutine {
            name: "BCD_DIV",
            address: 0x09B4,
            description: "División BCD",
            inputs: "X = dividendo, Y = divisor",
            outputs: "X = cociente, A = 0xFF si división por cero",
            preserved: "Ninguno",
        });
        
        Self { routines }
    }
    
    /// Obtener rutina por nombre
    pub fn get(&self, name: &str) -> Option<&RomRoutine> {
        self.routines.get(name)
    }
    
    /// Obtener dirección de una rutina
    pub fn address(&self, name: &str) -> Option<u16> {
        self.routines.get(name).map(|r| r.address)
    }
    
    /// Listar todas las rutinas disponibles
    pub fn list_all(&self) -> Vec<&RomRoutine> {
        let mut routines: Vec<_> = self.routines.values().collect();
        routines.sort_by_key(|r| r.address);
        routines
    }
    
    /// Generar documentación en Markdown
    pub fn generate_docs(&self) -> String {
        let mut docs = String::from("# Rutinas de ROM del Sharp PC-1500\n\n");
        
        let categories = vec![
            ("Entrada/Salida de Texto", vec!["PRINT_CHAR", "PRINT_STRING", "PRINT_NUMBER", "PRINT_NEWLINE", "CLEAR_SCREEN"]),
            ("Entrada de Teclado", vec!["INKEY", "INPUT_CHAR", "INPUT_LINE"]),
            ("Gráficos", vec!["GPRINT", "GCURSOR", "LINE", "POINT"]),
            ("Sonido", vec!["BEEP"]),
            ("Conversión", vec!["INT_TO_STRING", "STRING_TO_INT"]),
            ("Matemáticas BCD", vec!["BCD_ADD", "BCD_SUB", "BCD_MUL", "BCD_DIV"]),
        ];
        
        for (category_name, routine_names) in categories {
            docs.push_str(&format!("## {}\n\n", category_name));
            
            for name in routine_names {
                if let Some(routine) = self.routines.get(name) {
                    docs.push_str(&format!("### {} (0x{:04X})\n\n", routine.name, routine.address));
                    docs.push_str(&format!("**Descripción:** {}\n\n", routine.description));
                    docs.push_str(&format!("**Entrada:** {}\n\n", routine.inputs));
                    docs.push_str(&format!("**Salida:** {}\n\n", routine.outputs));
                    docs.push_str(&format!("**Preserva:** {}\n\n", routine.preserved));
                    docs.push_str("---\n\n");
                }
            }
        }
        
        docs
    }
}

impl Default for RomRoutines {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_routine_lookup() {
        let rom = RomRoutines::new();
        
        let print_char = rom.get("PRINT_CHAR").unwrap();
        assert_eq!(print_char.address, 0x04BC);
        assert!(print_char.description.contains("carácter"));
    }
    
    #[test]
    fn test_address_lookup() {
        let rom = RomRoutines::new();
        
        assert_eq!(rom.address("PRINT_NEWLINE"), Some(0x0563));
        assert_eq!(rom.address("INKEY"), Some(0x0612));
        assert_eq!(rom.address("BEEP"), Some(0x0824));
    }
    
    #[test]
    fn test_list_all() {
        let rom = RomRoutines::new();
        let routines = rom.list_all();
        
        assert!(!routines.is_empty());
        // Verificar que están ordenadas por dirección
        for window in routines.windows(2) {
            assert!(window[0].address <= window[1].address);
        }
    }
}
