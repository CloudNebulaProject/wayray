//! Winit event to protocol message conversion.
//!
//! Converts winit `WindowEvent` variants into `InputMessage` values
//! suitable for sending to the wrsrvd server over the QUIC input stream.

use wayray_protocol::messages::{
    ButtonState, InputMessage, KeyState, KeyboardEvent, PointerAxis, PointerButton, PointerMotion,
};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

/// evdev keycodes for the modifiers synced from `ModifiersChanged`.
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;

/// Whether a physical key is a modifier driven via [`convert_modifiers`]
/// instead of the regular key path (so it is neither sent twice nor left
/// stuck "down" when its release is reported only via `ModifiersChanged`).
fn is_modifier_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
    )
}

/// Convert a winit keyboard event to a protocol `InputMessage`.
///
/// Returns `None` for keys we cannot map (e.g. `Unidentified`) and for
/// modifier keys, which are driven from [`convert_modifiers`] instead.
pub fn convert_keyboard(event: &winit::event::KeyEvent) -> Option<InputMessage> {
    let PhysicalKey::Code(code) = event.physical_key else {
        return None;
    };
    if is_modifier_key(code) {
        return None;
    }

    let keycode = keycode_to_evdev(code)?;
    let state = match event.state {
        ElementState::Pressed => KeyState::Pressed,
        ElementState::Released => KeyState::Released,
    };

    Some(InputMessage::Keyboard(KeyboardEvent {
        keycode,
        state,
        time: 0, // winit doesn't provide timestamps on key events
    }))
}

/// Emit synthetic modifier key press/release events for every modifier whose
/// state changed between `prev` and `now`. Driving modifiers from winit's
/// authoritative `ModifiersChanged` state (rather than per-key events) ensures
/// a release is never lost — which would otherwise leave a modifier such as
/// Shift stuck down on the server, shifting all subsequent input.
pub fn convert_modifiers(prev: ModifiersState, now: ModifiersState) -> Vec<InputMessage> {
    let mut out = Vec::new();
    let mut emit = |was: bool, is: bool, keycode: u32| {
        if was != is {
            out.push(InputMessage::Keyboard(KeyboardEvent {
                keycode,
                state: if is {
                    KeyState::Pressed
                } else {
                    KeyState::Released
                },
                time: 0,
            }));
        }
    };
    emit(prev.shift_key(), now.shift_key(), KEY_LEFTSHIFT);
    emit(prev.control_key(), now.control_key(), KEY_LEFTCTRL);
    emit(prev.alt_key(), now.alt_key(), KEY_LEFTALT);
    emit(prev.super_key(), now.super_key(), KEY_LEFTMETA);
    out
}

/// Convert a winit cursor-moved event to a protocol `InputMessage`.
pub fn convert_cursor_moved(position: winit::dpi::PhysicalPosition<f64>) -> InputMessage {
    InputMessage::PointerMotion(PointerMotion {
        x: position.x,
        y: position.y,
        time: 0,
    })
}

/// Convert a winit mouse button event to a protocol `InputMessage`.
pub fn convert_mouse_button(button: MouseButton, state: ElementState) -> Option<InputMessage> {
    let btn_code = match button {
        MouseButton::Left => 0x110,    // BTN_LEFT
        MouseButton::Right => 0x111,   // BTN_RIGHT
        MouseButton::Middle => 0x112,  // BTN_MIDDLE
        MouseButton::Back => 0x116,    // BTN_BACK
        MouseButton::Forward => 0x115, // BTN_FORWARD
        MouseButton::Other(code) => 0x110 + code as u32,
    };

    let btn_state = match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    };

    Some(InputMessage::PointerButton(PointerButton {
        button: btn_code,
        state: btn_state,
        time: 0,
    }))
}

/// Convert a winit mouse wheel event to protocol `InputMessage`(s).
///
/// Returns up to two messages: one for horizontal axis, one for vertical.
pub fn convert_mouse_wheel(delta: MouseScrollDelta) -> Vec<InputMessage> {
    let mut messages = Vec::new();

    match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            if x != 0.0 {
                messages.push(InputMessage::PointerAxis(PointerAxis {
                    axis: wayray_protocol::messages::Axis::Horizontal,
                    value: x as f64 * 15.0, // approximate line-to-pixel conversion
                    time: 0,
                }));
            }
            if y != 0.0 {
                messages.push(InputMessage::PointerAxis(PointerAxis {
                    axis: wayray_protocol::messages::Axis::Vertical,
                    value: y as f64 * 15.0,
                    time: 0,
                }));
            }
        }
        MouseScrollDelta::PixelDelta(pos) => {
            if pos.x != 0.0 {
                messages.push(InputMessage::PointerAxis(PointerAxis {
                    axis: wayray_protocol::messages::Axis::Horizontal,
                    value: pos.x,
                    time: 0,
                }));
            }
            if pos.y != 0.0 {
                messages.push(InputMessage::PointerAxis(PointerAxis {
                    axis: wayray_protocol::messages::Axis::Vertical,
                    value: pos.y,
                    time: 0,
                }));
            }
        }
    }

    messages
}

/// Map a winit `KeyCode` to a Linux evdev keycode.
///
/// This covers the most common keys. Returns `None` for unmapped keys.
fn keycode_to_evdev(code: KeyCode) -> Option<u32> {
    let evdev = match code {
        KeyCode::Escape => 1,
        KeyCode::Digit1 => 2,
        KeyCode::Digit2 => 3,
        KeyCode::Digit3 => 4,
        KeyCode::Digit4 => 5,
        KeyCode::Digit5 => 6,
        KeyCode::Digit6 => 7,
        KeyCode::Digit7 => 8,
        KeyCode::Digit8 => 9,
        KeyCode::Digit9 => 10,
        KeyCode::Digit0 => 11,
        KeyCode::Minus => 12,
        KeyCode::Equal => 13,
        KeyCode::Backspace => 14,
        KeyCode::Tab => 15,
        KeyCode::KeyQ => 16,
        KeyCode::KeyW => 17,
        KeyCode::KeyE => 18,
        KeyCode::KeyR => 19,
        KeyCode::KeyT => 20,
        KeyCode::KeyY => 21,
        KeyCode::KeyU => 22,
        KeyCode::KeyI => 23,
        KeyCode::KeyO => 24,
        KeyCode::KeyP => 25,
        KeyCode::BracketLeft => 26,
        KeyCode::BracketRight => 27,
        KeyCode::Enter => 28,
        KeyCode::ControlLeft => 29,
        KeyCode::KeyA => 30,
        KeyCode::KeyS => 31,
        KeyCode::KeyD => 32,
        KeyCode::KeyF => 33,
        KeyCode::KeyG => 34,
        KeyCode::KeyH => 35,
        KeyCode::KeyJ => 36,
        KeyCode::KeyK => 37,
        KeyCode::KeyL => 38,
        KeyCode::Semicolon => 39,
        KeyCode::Quote => 40,
        KeyCode::Backquote => 41,
        KeyCode::ShiftLeft => 42,
        KeyCode::Backslash => 43,
        KeyCode::KeyZ => 44,
        KeyCode::KeyX => 45,
        KeyCode::KeyC => 46,
        KeyCode::KeyV => 47,
        KeyCode::KeyB => 48,
        KeyCode::KeyN => 49,
        KeyCode::KeyM => 50,
        KeyCode::Comma => 51,
        KeyCode::Period => 52,
        KeyCode::Slash => 53,
        KeyCode::ShiftRight => 54,
        KeyCode::AltLeft => 56,
        KeyCode::Space => 57,
        KeyCode::CapsLock => 58,
        KeyCode::F1 => 59,
        KeyCode::F2 => 60,
        KeyCode::F3 => 61,
        KeyCode::F4 => 62,
        KeyCode::F5 => 63,
        KeyCode::F6 => 64,
        KeyCode::F7 => 65,
        KeyCode::F8 => 66,
        KeyCode::F9 => 67,
        KeyCode::F10 => 68,
        KeyCode::NumLock => 69,
        KeyCode::ScrollLock => 70,
        KeyCode::F11 => 87,
        KeyCode::F12 => 88,
        KeyCode::ControlRight => 97,
        KeyCode::AltRight => 100,
        KeyCode::Home => 102,
        KeyCode::ArrowUp => 103,
        KeyCode::PageUp => 104,
        KeyCode::ArrowLeft => 105,
        KeyCode::ArrowRight => 106,
        KeyCode::End => 107,
        KeyCode::ArrowDown => 108,
        KeyCode::PageDown => 109,
        KeyCode::Insert => 110,
        KeyCode::Delete => 111,
        KeyCode::SuperLeft => 125,
        KeyCode::SuperRight => 126,
        _ => return None,
    };
    Some(evdev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_a_maps_to_evdev_30() {
        assert_eq!(keycode_to_evdev(KeyCode::KeyA), Some(30));
    }

    #[test]
    fn keycode_escape_maps_to_evdev_1() {
        assert_eq!(keycode_to_evdev(KeyCode::Escape), Some(1));
    }

    #[test]
    fn keycode_enter_maps_to_evdev_28() {
        assert_eq!(keycode_to_evdev(KeyCode::Enter), Some(28));
    }

    #[test]
    fn unmapped_keycode_returns_none() {
        // F13 and beyond are not in our mapping.
        assert!(keycode_to_evdev(KeyCode::F13).is_none());
    }

    #[test]
    fn mouse_button_left() {
        let msg = convert_mouse_button(MouseButton::Left, ElementState::Pressed).unwrap();
        match msg {
            InputMessage::PointerButton(pb) => {
                assert_eq!(pb.button, 0x110);
                assert_eq!(pb.state, ButtonState::Pressed);
            }
            _ => panic!("expected PointerButton"),
        }
    }

    #[test]
    fn cursor_moved_converts() {
        let msg = convert_cursor_moved(winit::dpi::PhysicalPosition::new(100.5, 200.0));
        match msg {
            InputMessage::PointerMotion(pm) => {
                assert!((pm.x - 100.5).abs() < f64::EPSILON);
                assert!((pm.y - 200.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected PointerMotion"),
        }
    }

    #[test]
    fn mouse_wheel_line_delta() {
        let msgs = convert_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0));
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            InputMessage::PointerAxis(pa) => {
                assert_eq!(pa.axis, wayray_protocol::messages::Axis::Vertical);
                assert!((pa.value - 15.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected PointerAxis"),
        }
    }
}
