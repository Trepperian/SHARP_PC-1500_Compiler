// A fixed point decimal number, in BASIC it has a mantissa of 10 decimal places,
// In BCD, it has a 4-bit packed representation for each decimal digit,
// an i8 exponent, and a sign byte.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecimalNumber {
    double: f64,
}

impl DecimalNumber {
    pub fn new(double: f64) -> Self {
        Self { double }
    }

    pub fn as_f64(&self) -> f64 {
        self.double
    }

    // If the decimal number is a whole number, convert it to an integer
    pub fn as_integer(&self) -> Option<i64> {
        if self.double.fract() == 0.0 {
            Some(self.double as i64)
        } else {
            None
        }
    }

    pub fn write_bytes(&self, bytes: &mut Vec<u8>) {
        if self.double == 0.0 {
            bytes.push(b'0');
            return;
        }

        let mut double_str = self.double.to_string();

        if self.double.abs() < 1.0 {
            // dont add 0 before .
            let trimmed = double_str.trim_start_matches('0');
            double_str = trimmed.to_string();
        }

        // If number is longer in normal format than in scientific len(1000000) > len(1E6)
        // then use scientific notation
        let scientific_str = format!("{:E}", self.double);

        // Compare lengths and use the shorter representation
        if double_str.len() > scientific_str.len() {
            bytes.extend_from_slice(scientific_str.as_bytes());
        } else {
            bytes.extend_from_slice(double_str.as_bytes());
        }
    }
}

impl std::str::FromStr for DecimalNumber {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let double = s.parse::<f64>().map_err(|_| "Invalid decimal number")?;
        Ok(Self::new(double))
    }
}

impl std::fmt::Display for DecimalNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.double == 0.0 {
            write!(f, "0")
        } else {
            let mut double_str = self.double.to_string();

            if self.double.abs() < 1.0 {
                // don't add 0 before .
                let trimmed = double_str.trim_start_matches('0');
                double_str = trimmed.to_string();
            }

            // If number is longer in normal format than in scientific len(1000000) > len(1E6)
            // then use scientific notation
            let scientific_str = format!("{:E}", self.double);
            if double_str.len() > scientific_str.len() {
                write!(f, "{}", scientific_str)
            } else {
                write!(f, "{}", double_str)
            }
        }
    }
}
