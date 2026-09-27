//! Zwift Ride advertisements, GATT identifiers, handshake, and button reports.
//! Wire format reference: https://www.makinolo.com/blog/2024/07/26/zwift-ride-protocol/
use crate::controller::ControllerButton;
use ControllerButton::*;

pub const LEGACY_SERVICE_UUID: u128 = 0x00000001_19ca_4651_86e5_fa29dcdd09d1;
pub const CURRENT_SERVICE_UUID: u16 = 0xfc82;
pub const NOTIFY_CHARACTERISTIC_UUID: u128 = 0x00000002_19ca_4651_86e5_fa29dcdd09d1;
pub const WRITE_CHARACTERISTIC_UUID: u128 = 0x00000003_19ca_4651_86e5_fa29dcdd09d1;
pub const INDICATE_CHARACTERISTIC_UUID: u128 = 0x00000004_19ca_4651_86e5_fa29dcdd09d1;
pub const HANDSHAKE_REQUEST: &[u8] = b"RideOn";
pub const BUTTON_STATUS_OPCODE: u8 = 0x23;
pub const ZWIFT_COMPANY_ID: u16 = 0x094a;
/// Ride advertises both halves; the left half owns the bonded BLE link.
pub const LEFT_DEVICE_TYPE: u8 = 0x08;

pub fn is_left_device_type(device_type: u8) -> bool {
    device_type == LEFT_DEVICE_TYPE
}

pub const BUTTON_MASKS: [(u32, ControllerButton); 16] = [
    (0x0001, DpadLeft),
    (0x0002, DpadUp),
    (0x0004, DpadRight),
    (0x0008, DpadDown),
    (0x0010, A),
    (0x0020, B),
    (0x0040, Y),
    (0x0100, Z),
    (0x0200, LeftShiftUp),
    (0x0400, LeftShiftDown),
    (0x0800, LeftPower),
    (0x1000, LeftOnOff),
    (0x2000, RightShiftUp),
    (0x4000, RightShiftDown),
    (0x10000, RightPower),
    (0x20000, RightOnOff),
];

/// Decode protobuf uint32 field 1, skipping supported unknown wire fields.
/// A missing bitmap is not an all-buttons-pressed report (the bitmap is active-low).
pub fn parse_button_bitmap(bytes: &[u8]) -> Result<Option<u32>, &'static str> {
    const BITMAP_FIELD: u64 = 1;
    const WIRE_BITS: u32 = 3;
    const WIRE_MASK: u64 = 7;
    const FIXED64_BYTES: usize = 8;
    const FIXED32_BYTES: usize = 4;
    let mut input = bytes;
    let mut bitmap = None;
    while !input.is_empty() {
        let key = varint(&mut input)?;
        if key >> WIRE_BITS == 0 {
            return Err("invalid protobuf field");
        }
        let wire = key & WIRE_MASK;
        if key >> WIRE_BITS == BITMAP_FIELD {
            if wire != 0 {
                return Err("invalid bitmap wire type");
            }
            bitmap = Some(u32::try_from(varint(&mut input)?).map_err(|_| "bitmap overflow")?);
            continue;
        }
        let skip = match wire {
            0 => {
                varint(&mut input)?;
                0
            }
            1 => FIXED64_BYTES,
            2 => usize::try_from(varint(&mut input)?).map_err(|_| "length overflow")?,
            5 => FIXED32_BYTES,
            _ => return Err("unsupported protobuf wire type"),
        };
        input = input.get(skip..).ok_or("truncated protobuf field")?;
    }
    Ok(bitmap)
}

fn varint(input: &mut &[u8]) -> Result<u64, &'static str> {
    const DATA_BITS: u32 = 7;
    const DATA_MASK: u8 = 0x7f;
    const MORE: u8 = 0x80;
    let mut value = 0u64;
    for shift in (0..u64::BITS).step_by(DATA_BITS as usize) {
        let (&byte, rest) = input.split_first().ok_or("truncated varint")?;
        *input = rest;
        let data = u64::from(byte & DATA_MASK);
        if data > (u64::MAX >> shift) {
            return Err("varint overflow");
        }
        value |= data << shift;
        if byte & MORE == 0 {
            return Ok(value);
        }
    }
    Err("varint overflow")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ride_bitmap_unknown_fields_and_malformed_packets() {
        assert_eq!(
            parse_button_bitmap(&[8, 0xef, 0xff, 3, 0x1a, 2, 8, 0]),
            Ok(Some(0xffef))
        );
        assert_eq!(parse_button_bitmap(&[0x1a, 0]), Ok(None));
        // Captured Ride packet from the protocol reference: only left is pressed.
        let bitmap = parse_button_bitmap(&[0x08, 0xfe, 0xff, 0xff, 0xff, 0x0f])
            .unwrap()
            .unwrap();
        let pressed: Vec<_> = BUTTON_MASKS
            .iter()
            .filter_map(|(mask, button)| (bitmap & mask == 0).then_some(*button))
            .collect();
        assert_eq!(pressed, vec![DpadLeft]);
        for bytes in [
            &[8, 0x80][..],
            &[0],
            &[0x1a, 8],
            &[0x0a, 0],
            &[8, 0xff, 0xff, 0xff, 0xff, 0x1f],
        ] {
            assert!(parse_button_bitmap(bytes).is_err());
        }
    }

    #[test]
    fn a_button_held_during_connection_requires_a_fresh_press() {
        let bitmap = parse_button_bitmap(&[0x08, 0xbf, 0xff, 0xff, 0xff, 0x0f])
            .unwrap()
            .unwrap();
        let mut edges = crate::controller::ButtonEdges::default();
        for (mask, button) in BUTTON_MASKS {
            edges.update(button, bitmap & mask == 0);
        }
        let release = edges.update(Y, false);
        let press = edges.update(Y, true);
        assert_eq!(
            release,
            Some(crate::ControllerInputEvent::Button {
                button: Y,
                pressed: false,
            })
        );
        assert_eq!(
            press,
            Some(crate::ControllerInputEvent::Button {
                button: Y,
                pressed: true,
            })
        );
    }
}
