#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryNumber {
    value: u16,
}

impl BinaryNumber {
    pub fn new(value: u16) -> Self {
        BinaryNumber { value }
    }

    pub fn as_u16(&self) -> u16 {
        self.value
    }

    pub fn write_bytes(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(format!("&{:X}", self.value).as_bytes());
    }
}

impl std::str::FromStr for BinaryNumber {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(hex_str) = s.strip_prefix("&") {
            let value =
                u16::from_str_radix(hex_str, 16).map_err(|_| "Invalid hex number format")?;

            Ok(BinaryNumber::new(value))
        } else {
            Err("Binary number must start with '&'")
        }
    }
}

impl std::fmt::Display for BinaryNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show value in all caps hexadecimal format
        write!(f, "&{:X}", self.value)
    }
}
