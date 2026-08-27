//! Formato de archivo binario LH5 para el emulador PC-1500
//! 
//! Este formato contiene código máquina LH5801 puro sin empaquetado adicional.
//! El emulador debe cargarlo directamente en memoria.
//!
//! # Estructura del archivo
//! 
//! ```text
//! Offset | Tamaño | Descripción
//! -------|--------|-------------
//! 0x0000 | 2      | Dirección de carga (little-endian)
//! 0x0002 | 2      | Longitud del código (little-endian)
//! 0x0004 | N      | Código máquina LH5801 (N bytes)
//! ```
//!
//! # Ejemplo de uso en el emulador
//! 
//! ```rust,ignore
//! fn load_lh5_file(&mut self, path: &Path) -> Result<(), Error> {
//!     let data = fs::read(path)?;
//!     
//!     if data.len() < 4 {
//!         return Err(Error::InvalidFormat);
//!     }
//!     
//!     // Leer dirección de carga (little-endian)
//!     let load_address = u16::from_le_bytes([data[0], data[1]]);
//!     
//!     // Leer longitud del código
//!     let code_length = u16::from_le_bytes([data[2], data[3]]);
//!     
//!     // Extraer código máquina
//!     let machine_code = &data[4..];
//!     
//!     if machine_code.len() != code_length as usize {
//!         return Err(Error::LengthMismatch);
//!     }
//!     
//!     // Cargar en memoria
//!     let start = load_address as usize;
//!     let end = start + code_length as usize;
//!     self.memory[start..end].copy_from_slice(machine_code);
//!     
//!     // Opcionalmente ejecutar
//!     self.cpu.pc = load_address;
//!     
//!     Ok(())
//! }
//! ```
//!
//! # Consideraciones
//! 
//! - El código se carga en la dirección especificada (típicamente 0x3800)
//! - El emulador debe verificar que no se sobrescriba memoria del sistema
//! - El rango de memoria válido para usuario es 0x3800-0x5FFF (STANDARD_USER_MEMORY)
//! - La pila implementada por software usa 0x5800-0x5FEF (752 bytes)
//! - El stack pointer está en 0x5FF0-0x5FF1 (2 bytes, little-endian)

use std::io::{self, Write};
use std::fs::File;
use std::path::Path;

/// Encabezado del archivo LH5
#[derive(Debug, Clone)]
pub struct Lh5Header {
    /// Dirección de carga en memoria (típicamente 0x3800)
    pub load_address: u16,
    
    /// Longitud del código máquina en bytes
    pub code_length: u16,
}

impl Lh5Header {
    /// Crear un nuevo encabezado
    pub fn new(load_address: u16, code_length: u16) -> Self {
        Self {
            load_address,
            code_length,
        }
    }
    
    /// Serializar el encabezado a bytes (little-endian)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4);
        bytes.extend_from_slice(&self.load_address.to_le_bytes());
        bytes.extend_from_slice(&self.code_length.to_le_bytes());
        bytes
    }
    
    /// Deserializar desde bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        
        let load_address = u16::from_le_bytes([bytes[0], bytes[1]]);
        let code_length = u16::from_le_bytes([bytes[2], bytes[3]]);
        
        Some(Self {
            load_address,
            code_length,
        })
    }
    
    /// Tamaño del encabezado en bytes
    pub const fn header_size() -> usize {
        4
    }
}

/// Escribir un archivo LH5 completo
pub fn write_lh5_file<P: AsRef<Path>>(
    path: P,
    load_address: u16,
    machine_code: &[u8],
) -> io::Result<()> {
    // El campo de longitud del formato `.lh5` es de 16 bits (ver la
    // estructura del archivo, arriba) — `machine_code.len() as u16` sin
    // comprobar antes desborda en silencio para cualquier programa de
    // 65536 bytes o más (`len() as u16` = `len() % 65536`), escribiendo
    // una cabecera con una longitud MENOR que el contenido real del
    // archivo. `Pc1500::load_lh5_file` (el cargador real del emulador)
    // confía en esa cabecera y copia solo esos bytes a memoria — el resto
    // del programa, aunque está en el archivo, nunca llega a cargarse.
    // Cualquier salto/llamada a una dirección más allá de esa longitud
    // trunca acaba leyendo memoria nunca escrita: exactamente el bug que
    // producía un "Illegal opcode" confuso al ejecutar decathlon.bas
    // (67310 bytes → cabecera con 1774) en vez de un error claro de
    // "programa demasiado grande" en el momento de compilar/escribir.
    if machine_code.len() > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "código máquina demasiado grande para el formato .lh5 ({} bytes, el campo de longitud es de 16 bits, máximo {})",
                machine_code.len(),
                u16::MAX
            ),
        ));
    }

    let header = Lh5Header::new(load_address, machine_code.len() as u16);

    let mut file = File::create(path)?;
    file.write_all(&header.to_bytes())?;
    file.write_all(machine_code)?;

    Ok(())
}

/// Leer un archivo LH5
pub fn read_lh5_file<P: AsRef<Path>>(path: P) -> io::Result<(u16, Vec<u8>)> {
    use std::fs;
    
    let data = fs::read(path)?;
    
    if data.len() < Lh5Header::header_size() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "File too small to contain valid LH5 header",
        ));
    }
    
    let header = Lh5Header::from_bytes(&data).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Invalid LH5 header")
    })?;
    
    let machine_code = data[Lh5Header::header_size()..].to_vec();
    
    if machine_code.len() != header.code_length as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Code length mismatch: header says {} bytes, found {} bytes",
                header.code_length,
                machine_code.len()
            ),
        ));
    }
    
    Ok((header.load_address, machine_code))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_header_serialization() {
        let header = Lh5Header::new(0x3800, 1024);
        let bytes = header.to_bytes();
        
        assert_eq!(bytes.len(), 4);
        assert_eq!(bytes, vec![0x00, 0x38, 0x00, 0x04]); // Little-endian
        
        let deserialized = Lh5Header::from_bytes(&bytes).unwrap();
        assert_eq!(deserialized.load_address, 0x3800);
        assert_eq!(deserialized.code_length, 1024);
    }
    
    #[test]
    fn test_roundtrip() {
        let machine_code = vec![0xB5, 0x58, 0xAE, 0x5F, 0xF0]; // Ejemplo de código
        let temp_file = std::env::temp_dir().join("test.lh5");
        
        write_lh5_file(&temp_file, 0x3800, &machine_code).unwrap();
        let (load_addr, read_code) = read_lh5_file(&temp_file).unwrap();
        
        assert_eq!(load_addr, 0x3800);
        assert_eq!(read_code, machine_code);    
        
        let _ = std::fs::remove_file(&temp_file);
    }
}
