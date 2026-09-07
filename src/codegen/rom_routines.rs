/// Direcciones de rutinas de ROM del Sharp PC-1500
///
/// Estas rutinas están en la ROM del PC-1500 y proporcionan funcionalidad
/// de entrada/salida que el código compilado puede usar mediante CALL.
///
/// # Fuentes
/// Desensamblado real de la ROM (`PC-1500_ROM-A0x.lh5801.asm`, proyecto
/// `Sharp_PC-1500_ROM_Disassembly` de Jeff Birt, incluido como submódulo en
/// el repo hermano `PC-1500-Emulator`). Las direcciones y convenciones de
/// llamada de las rutinas marcadas `verified: true` se obtuvieron leyendo el
/// código real alrededor de cada dirección, no adivinando por el nombre.
///
/// # Historial
/// La tabla anterior (direcciones en el rango `0x04xx-0x09xx`) era
/// completamente inventada: ese rango no es ni ROM (`$C000+`) ni RAM de
/// usuario (`$4000+`) en el mapa de memoria real. Se sustituyó por
/// direcciones reales verificadas contra el desensamblado.
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

    /// Registros/memoria de entrada esperados
    pub inputs: &'static str,

    /// Registros/memoria de salida o modificados
    pub outputs: &'static str,

    /// Registros preservados
    pub preserved: &'static str,

    /// `true` si la dirección y la convención de llamada se verificaron
    /// leyendo el código real del desensamblado (no solo la etiqueta).
    /// `false` significa que solo se localizó una dirección plausible
    /// (p.ej. el dispatch de un comando BASIC) sin confirmar que sea segura
    /// de llamar directamente desde código generado — usar con precaución.
    pub verified: bool,
}

impl RomRoutines {
    /// Crear nueva tabla de rutinas de ROM
    pub fn new() -> Self {
        let mut routines = HashMap::new();

        // ===== ARITMÉTICA BCD (verificado) =====
        //
        // Las cuatro operan directamente sobre ARX ($7A00, 8 bytes) y ARY
        // ($7A10, 8 bytes) en memoria — no esperan punteros en X/Y, solo
        // que los operandos ya estén escritos ahí. El resultado siempre
        // sustituye a ARX. Convención de error común a las cuatro:
        // Carry=1 + UH=código de error si falla, Carry=0 + UH=0 si OK
        // ($25 = overflow, $26 = división por cero). Como toda rutina de
        // ROM, requieren S (stack pointer hardware) inicializado antes de
        // llamarlas (usan SJP/PSH/POP internamente).

        routines.insert("ADDIT", RomRoutine {
            name: "ADDIT",
            address: 0xEFBA,
            description: "Suma BCD de punto flotante: ARX = ARX + ARY",
            inputs: "ARX ($7A00) y ARY ($7A10) con los operandos en BCD; S inicializado",
            outputs: "ARX = resultado; Carry+UH=código de error si falla ($25=overflow)",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("SUBTR", RomRoutine {
            name: "SUBTR",
            address: 0xEFB6,
            description: "Resta BCD de punto flotante: ARX = ARX - ARY (invierte signo de ARY y cae en ADDIT)",
            inputs: "ARX ($7A00) y ARY ($7A10) con los operandos en BCD; S inicializado",
            outputs: "ARX = resultado; Carry+UH=código de error si falla ($25=overflow)",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("MULTIPLY", RomRoutine {
            name: "MULTIPLY",
            address: 0xF01A,
            description: "Multiplicación BCD: ARX = ARX * ARY",
            inputs: "ARX ($7A00) y ARY ($7A10) con los operandos en BCD; S inicializado",
            outputs: "ARX = resultado; Carry+UH=código de error si falla ($25=overflow)",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("DIVISION", RomRoutine {
            name: "DIVISION",
            address: 0xF084,
            description: "División BCD: ARX = ARX / ARY",
            inputs: "ARX ($7A00) y ARY ($7A10) con los operandos en BCD; S inicializado",
            outputs: "ARX = resultado; Carry+UH=código de error si falla ($26=división por cero)",
            preserved: "Ninguno documentado",
            verified: true,
        });

        // ===== SALIDA DE TEXTO (verificado, con avisos) =====

        routines.insert("CHAR_OUT", RomRoutine {
            name: "CHAR_OUT",
            address: 0xED4D,
            description: "Imprime un carácter en pantalla en la posición del cursor",
            inputs: "A = código ASCII; CURSOR_PTR ($7875) debe estar en 0..=0x9C; KATAFLAGS ($785D)=0 recomendado; S inicializado",
            outputs: "Avanza CURSOR_PTR; ejecuta SIE (habilita interrupciones) incondicionalmente al final",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("STR_2_OUTBUF", RomRoutine {
            name: "STR_2_OUTBUF",
            address: 0xEC5C,
            description: "Copia una cadena a OUT_BUF ($7B60), no directamente a pantalla",
            inputs: "X = puntero a la cadena, UL = longitud; puntero de escritura ($788F) debe valer 0x60 en un buffer limpio",
            outputs: "Texto anexado en OUT_BUF; Carry=1 si error (buffer lleno, tope $7BB0)",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("BCD_2_ASCII_OUTBUF", RomRoutine {
            name: "BCD_2_ASCII_OUTBUF",
            address: 0xEC74,
            description: "Convierte ARX (BCD) a ASCII decimal y lo anexa a OUT_BUF ($7B60), no directamente a pantalla",
            inputs: "ARX ($7A00) con el valor; bloque USING ($7895-7898)=0 para formato decimal simple; puntero OUT_BUF ($788F)=0x60 en buffer limpio",
            outputs: "Texto anexado en OUT_BUF, puntero $788F avanzado",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("ARX_2_STRNG", RomRoutine {
            name: "ARX_2_STRNG",
            address: 0xEF1B,
            description: "Convierte ARX (número) a un descriptor de cadena BASIC (CSI) — es la primitiva real detrás de STR$",
            inputs: "ARX ($7A00) con el valor numérico",
            outputs: "Descriptor de cadena (CSI) vía CREATE_CSI_4 ($DEAF)",
            preserved: "Ninguno documentado",
            verified: true,
        });

        // ===== ENTRADA DE TECLADO (verificado) =====
        //
        // Estas dos son puras: solo tocan los puertos de E/S hardware
        // (PC1500_PRT_A/PC1500_PRT_A_DIR, $F00E/$F00C) y una tabla en ROM.
        // No dependen de ninguna variable de sistema en RAM, así que son
        // seguras de llamar incluso desde un arranque en frío.

        routines.insert("ISKEY", RomRoutine {
            name: "ISKEY",
            address: 0xE418,
            description: "Comprueba si hay una tecla pulsada, sin bloquear",
            inputs: "Ninguno",
            outputs: "Z=1 si no hay tecla pulsada",
            preserved: "No depende de RAM de sistema",
            verified: true,
        });

        routines.insert("KEY_2_ASCII", RomRoutine {
            name: "KEY_2_ASCII",
            address: 0xE42C,
            description: "Lee el código ASCII de la tecla pulsada",
            inputs: "Ninguno",
            outputs: "A = código ASCII; Carry=1 si no hay tecla pulsada",
            preserved: "No depende de RAM de sistema",
            verified: true,
        });

        // ===== GRÁFICOS / PANTALLA (verificado) =====
        //
        // Direcciones confirmadas decodificando las tablas de vectores
        // reales de la ROM (`CALL_VECTORS`, $FF00, indexada por los
        // opcodes VEJ/VMJ) en vez de fiarse de los comentarios del
        // desensamblado — varios de esos comentarios están corruptos/mal
        // transcritos. Cruzado con las direcciones ya verificadas
        // (VEJ(F0)->ADDIT, VMJ($7E)->MULTIPLY) como comprobación de que la
        // decodificación es correcta.

        routines.insert("LCD_CLR", RomRoutine {
            name: "LCD_CLR",
            address: 0xEE71,
            description: "CLS real: pone a cero 77 bytes en $7600-$764C y 77 en $7700-$774C (buffer de pantalla). NO toca $764E/$764F/$774E/$774F (símbolos de estado)",
            inputs: "Ninguno",
            outputs: "Buffer de pantalla a cero; clobbers A, U-Reg",
            preserved: "$764E/$764F/$774E/$774F (símbolos), S",
            verified: true,
        });

        routines.insert("INIT_CURS", RomRoutine {
            name: "INIT_CURS",
            address: 0xECAE,
            description: "Limpia el cursor de texto: ANI (CURSOR_ENA),$FE ; ANI (CURSOR_PTR),$00 ; RTN — la propia CLS lo llama tras LCD_CLR",
            inputs: "Ninguno",
            outputs: "CURSOR_ENA bit0=0, CURSOR_PTR=0",
            preserved: "Todo lo demás (operaciones ANI (mem),imm directas a memoria, no pasan por A)",
            verified: true,
        });

        routines.insert("CLR_NO_CURSOR", RomRoutine {
            name: "CLR_NO_CURSOR",
            address: 0xEC9C,
            description: "\"Clears LCD if cursor is not allowed and sets matrix pointer to 00\" — si CURSOR_ENA bit0=0, llama a LCD_CLR y resetea CURSOR_PTR a 0; si bit0=1 (un CURSOR n lo acaba de posicionar), no hace nada. Es lo que llaman de verdad BCMD_PRINT/BCMD_PAUSE (vía su código compartido) antes de imprimir, NO el INIT_CURS incondicional — así `CURSOR n:PRINT ...`/`CURSOR n:PAUSE ...` preservan la posición y el contenido de pantalla ya dibujado (p.ej. un GPRINT anterior), y un PRINT/PAUSE sin CURSOR previo sigue limpiando como siempre",
            inputs: "CURSOR_ENA bit0",
            outputs: "Si bit0=0: pantalla a 0, CURSOR_PTR=0. Si bit0=1: sin efecto",
            preserved: "CURSOR_ENA (no lo modifica)",
            verified: true,
        });

        routines.insert("INIT_MTRX", RomRoutine {
            name: "INIT_MTRX",
            address: 0xECB2,
            description: "Resetea solo CURSOR_PTR a 0 (ANI (CURSOR_PTR),$00 ; RTN), sin tocar CURSOR_ENA — la segunda mitad de INIT_CURS, y el mismo destino al que salta CHAR_OUT cuando el Carry indica que imprimir el último carácter desbordó el ancho de la pantalla (BCS INIT_MTRX). Ver el comentario de StackInstruction::Newline: imprimir 0x0D vía CHAR_OUT NO resetea el cursor por sí mismo (0x0D solo se dibuja como un carácter más y avanza CURSOR_PTR en 6, igual que cualquier otro) — para saltar de verdad a la columna 0 hay que llamar aquí directamente",
            inputs: "Ninguno",
            outputs: "CURSOR_PTR=0",
            preserved: "CURSOR_ENA y todo lo demás",
            verified: true,
        });

        routines.insert("GPRINT_OUT", RomRoutine {
            name: "GPRINT_OUT",
            address: 0xEDEF,
            description: "Escribe un patrón de puntos (1 byte, bit b = fila b de la LCD) en la columna actual de CURSOR_PTR, con lectura-modificación-escritura correcta del buffer nibble-empaquetado. No avanza el cursor (ver MTRX_INC)",
            inputs: "A = patrón de 8 bits (bits 0-6 usados); CURSOR_PTR ($7875) = columna 0..=0x9C",
            outputs: "Buffer de pantalla actualizado en esa columna",
            preserved: "CURSOR_PTR (no lo modifica); clobbers A, UH, X",
            verified: true,
        });

        routines.insert("MTRX_INC", RomRoutine {
            name: "MTRX_INC",
            address: 0xEDB1,
            description: "Incrementa CURSOR_PTR en 1 (saturado en 0x9C). Llamar tras GPRINT_OUT para emular la salida multi-byte de GPRINT",
            inputs: "CURSOR_PTR ($7875)",
            outputs: "CURSOR_PTR incrementado; Carry=1 si ya estaba fuera de rango",
            preserved: "Ninguno documentado",
            verified: true,
        });

        routines.insert("MTRX_IN_RANGE", RomRoutine {
            name: "MTRX_IN_RANGE",
            address: 0xEDAB,
            description: "Comprueba límites de CURSOR_PTR (CPI A,$9C sobre CURSOR_PTR)",
            inputs: "CURSOR_PTR ($7875)",
            outputs: "Carry=1 si CURSOR_PTR >= 156 (fuera de rango)",
            preserved: "Todo",
            verified: true,
        });

        // ===== SONIDO / TEMPORIZACIÓN (verificado) =====

        routines.insert("BEEP", RomRoutine {
            name: "BEEP",
            address: 0xE66F,
            description: "Genera un tono cuadrado real en el zumbador, alternando el bit6 ($40) de PC1500_PRT_C ($F008, registro OPC del LH5810) entre los patrones $C8/$88",
            inputs: "UL = tono (cuenta de semiperiodo interno); X-Reg = duración/repetición",
            outputs: "Sonido emitido; puede terminar antes si se detecta la tecla BREAK (sondea PC1500_IF_REG, $F00B)",
            preserved: "Y, X, U (PSH al entrar, POP antes de RTN)",
            verified: true,
        });

        routines.insert("TIME_DELAY", RomRoutine {
            name: "TIME_DELAY",
            address: 0xE88C,
            description: "Espera U-Reg * 15.625 ms (1/64 s, periodo de la señal cuadrada del reloj vía PC1500_PRT_B/PC1500_PRT_B_DIR, $F00D/$F00F) — interrumpible con BREAK",
            inputs: "U-Reg = número de ciclos de espera",
            outputs: "Ninguna, solo consume tiempo",
            preserved: "Y (PSH/POP interno); clobbers U-Reg",
            verified: true,
        });

        // ===== VARIABLES / ALEATORIOS (verificado) =====

        routines.insert("DEL_STD_VARS", RomRoutine {
            name: "DEL_STD_VARS",
            address: 0xD080,
            description: "CLEAR real: borra el área de variables fijas de la ROM ($7650-$76FF/$77xx) y las variables array (equivalente a 'sin arrays'). Nota: esto es la tabla de variables del INTÉRPRETE de la ROM, no la nuestra — nuestras variables viven en DATA_BASE ($5000+), así que esta rutina no las toca",
            inputs: "Ninguno",
            outputs: "Ninguna relevante para código nativo (limpia memoria que no usamos)",
            preserved: "Clobbers X-Reg, U-Reg, A (según cabecera original de la ROM)",
            verified: true,
        });

        routines.insert("RAND_GEN", RomRoutine {
            name: "RAND_GEN",
            address: 0xF5EB,
            description: "NO confirmada como punto de entrada general para RND(n): es la subrutina reentrante del camino 'n=0/dígito crudo' de BCMD_RND ($F5DD), no el escalado por n. Llamarla directamente con ARX=n (n>0) causó escrituras a memoria no mapeada en pruebas reales — necesita más investigación (la ruta con escalado pasa por más subrutinas sin documentar: $F707, $F715, $F6B4, $F661, $F88F) antes de usarse",
            inputs: "Sin confirmar para el caso general (n>0)",
            outputs: "Sin confirmar para el caso general (n>0)",
            preserved: "Sin confirmar",
            verified: false,
        });

        // No se encontró ninguna rutina aislada de "imprimir salto de
        // línea"; probablemente sea CHAR_OUT con el byte 0x0D (CR).
        // No se encontró "LINE" como palabra clave BASIC real en la tabla
        // de tokens de la ROM — revisar si el parser asume una sintaxis
        // que no corresponde a la Sharp PC-1500 real.
        // No se encontró ninguna primitiva de bajo nivel para VAL
        // (ASCII -> número): la única dirección candidata anterior
        // ($D9D7) resultó ser BCMD_LEN, no VAL. Necesitará un parser
        // manual (Fase 5).
        // No se encontró ninguna rutina aislada de "leer línea completa"
        // reutilizable fuera del bucle del intérprete para INPUT.

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
            ("Aritmética BCD", vec!["ADDIT", "SUBTR", "MULTIPLY", "DIVISION"]),
            ("Salida de texto", vec!["CHAR_OUT", "STR_2_OUTBUF", "BCD_2_ASCII_OUTBUF", "ARX_2_STRNG"]),
            ("Entrada de teclado", vec!["ISKEY", "KEY_2_ASCII"]),
            ("Gráficos / pantalla", vec!["LCD_CLR", "INIT_CURS", "INIT_MTRX", "CLR_NO_CURSOR", "GPRINT_OUT", "MTRX_INC", "MTRX_IN_RANGE"]),
            ("Sonido / temporización", vec!["BEEP", "TIME_DELAY"]),
            ("Variables / aleatorios", vec!["DEL_STD_VARS", "RAND_GEN"]),
        ];

        for (category_name, routine_names) in categories {
            docs.push_str(&format!("## {}\n\n", category_name));

            for name in routine_names {
                if let Some(routine) = self.routines.get(name) {
                    docs.push_str(&format!(
                        "### {} (0x{:04X}){}\n\n",
                        routine.name,
                        routine.address,
                        if routine.verified { "" } else { " ⚠️ sin verificar" }
                    ));
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

        let char_out = rom.get("CHAR_OUT").unwrap();
        assert_eq!(char_out.address, 0xED4D);
        assert!(char_out.verified);
    }

    #[test]
    fn test_address_lookup() {
        let rom = RomRoutines::new();

        assert_eq!(rom.address("ADDIT"), Some(0xEFBA));
        assert_eq!(rom.address("ISKEY"), Some(0xE418));
        assert_eq!(rom.address("DIVISION"), Some(0xF084));
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

    #[test]
    fn test_unverified_routines_are_flagged() {
        // RAND_GEN quedó marcada sin verificar tras confirmar
        // empíricamente que $F5EB no es el punto de entrada general para
        // RND(n>0) (ver comentario de la entrada) — el resto sí se
        // confirmó leyendo el desensamblado real.
        let rom = RomRoutines::new();
        assert!(!rom.get("RAND_GEN").unwrap().verified);
        assert!(rom.get("ADDIT").unwrap().verified);
        assert!(rom.get("LCD_CLR").unwrap().verified);
    }
}
