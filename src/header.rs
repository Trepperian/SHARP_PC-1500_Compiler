const HEADER_MAGIC_BYTE: u8 = 0x01;
const BASIC_MAGIC_STR: &[u8; 4] = b"@COM";
// TODO: both of these parameters should be irrelevant for BASIC programs, check
const STANDARD_ENTRY_POINT: u16 = 0x044f;
const STANDARD_START_ADDRESS: u16 = 0x00c5;

pub struct Header {
    magic_number: u8,
    file_type: [u8; 4],
    program_name: [u8; 16],
    start_address: [u8; 2],
    capacity: [u8; 2],
    entry_address: [u8; 2],
}

impl Header {
    pub fn new(program_name: &str, length: u16) -> Self {
        let mut name_bytes = [0u8; 16];
        // FIXME: ensure this is ASCII
        let bytes = program_name.as_bytes();
        let name_length = bytes.len().min(16);
        name_bytes[..name_length].copy_from_slice(&bytes[..name_length]);

        Header {
            magic_number: HEADER_MAGIC_BYTE,
            file_type: *BASIC_MAGIC_STR,
            program_name: name_bytes,
            start_address: STANDARD_START_ADDRESS.to_le_bytes(),
            // Page 28 of CE158 manual, this is the number of payload bytes - 1
            capacity: (length - 1).to_le_bytes(),
            entry_address: STANDARD_ENTRY_POINT.to_le_bytes(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut header_bytes = vec![self.magic_number];
        header_bytes.extend_from_slice(&self.file_type);
        header_bytes.extend_from_slice(&self.program_name);
        header_bytes.extend_from_slice(&self.start_address);
        header_bytes.extend_from_slice(&self.capacity);
        header_bytes.extend_from_slice(&self.entry_address);
        header_bytes
    }
}
