//! FIT CRC-16 (nibble-table algorithm from the FIT SDK "CRC" chapter).

/// The 16-entry nibble lookup table from the FIT SDK CRC chapter.
const CRC_TABLE: [u16; 16] = [
    0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401,
    0xA001, 0x6C00, 0x7800, 0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
];

/// Update `crc` with one byte.
pub fn update(crc: u16, byte: u8) -> u16 {
    // Low nibble first, then high nibble — per the SDK reference code.
    let mut crc = crc;

    let tmp = CRC_TABLE[(crc & 0x0F) as usize];
    crc = (crc >> 4) & 0x0FFF;
    crc = crc ^ tmp ^ CRC_TABLE[(byte & 0x0F) as usize];

    let tmp = CRC_TABLE[(crc & 0x0F) as usize];
    crc = (crc >> 4) & 0x0FFF;
    crc = crc ^ tmp ^ CRC_TABLE[((byte >> 4) & 0x0F) as usize];

    crc
}

/// CRC over a whole buffer, starting from 0.
pub fn checksum(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |c, b| update(c, *b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_is_zero() {
        assert_eq!(checksum(&[]), 0);
    }

    #[test]
    fn single_bytes() {
        // Feeding a byte into CRC 0 must equal the combination of both
        // nibble steps; spot-check a few knowns derived from the algorithm.
        assert_eq!(checksum(&[0x00]), 0x0000);
        // CRC-16/ARC of "123456789" is the classic check value 0xBB3D;
        // the FIT nibble algorithm is exactly CRC-16/ARC (poly 0x8005,
        // reflected, init 0).
        assert_eq!(checksum(b"123456789"), 0xBB3D);
    }

    #[test]
    fn appending_crc_le_yields_zero() {
        // Property used by FIT validators: CRC over (data ++ crc_le) == 0.
        let data = b"TrainerPro FIT crc self-check";
        let crc = checksum(data);
        let mut buf = data.to_vec();
        buf.extend_from_slice(&crc.to_le_bytes());
        assert_eq!(checksum(&buf), 0);
    }

    #[test]
    fn update_is_incremental() {
        let data = b"abcdef";
        let whole = checksum(data);
        let mut c = checksum(&data[..3]);
        for b in &data[3..] {
            c = update(c, *b);
        }
        assert_eq!(c, whole);
    }
}
