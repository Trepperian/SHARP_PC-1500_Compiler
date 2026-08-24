/// Direcciones de variables de sistema de la ROM del Sharp PC-1500.
///
/// Verificadas leyendo el desensamblado real (`PC-1500_ROM-A0x.lh5801.asm`
/// y `lib/PC-1500.lib` del proyecto hermano `PC-1500-Emulator`), no
/// inventadas. Solo tienen sentido asumiendo una PC-1500 **con expansión de
/// RAM** (CE-150/CE-158): en el modelo base sin expansión la RAM termina en
/// `0x47FF` y ninguna de estas direcciones cae en RAM real.
///
/// # Por qué hace falta esto
/// Toda rutina de ROM que llamemos (`ADDIT`, `CHAR_OUT`, `STR_2_OUTBUF`...)
/// usa `SJP`/`PSH`/`POP` internamente, así que el registro `S` (puntero de
/// pila hardware) debe apuntar a RAM válida *antes* de la primera llamada —
/// si no, la primera `SJP`/`PSH` corrompe memoria no mapeada. Además varias
/// rutinas de formateo de texto asumen que ciertas variables de sistema
/// (cursor, formato USING, puntero de OUT_BUF) están en un estado "limpio"
/// que normalmente pone `INIT_SYS_ADDR` ($CFCC) al arrancar BASIC — como
/// generamos código que se carga y ejecuta directamente sin pasar por ese
/// arranque, el prólogo del backend debe replicar ese subconjunto mínimo.

/// Acumulador de coma flotante BCD: primer operando y resultado de
/// `ADDIT`/`SUBTR`/`MULTIPLY`/`DIVISION`. 8 bytes: `$7A00`-`$7A07`.
pub const ARX: u16 = 0x7A00;
pub const ARX_LEN: u16 = 8;

/// Segundo operando de `ADDIT`/`SUBTR`/`MULTIPLY`/`DIVISION`. 8 bytes:
/// `$7A10`-`$7A17`.
pub const ARY: u16 = 0x7A10;
pub const ARY_LEN: u16 = 8;

/// Extremo superior de la pila de la CPU que usa la propia ROM
/// (`CPU_STACK`, `$7800`-`$784F`, 80 bytes). Solo documentado como
/// referencia — el código generado usa su **propia** región de pila más
/// amplia (ver `Lh5801Backend`), no esta, para no competir por espacio con
/// lo que empujen internamente las rutinas de ROM que llamemos.
pub const ROM_CPU_STACK_TOP: u16 = 0x784F;

/// Posición del cursor de texto (columna, `0..=0x9C`). `CHAR_OUT` **no
/// comprueba límites** en su rama final — si esto contiene basura puede
/// escribir fuera del buffer de pantalla (hacia `CPU_STACK`, `$7800`).
pub const CURSOR_PTR: u16 = 0x7875;
pub const CURSOR_ENA: u16 = 0x7874;
/// Selector de tabla de glifos (normal/Katakana). Debe ser 0 para texto
/// normal.
pub const KATAFLAGS: u16 = 0x785D;

/// Bloque de formato `USING` (`USINGF`, `USINGM`, `USING_CHR`, `USINGMD`,
/// 4 bytes consecutivos desde `$7895`). Debe estar a cero para que
/// `ARX_2_ASCII`/`ARX_2_STRNG`/`BCD_2_ASCII_OUTBUF` impriman números en
/// formato decimal simple, sin aplicar un formato `USING` activo.
pub const USING_BLOCK: u16 = 0x7895;
pub const USING_BLOCK_LEN: u16 = 4;

/// Puntero de escritura dentro de `OUT_BUF` (offset dentro de la página
/// `$7Bxx`, es decir `0x60` = inicio de `OUT_BUF`). Debe valer `0x60` antes
/// de usar `STR_2_OUTBUF`/`BCD_2_ASCII_OUTBUF` sobre un buffer limpio.
pub const OUT_BUF_WRITE_PTR: u16 = 0x788F;
pub const OUT_BUF: u16 = 0x7B60;
pub const OUT_BUF_LEN: u16 = 0x50;

/// Buffer de cadenas de la ROM (usado por `STR_2_OUTBUF` y similares como
/// área de trabajo para valores de tipo cadena).
pub const STR_BUF: u16 = 0x7B10;
pub const STR_BUF_LEN: u16 = 0x50;

/// Buffer de entrada de la ROM (líneas leídas por teclado).
pub const IN_BUF: u16 = 0x7BB0;
pub const IN_BUF_LEN: u16 = 0x80;
