//! Input injection implementation

#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::Result;

/// Lock key states for synchronization
#[derive(Debug, Clone, Copy, Default)]
pub struct LockStates {
    pub caps_lock: bool,
    pub num_lock: bool,
    pub scroll_lock: bool,
}

#[cfg(windows)]
mod windows_input {
    use super::*;
    use anyhow::Result;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    // Virtual key codes for lock keys
    const VK_CAPITAL: u16 = 0x14; // CapsLock
    const VK_NUMLOCK: u16 = 0x90;
    const VK_SCROLL: u16 = 0x91;

    pub struct InputInjector {
        screen_width: i32,
        screen_height: i32,
        last_mouse_x: i32,
        last_mouse_y: i32,
    }

    impl InputInjector {
        pub fn new() -> Self {
            let (w, h) = unsafe {
                (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
            };
            Self {
                screen_width: w,
                screen_height: h,
                last_mouse_x: 0,
                last_mouse_y: 0,
            }
        }

        /// Get current lock key states
        pub fn get_lock_states(&self) -> LockStates {
            unsafe {
                LockStates {
                    caps_lock: (GetKeyState(VK_CAPITAL as i32) & 1) != 0,
                    num_lock: (GetKeyState(VK_NUMLOCK as i32) & 1) != 0,
                    scroll_lock: (GetKeyState(VK_SCROLL as i32) & 1) != 0,
                }
            }
        }

        /// Synchronize lock states with remote
        pub fn sync_lock_states(&self, remote_states: LockStates) -> Result<()> {
            let local = self.get_lock_states();

            // Toggle CapsLock if different
            if local.caps_lock != remote_states.caps_lock {
                self.toggle_lock_key(VK_CAPITAL)?;
            }

            // Toggle NumLock if different
            if local.num_lock != remote_states.num_lock {
                self.toggle_lock_key(VK_NUMLOCK)?;
            }

            // Toggle ScrollLock if different
            if local.scroll_lock != remote_states.scroll_lock {
                self.toggle_lock_key(VK_SCROLL)?;
            }

            Ok(())
        }

        /// Toggle a lock key by simulating press + release
        fn toggle_lock_key(&self, vk: u16) -> Result<()> {
            self.key_event(vk, true)?;
            self.key_event(vk, false)?;
            Ok(())
        }

        pub fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
            let dx = (x - self.last_mouse_x).abs();
            let dy = (y - self.last_mouse_y).abs();
            if dx < 2 && dy < 2 {
                // Ignore tiny movements to reduce traffic
                return Ok(());
            }

            self.last_mouse_x = x;
            self.last_mouse_y = y;

            let norm_x = (x * 65535) / self.screen_width;
            let norm_y = (y * 65535) / self.screen_height;

            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: norm_x,
                        dy: norm_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            unsafe {
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            Ok(())
        }

        pub fn mouse_button(&mut self, button: u8, pressed: bool, x: i32, y: i32) -> Result<()> {
            // Force move to position before button action
            self.last_mouse_x = x;
            self.last_mouse_y = y;

            let norm_x = (x * 65535) / self.screen_width;
            let norm_y = (y * 65535) / self.screen_height;

            let flags = match (button, pressed) {
                (0, true) => MOUSEEVENTF_LEFTDOWN,
                (0, false) => MOUSEEVENTF_LEFTUP,
                (1, true) => MOUSEEVENTF_MIDDLEDOWN,  // Fixed: button 1 = middle in web
                (1, false) => MOUSEEVENTF_MIDDLEUP,
                (2, true) => MOUSEEVENTF_RIGHTDOWN,
                (2, false) => MOUSEEVENTF_RIGHTUP,
                (3, true) => MOUSEEVENTF_XDOWN,
                (3, false) => MOUSEEVENTF_XUP,
                (4, true) => MOUSEEVENTF_XDOWN,
                (4, false) => MOUSEEVENTF_XUP,
                _ => return Ok(()),
            };

            // Mouse data for X buttons (back=1, forward=2)
            let mouse_data = match button {
                3 => 1u32, // XBUTTON1 (back)
                4 => 2u32, // XBUTTON2 (forward)
                _ => 0u32,
            };

            // Combine move and click in one call for accuracy
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: norm_x,
                        dy: norm_y,
                        mouseData: mouse_data,
                        dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            unsafe {
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            Ok(())
        }

        pub fn mouse_scroll(&self, dx: i32, dy: i32) -> Result<()> {
            // Vertical scroll
            if dy != 0 {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: (dy * WHEEL_DELTA as i32) as u32,
                            dwFlags: MOUSEEVENTF_WHEEL,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                unsafe {
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }

            // Horizontal scroll
            if dx != 0 {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: (dx * WHEEL_DELTA as i32) as u32,
                            dwFlags: MOUSEEVENTF_HWHEEL,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                unsafe {
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }

            Ok(())
        }

        pub fn key_event(&self, key_code: u16, pressed: bool) -> Result<()> {
            let flags = if pressed {
                KEYBD_EVENT_FLAGS(0)
            } else {
                KEYEVENTF_KEYUP
            };

            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(key_code),
                        wScan: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            unsafe {
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            Ok(())
        }

        /// Send key event using scancode (better for international keyboards)
        pub fn key_event_scancode(&self, scan_code: u16, pressed: bool, extended: bool) -> Result<()> {
            let mut flags = KEYEVENTF_SCANCODE;
            if !pressed {
                flags |= KEYEVENTF_KEYUP;
            }
            if extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }

            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan_code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            unsafe {
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            Ok(())
        }

        /// Type a Unicode character directly
        pub fn type_char(&self, c: char) -> Result<()> {
            let code = c as u16;

            // Key down
            let input_down = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code,
                        dwFlags: KEYEVENTF_UNICODE,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            // Key up
            let input_up = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            unsafe {
                SendInput(&[input_down, input_up], std::mem::size_of::<INPUT>() as i32);
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
pub use windows_input::InputInjector;

#[cfg(target_os = "macos")]
mod macos_input {
    use super::*;
    use anyhow::Result;
    use core_graphics::display::{CGDisplay, CGMainDisplayID};
    use core_graphics::event::{
        CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
    };
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    pub struct InputInjector {
        screen_width: i32,
        screen_height: i32,
        last_mouse_x: i32,
        last_mouse_y: i32,
        event_source: CGEventSource,
    }

    impl InputInjector {
        pub fn new() -> Self {
            let display_id = unsafe { CGMainDisplayID() };
            let display = CGDisplay::new(display_id);
            let w = display.pixels_wide() as i32;
            let h = display.pixels_high() as i32;

            let event_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .expect("Failed to create event source");

            Self {
                screen_width: w,
                screen_height: h,
                last_mouse_x: 0,
                last_mouse_y: 0,
                event_source,
            }
        }

        pub fn get_lock_states(&self) -> LockStates {
            // macOS doesn't have NumLock/ScrollLock in the same way
            // CapsLock state can be detected but requires IOKit
            LockStates::default()
        }

        pub fn sync_lock_states(&self, _remote_states: LockStates) -> Result<()> {
            // Not implemented for macOS
            Ok(())
        }

        pub fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
            let dx = (x - self.last_mouse_x).abs();
            let dy = (y - self.last_mouse_y).abs();
            if dx < 2 && dy < 2 {
                return Ok(());
            }

            self.last_mouse_x = x;
            self.last_mouse_y = y;

            let point = core_graphics::geometry::CGPoint::new(x as f64, y as f64);

            if let Ok(event) = CGEvent::new_mouse_event(
                self.event_source.clone(),
                CGEventType::MouseMoved,
                point,
                CGMouseButton::Left,
            ) {
                event.post(CGEventTapLocation::HID);
            }

            Ok(())
        }

        pub fn mouse_button(&mut self, button: u8, pressed: bool, x: i32, y: i32) -> Result<()> {
            self.last_mouse_x = x;
            self.last_mouse_y = y;

            let point = core_graphics::geometry::CGPoint::new(x as f64, y as f64);

            let (event_type, mouse_button) = match (button, pressed) {
                (0, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
                (0, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
                (1, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
                (1, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
                (2, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
                (2, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
                _ => return Ok(()),
            };

            if let Ok(event) = CGEvent::new_mouse_event(
                self.event_source.clone(),
                event_type,
                point,
                mouse_button,
            ) {
                event.post(CGEventTapLocation::HID);
            }

            Ok(())
        }

        pub fn mouse_scroll(&self, dx: i32, dy: i32) -> Result<()> {
            if let Ok(event) = CGEvent::new_scroll_event(
                self.event_source.clone(),
                ScrollEventUnit::PIXEL,
                2, // wheel_count
                dy,
                dx,
                0,
            ) {
                event.post(CGEventTapLocation::HID);
            }
            Ok(())
        }

        pub fn key_event(&self, key_code: u16, pressed: bool) -> Result<()> {
            // Convert Windows virtual key to macOS key code
            let mac_keycode = self.windows_vk_to_mac(key_code);

            if let Ok(event) = CGEvent::new_keyboard_event(
                self.event_source.clone(),
                mac_keycode,
                pressed,
            ) {
                event.post(CGEventTapLocation::HID);
            }

            Ok(())
        }

        pub fn key_event_scancode(&self, scan_code: u16, pressed: bool, _extended: bool) -> Result<()> {
            // Use scan code directly as macOS keycode (approximate)
            if let Ok(event) = CGEvent::new_keyboard_event(
                self.event_source.clone(),
                scan_code,
                pressed,
            ) {
                event.post(CGEventTapLocation::HID);
            }
            Ok(())
        }

        pub fn type_char(&self, c: char) -> Result<()> {
            // For Unicode input, we use CGEvent's set_string_from_string
            if let Ok(event) = CGEvent::new_keyboard_event(
                self.event_source.clone(),
                0,
                true,
            ) {
                let s = c.to_string();
                event.set_string(&s);
                event.post(CGEventTapLocation::HID);
            }
            Ok(())
        }

        // Convert Windows virtual key codes to macOS key codes
        fn windows_vk_to_mac(&self, vk: u16) -> u16 {
            match vk {
                // Letters A-Z (0x41-0x5A)
                0x41 => 0x00, // A
                0x42 => 0x0B, // B
                0x43 => 0x08, // C
                0x44 => 0x02, // D
                0x45 => 0x0E, // E
                0x46 => 0x03, // F
                0x47 => 0x05, // G
                0x48 => 0x04, // H
                0x49 => 0x22, // I
                0x4A => 0x26, // J
                0x4B => 0x28, // K
                0x4C => 0x25, // L
                0x4D => 0x2E, // M
                0x4E => 0x2D, // N
                0x4F => 0x1F, // O
                0x50 => 0x23, // P
                0x51 => 0x0C, // Q
                0x52 => 0x0F, // R
                0x53 => 0x01, // S
                0x54 => 0x11, // T
                0x55 => 0x20, // U
                0x56 => 0x09, // V
                0x57 => 0x0D, // W
                0x58 => 0x07, // X
                0x59 => 0x10, // Y
                0x5A => 0x06, // Z

                // Numbers 0-9 (0x30-0x39)
                0x30 => 0x1D, // 0
                0x31 => 0x12, // 1
                0x32 => 0x13, // 2
                0x33 => 0x14, // 3
                0x34 => 0x15, // 4
                0x35 => 0x17, // 5
                0x36 => 0x16, // 6
                0x37 => 0x1A, // 7
                0x38 => 0x1C, // 8
                0x39 => 0x19, // 9

                // Function keys
                0x70 => 0x7A, // F1
                0x71 => 0x78, // F2
                0x72 => 0x63, // F3
                0x73 => 0x76, // F4
                0x74 => 0x60, // F5
                0x75 => 0x61, // F6
                0x76 => 0x62, // F7
                0x77 => 0x64, // F8
                0x78 => 0x65, // F9
                0x79 => 0x6D, // F10
                0x7A => 0x67, // F11
                0x7B => 0x6F, // F12

                // Special keys
                0x08 => 0x33, // Backspace
                0x09 => 0x30, // Tab
                0x0D => 0x24, // Enter
                0x10 => 0x38, // Shift
                0x11 => 0x3B, // Control
                0x12 => 0x3A, // Alt/Option
                0x14 => 0x39, // CapsLock
                0x1B => 0x35, // Escape
                0x20 => 0x31, // Space

                // Arrow keys
                0x25 => 0x7B, // Left
                0x26 => 0x7E, // Up
                0x27 => 0x7C, // Right
                0x28 => 0x7D, // Down

                // Other keys
                0x2E => 0x75, // Delete
                0x24 => 0x73, // Home
                0x23 => 0x77, // End
                0x21 => 0x74, // PageUp
                0x22 => 0x79, // PageDown

                // Default: pass through
                _ => vk,
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_input::InputInjector;

#[cfg(target_os = "linux")]
mod linux_input {
    use super::*;
    use anyhow::Result;
    use std::ptr;
    use x11::xlib::*;
    use x11::xtest::*;

    pub struct InputInjector {
        display: *mut Display,
        screen_width: i32,
        screen_height: i32,
        last_mouse_x: i32,
        last_mouse_y: i32,
    }

    // Display pointer is thread-safe for our use case
    unsafe impl Send for InputInjector {}
    unsafe impl Sync for InputInjector {}

    impl InputInjector {
        pub fn new() -> Self {
            unsafe {
                let display = XOpenDisplay(ptr::null());
                if display.is_null() {
                    panic!("Failed to open X11 display for input injection");
                }

                let screen = XDefaultScreen(display);
                let w = XDisplayWidth(display, screen);
                let h = XDisplayHeight(display, screen);

                println!("[INPUT] Linux X11 input ready: {}x{}", w, h);

                Self {
                    display,
                    screen_width: w,
                    screen_height: h,
                    last_mouse_x: 0,
                    last_mouse_y: 0,
                }
            }
        }

        pub fn get_lock_states(&self) -> LockStates {
            unsafe {
                let mut state: XKeyboardState = std::mem::zeroed();
                XGetKeyboardControl(self.display, &mut state);

                LockStates {
                    caps_lock: (state.led_mask & 1) != 0,
                    num_lock: (state.led_mask & 2) != 0,
                    scroll_lock: (state.led_mask & 4) != 0,
                }
            }
        }

        pub fn sync_lock_states(&self, remote_states: LockStates) -> Result<()> {
            let local = self.get_lock_states();

            // Toggle CapsLock if different
            if local.caps_lock != remote_states.caps_lock {
                self.toggle_lock_key(0xFFE5)?; // XK_Caps_Lock
            }

            // Toggle NumLock if different
            if local.num_lock != remote_states.num_lock {
                self.toggle_lock_key(0xFF7F)?; // XK_Num_Lock
            }

            // Toggle ScrollLock if different
            if local.scroll_lock != remote_states.scroll_lock {
                self.toggle_lock_key(0xFF14)?; // XK_Scroll_Lock
            }

            Ok(())
        }

        fn toggle_lock_key(&self, keysym: u32) -> Result<()> {
            unsafe {
                let keycode = XKeysymToKeycode(self.display, keysym as u64);
                if keycode != 0 {
                    XTestFakeKeyEvent(self.display, keycode as u32, 1, 0); // Press
                    XTestFakeKeyEvent(self.display, keycode as u32, 0, 0); // Release
                    XFlush(self.display);
                }
            }
            Ok(())
        }

        pub fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
            let dx = (x - self.last_mouse_x).abs();
            let dy = (y - self.last_mouse_y).abs();
            if dx < 2 && dy < 2 {
                return Ok(());
            }

            self.last_mouse_x = x;
            self.last_mouse_y = y;

            unsafe {
                XTestFakeMotionEvent(self.display, -1, x, y, 0);
                XFlush(self.display);
            }
            Ok(())
        }

        pub fn mouse_button(&mut self, button: u8, pressed: bool, x: i32, y: i32) -> Result<()> {
            self.last_mouse_x = x;
            self.last_mouse_y = y;

            // First move to position
            unsafe {
                XTestFakeMotionEvent(self.display, -1, x, y, 0);
            }

            // X11 button mapping:
            // 1 = left, 2 = middle, 3 = right, 4 = scroll up, 5 = scroll down
            // 8 = back, 9 = forward
            let x_button = match button {
                0 => 1, // Left
                1 => 2, // Middle
                2 => 3, // Right
                3 => 8, // Back
                4 => 9, // Forward
                _ => return Ok(()),
            };

            unsafe {
                XTestFakeButtonEvent(self.display, x_button, if pressed { 1 } else { 0 }, 0);
                XFlush(self.display);
            }
            Ok(())
        }

        pub fn mouse_scroll(&self, dx: i32, dy: i32) -> Result<()> {
            unsafe {
                // Vertical scroll
                if dy != 0 {
                    let button = if dy > 0 { 4 } else { 5 }; // 4=up, 5=down
                    let count = dy.abs();
                    for _ in 0..count {
                        XTestFakeButtonEvent(self.display, button, 1, 0);
                        XTestFakeButtonEvent(self.display, button, 0, 0);
                    }
                }

                // Horizontal scroll
                if dx != 0 {
                    let button = if dx > 0 { 7 } else { 6 }; // 6=left, 7=right
                    let count = dx.abs();
                    for _ in 0..count {
                        XTestFakeButtonEvent(self.display, button, 1, 0);
                        XTestFakeButtonEvent(self.display, button, 0, 0);
                    }
                }

                XFlush(self.display);
            }
            Ok(())
        }

        pub fn key_event(&self, key_code: u16, pressed: bool) -> Result<()> {
            unsafe {
                // Convert Windows VK to X11 keysym, then to keycode
                let keysym = self.windows_vk_to_x11_keysym(key_code);
                let keycode = XKeysymToKeycode(self.display, keysym);

                if keycode != 0 {
                    XTestFakeKeyEvent(self.display, keycode as u32, if pressed { 1 } else { 0 }, 0);
                    XFlush(self.display);
                }
            }
            Ok(())
        }

        pub fn key_event_scancode(&self, scan_code: u16, pressed: bool, _extended: bool) -> Result<()> {
            unsafe {
                // Scan codes are roughly offset by 8 in X11
                let keycode = (scan_code as u32).wrapping_add(8);
                XTestFakeKeyEvent(self.display, keycode, if pressed { 1 } else { 0 }, 0);
                XFlush(self.display);
            }
            Ok(())
        }

        pub fn type_char(&self, c: char) -> Result<()> {
            unsafe {
                // For Unicode input, we need to find the keysym and send it
                let keysym = c as u64;
                let keycode = XKeysymToKeycode(self.display, keysym);

                if keycode != 0 {
                    XTestFakeKeyEvent(self.display, keycode as u32, 1, 0);
                    XTestFakeKeyEvent(self.display, keycode as u32, 0, 0);
                    XFlush(self.display);
                }
            }
            Ok(())
        }

        // Convert Windows virtual key codes to X11 keysyms
        fn windows_vk_to_x11_keysym(&self, vk: u16) -> u64 {
            match vk {
                // Letters A-Z (0x41-0x5A) - lowercase keysyms
                0x41..=0x5A => (vk as u64) + 0x20, // 'a' = 0x61

                // Numbers 0-9 (0x30-0x39)
                0x30..=0x39 => vk as u64,

                // Function keys F1-F12
                0x70 => 0xFFBE, // F1
                0x71 => 0xFFBF, // F2
                0x72 => 0xFFC0, // F3
                0x73 => 0xFFC1, // F4
                0x74 => 0xFFC2, // F5
                0x75 => 0xFFC3, // F6
                0x76 => 0xFFC4, // F7
                0x77 => 0xFFC5, // F8
                0x78 => 0xFFC6, // F9
                0x79 => 0xFFC7, // F10
                0x7A => 0xFFC8, // F11
                0x7B => 0xFFC9, // F12

                // Special keys
                0x08 => 0xFF08, // Backspace
                0x09 => 0xFF09, // Tab
                0x0D => 0xFF0D, // Enter/Return
                0x10 => 0xFFE1, // Shift_L
                0x11 => 0xFFE3, // Control_L
                0x12 => 0xFFE9, // Alt_L
                0x14 => 0xFFE5, // CapsLock
                0x1B => 0xFF1B, // Escape
                0x20 => 0x0020, // Space

                // Arrow keys
                0x25 => 0xFF51, // Left
                0x26 => 0xFF52, // Up
                0x27 => 0xFF53, // Right
                0x28 => 0xFF54, // Down

                // Navigation keys
                0x2E => 0xFFFF, // Delete
                0x2D => 0xFF63, // Insert
                0x24 => 0xFF50, // Home
                0x23 => 0xFF57, // End
                0x21 => 0xFF55, // PageUp
                0x22 => 0xFF56, // PageDown

                // Numpad
                0x60 => 0xFFB0, // Numpad 0
                0x61 => 0xFFB1, // Numpad 1
                0x62 => 0xFFB2, // Numpad 2
                0x63 => 0xFFB3, // Numpad 3
                0x64 => 0xFFB4, // Numpad 4
                0x65 => 0xFFB5, // Numpad 5
                0x66 => 0xFFB6, // Numpad 6
                0x67 => 0xFFB7, // Numpad 7
                0x68 => 0xFFB8, // Numpad 8
                0x69 => 0xFFB9, // Numpad 9
                0x6A => 0xFFAA, // Multiply
                0x6B => 0xFFAB, // Add
                0x6D => 0xFFAD, // Subtract
                0x6E => 0xFFAE, // Decimal
                0x6F => 0xFFAF, // Divide

                // Windows keys
                0x5B => 0xFFEB, // Left Windows/Super
                0x5C => 0xFFEC, // Right Windows/Super
                0x5D => 0xFF67, // Menu

                // Lock keys
                0x90 => 0xFF7F, // NumLock
                0x91 => 0xFF14, // ScrollLock

                // Punctuation
                0xBA => 0x003B, // Semicolon
                0xBB => 0x003D, // Equals
                0xBC => 0x002C, // Comma
                0xBD => 0x002D, // Minus
                0xBE => 0x002E, // Period
                0xBF => 0x002F, // Slash
                0xC0 => 0x0060, // Backtick
                0xDB => 0x005B, // Left bracket
                0xDC => 0x005C, // Backslash
                0xDD => 0x005D, // Right bracket
                0xDE => 0x0027, // Quote

                // Print Screen, Pause
                0x2C => 0xFF61, // PrintScreen
                0x13 => 0xFF13, // Pause

                // Default: pass through as-is
                _ => vk as u64,
            }
        }
    }

    impl Drop for InputInjector {
        fn drop(&mut self) {
            unsafe {
                if !self.display.is_null() {
                    XCloseDisplay(self.display);
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_input::InputInjector;

// Stub for unsupported platforms
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub struct InputInjector;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl InputInjector {
    pub fn new() -> Self {
        Self
    }

    pub fn get_lock_states(&self) -> LockStates {
        LockStates::default()
    }

    pub fn sync_lock_states(&self, _remote_states: LockStates) -> Result<()> {
        Ok(())
    }

    pub fn move_mouse(&mut self, _x: i32, _y: i32) -> Result<()> {
        Ok(())
    }

    pub fn mouse_button(&mut self, _b: u8, _p: bool, _x: i32, _y: i32) -> Result<()> {
        Ok(())
    }

    pub fn mouse_scroll(&self, _dx: i32, _dy: i32) -> Result<()> {
        Ok(())
    }

    pub fn key_event(&self, _k: u16, _p: bool) -> Result<()> {
        Ok(())
    }

    pub fn key_event_scancode(&self, _scan_code: u16, _pressed: bool, _extended: bool) -> Result<()> {
        Ok(())
    }

    pub fn type_char(&self, _c: char) -> Result<()> {
        Ok(())
    }
}
