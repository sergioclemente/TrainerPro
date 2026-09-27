//! Wahoo Virtual Bike button characteristic: wire identifiers and reports.
//! Wire format reference: https://github.com/StephenDone/kickr_bluetooth/wiki/KICKR-Virtual-Bike-Service
use crate::ControllerButton;
use ControllerButton::*;

pub const SERVICE_UUID: u128 = 0xa026ee0d_0a7d_4ab3_97fa_f1500f9feb8b;
pub const BUTTONS_CHARACTERISTIC_UUID: u128 = 0xa026e03c_0a7d_4ab3_97fa_f1500f9feb8b;

const PRESSED_FLAG: u8 = 0x80;
const SNAPSHOT_PREFIX: [u8; 2] = [0xff, 0x0f];
const BUTTONS_IN_SNAPSHOT_ORDER: [ControllerButton; 12] = [
    LeftBrake,
    LeftUp,
    LeftDown,
    LeftShiftDown,
    LeftShiftUp,
    LeftSteer,
    RightBrake,
    RightDown,
    RightUp,
    RightShiftDown,
    RightShiftUp,
    RightSteer,
];

/// Event reports carry one button; periodic snapshots carry all 12 states.
/// Both formats use bit 7 for down. A snapshot can recover a missed release.
pub fn decode_button_report(bytes: &[u8]) -> Option<Vec<(ControllerButton, bool)>> {
    match bytes {
        [low, high, state] => {
            let button = match [*low, *high] {
                [0x00, 0x01] => RightUp,
                [0x80, 0x00] => RightDown,
                [0x00, 0x08] => RightSteer,
                [0x02, 0x00] => LeftUp,
                [0x04, 0x00] => LeftDown,
                [0x20, 0x00] => LeftSteer,
                [0x00, 0x04] => RightShiftUp,
                [0x00, 0x02] => RightShiftDown,
                [0x10, 0x00] => LeftShiftUp,
                [0x08, 0x00] => LeftShiftDown,
                [0x40, 0x00] => RightBrake,
                [0x01, 0x00] => LeftBrake,
                _ => return None,
            };
            Some(vec![(button, state & PRESSED_FLAG != 0)])
        }
        [prefix_low, prefix_high, states @ ..]
            if [*prefix_low, *prefix_high] == SNAPSHOT_PREFIX
                && states.len() == BUTTONS_IN_SNAPSHOT_ORDER.len() =>
        {
            Some(
                BUTTONS_IN_SNAPSHOT_ORDER
                    .into_iter()
                    .zip(states.iter().map(|state| state & PRESSED_FLAG != 0))
                    .collect(),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_event_reports_and_ignores_unknown_frames() {
        assert_eq!(
            decode_button_report(&[0x20, 0, 0x85]),
            Some(vec![(LeftSteer, true)])
        );
        assert_eq!(
            decode_button_report(&[0, 8, 5]),
            Some(vec![(RightSteer, false)])
        );
        assert_eq!(decode_button_report(&[0xff, 0x0f, 1]), None);
        assert_eq!(decode_button_report(&[]), None);
    }

    #[test]
    fn snapshot_releases_a_button_after_a_missed_event() {
        let mut edges = crate::controller::ButtonEdges::default();
        edges.update(RightSteer, true);
        let snapshot = [0xff, 0x0f, 2, 7, 5, 3, 2, 0x29, 0, 3, 3, 1, 0, 0x0b];
        let events: Vec<_> = decode_button_report(&snapshot)
            .unwrap()
            .into_iter()
            .filter_map(|(button, pressed)| edges.update(button, pressed))
            .collect();
        assert_eq!(
            events,
            vec![crate::ControllerInputEvent::Button {
                button: RightSteer,
                pressed: false,
            }]
        );
    }
}
