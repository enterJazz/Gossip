use crate::communication::api::parse::{get_u16, get_u8, skip, Error};
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq)]
pub struct Validation {
    /// random number to identify this response and its corresponding notification message
    pub message_id: u16,
    pub well_formed: bool,
}

impl Validation {
    pub fn parse(src: &mut Cursor<&[u8]>, size: u16) -> Result<Validation, Error> {
        let message_id = get_u16(src)?;
        skip(src, 1)?;
        let well_formed = match get_u8(src)? & 0b00000001 {
            0 => false,
            _ => true,
        };
        Ok(Validation {
            message_id,
            well_formed,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_well_formed_parse() {
        let comp: Vec<(u8, bool)> = vec![
            (0b00000001, true),
            (0b00000000, false),
            (0b00101000, false),
            (0b00101001, true),
            (0b11111111, true),
            (0b11111110, false),
        ];
        for (byte, exp) in comp {
            let well_formed = match byte & 0b00000001 {
                0 => false,
                _ => true,
            };
            assert_eq!(well_formed, exp);
        }
    }
}
