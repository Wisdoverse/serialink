use crate::protocol::types::ChecksumType;

const MODBUS_CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_MODBUS);
const SMBUS_CRC: crc::Crc<u8> = crc::Crc::<u8>::new(&crc::CRC_8_SMBUS);

/// Compute the checksum bytes for `data` (the payload, without any existing checksum appended).
/// Returns the checksum as a `Vec<u8>` (1 or 2 bytes depending on algorithm).
pub fn compute(data: &[u8], checksum_type: &ChecksumType) -> Vec<u8> {
    match checksum_type {
        ChecksumType::Crc16Modbus => {
            let crc = MODBUS_CRC.checksum(data);
            vec![(crc & 0xFF) as u8, (crc >> 8) as u8]
        }
        ChecksumType::Crc8 => {
            let crc = SMBUS_CRC.checksum(data);
            vec![crc]
        }
        ChecksumType::Xor => {
            let result = data.iter().fold(0u8, |acc, &b| acc ^ b);
            vec![result]
        }
        ChecksumType::Sum8 => {
            let result = data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
            vec![result]
        }
        ChecksumType::Lrc => {
            let result = data
                .iter()
                .fold(0u8, |acc, &b| acc.wrapping_add(b))
                .wrapping_neg();
            vec![result]
        }
    }
}

/// Validate a framed buffer where the last 1 or 2 bytes are the checksum.
/// Returns `false` for buffers that are too short to contain a checksum.
pub fn validate(data: &[u8], checksum_type: &ChecksumType) -> bool {
    match checksum_type {
        ChecksumType::Crc16Modbus => {
            if data.len() < 3 {
                return false;
            }
            let (payload, tail) = data.split_at(data.len() - 2);
            let expected = MODBUS_CRC.checksum(payload);
            let lo = tail[0];
            let hi = tail[1];
            let actual = u16::from_le_bytes([lo, hi]);
            expected == actual
        }
        ChecksumType::Crc8 => {
            if data.len() < 2 {
                return false;
            }
            let (payload, tail) = data.split_at(data.len() - 1);
            let expected = SMBUS_CRC.checksum(payload);
            expected == tail[0]
        }
        ChecksumType::Xor => {
            if data.len() < 2 {
                return false;
            }
            let (payload, tail) = data.split_at(data.len() - 1);
            let expected = payload.iter().fold(0u8, |acc, &b| acc ^ b);
            expected == tail[0]
        }
        ChecksumType::Sum8 => {
            if data.len() < 2 {
                return false;
            }
            let (payload, tail) = data.split_at(data.len() - 1);
            let expected = payload.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
            expected == tail[0]
        }
        ChecksumType::Lrc => {
            if data.len() < 2 {
                return false;
            }
            let (payload, tail) = data.split_at(data.len() - 1);
            let expected = payload
                .iter()
                .fold(0u8, |acc, &b| acc.wrapping_add(b))
                .wrapping_neg();
            expected == tail[0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::ChecksumType;

    // --- CRC-16 Modbus ---

    #[test]
    fn crc16_modbus_validate_known_frame() {
        // Standard Modbus RTU read-holding-registers request with known CRC
        let frame = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD];
        assert!(validate(&frame, &ChecksumType::Crc16Modbus));
    }

    #[test]
    fn crc16_modbus_compute_known_payload() {
        let payload = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
        assert_eq!(
            compute(&payload, &ChecksumType::Crc16Modbus),
            vec![0xC5, 0xCD]
        );
    }

    #[test]
    fn crc16_modbus_corrupted_byte_returns_false() {
        let mut frame = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD];
        frame[2] = 0xFF; // corrupt a payload byte
        assert!(!validate(&frame, &ChecksumType::Crc16Modbus));
    }

    // --- CRC-8 ---

    #[test]
    fn crc8_validate_known_frame() {
        let payload = [0x01, 0x02, 0x03];
        let cs = compute(&payload, &ChecksumType::Crc8);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        assert!(validate(&frame, &ChecksumType::Crc8));
    }

    #[test]
    fn crc8_corrupted_byte_returns_false() {
        let payload = [0x01, 0x02, 0x03];
        let cs = compute(&payload, &ChecksumType::Crc8);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        frame[1] = 0xFF;
        assert!(!validate(&frame, &ChecksumType::Crc8));
    }

    // --- XOR ---

    #[test]
    fn xor_validate_known_frame() {
        let payload: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];
        let cs = compute(&payload, &ChecksumType::Xor);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        assert!(validate(&frame, &ChecksumType::Xor));
    }

    #[test]
    fn xor_corrupted_byte_returns_false() {
        let payload: [u8; 3] = [0x10, 0x20, 0x30];
        let cs = compute(&payload, &ChecksumType::Xor);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        frame[0] ^= 0x01;
        assert!(!validate(&frame, &ChecksumType::Xor));
    }

    // --- Sum8 ---

    #[test]
    fn sum8_validate_known_frame() {
        let payload: [u8; 3] = [0x10, 0x20, 0x30];
        let cs = compute(&payload, &ChecksumType::Sum8);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        assert!(validate(&frame, &ChecksumType::Sum8));
    }

    #[test]
    fn sum8_corrupted_byte_returns_false() {
        let payload: [u8; 3] = [0x10, 0x20, 0x30];
        let cs = compute(&payload, &ChecksumType::Sum8);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        frame[1] = 0xFF;
        assert!(!validate(&frame, &ChecksumType::Sum8));
    }

    // --- LRC ---

    #[test]
    fn lrc_validate_known_frame() {
        // sum = 0x01 + 0x03 + 0x00 + 0x00 + 0x00 + 0x0A = 0x0E, LRC = (-0x0E) & 0xFF = 0xF2
        let payload: [u8; 6] = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
        let cs = compute(&payload, &ChecksumType::Lrc);
        assert_eq!(cs, vec![0xF2]);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        assert!(validate(&frame, &ChecksumType::Lrc));
    }

    #[test]
    fn lrc_corrupted_byte_returns_false() {
        let payload: [u8; 3] = [0x10, 0x20, 0x30];
        let cs = compute(&payload, &ChecksumType::Lrc);
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&cs);
        frame[0] = 0x00;
        assert!(!validate(&frame, &ChecksumType::Lrc));
    }

    // --- Edge cases ---

    #[test]
    fn empty_data_returns_false_for_all_types() {
        for ct in &[
            ChecksumType::Crc16Modbus,
            ChecksumType::Crc8,
            ChecksumType::Xor,
            ChecksumType::Sum8,
            ChecksumType::Lrc,
        ] {
            assert!(
                !validate(&[], ct),
                "expected false for empty data with {ct:?}"
            );
        }
    }

    #[test]
    fn single_byte_returns_false_for_all_types() {
        for ct in &[
            ChecksumType::Crc16Modbus,
            ChecksumType::Crc8,
            ChecksumType::Xor,
            ChecksumType::Sum8,
            ChecksumType::Lrc,
        ] {
            assert!(
                !validate(&[0x42], ct),
                "expected false for single byte with {ct:?}"
            );
        }
    }
}
