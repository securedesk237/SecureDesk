//! Privacy mode implementation (black screen, input blocking)

#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::Result;
#[cfg(not(windows))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
mod windows_privacy {
    use anyhow::Result;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::core::*;
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    static INPUT_BLOCKED: AtomicBool = AtomicBool::new(false);

    pub struct PrivacyMode {
        black_screen: AtomicBool,
        input_blocked: AtomicBool,
        overlay_hwnd: Option<HWND>,
        hook: Option<HHOOK>,
    }

    impl PrivacyMode {
        pub fn new() -> Self {
            Self {
                black_screen: AtomicBool::new(false),
                input_blocked: AtomicBool::new(false),
                overlay_hwnd: None,
                hook: None,
            }
        }

        pub fn enable_black_screen(&mut self) -> Result<()> {
            if self.black_screen.load(Ordering::SeqCst) {
                return Ok(());
            }

            self.black_screen.store(true, Ordering::SeqCst);
            self.overlay_hwnd = Some(unsafe { self.create_overlay()? });
            Ok(())
        }

        pub fn disable_black_screen(&mut self) -> Result<()> {
            if !self.black_screen.load(Ordering::SeqCst) {
                return Ok(());
            }

            self.black_screen.store(false, Ordering::SeqCst);
            if let Some(hwnd) = self.overlay_hwnd.take() {
                unsafe { let _ = DestroyWindow(hwnd); }
            }
            Ok(())
        }

        pub fn block_input(&mut self) -> Result<()> {
            if self.input_blocked.load(Ordering::SeqCst) {
                return Ok(());
            }

            self.input_blocked.store(true, Ordering::SeqCst);
            INPUT_BLOCKED.store(true, Ordering::SeqCst);

            unsafe {
                let module = GetModuleHandleW(None)?;
                let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), module, 0)?;
                self.hook = Some(hook);
            }
            Ok(())
        }

        pub fn unblock_input(&mut self) -> Result<()> {
            if !self.input_blocked.load(Ordering::SeqCst) {
                return Ok(());
            }

            self.input_blocked.store(false, Ordering::SeqCst);
            INPUT_BLOCKED.store(false, Ordering::SeqCst);

            if let Some(hook) = self.hook.take() {
                unsafe { let _ = UnhookWindowsHookEx(hook); }
            }
            Ok(())
        }

        pub fn disable_all(&mut self) -> Result<()> {
            self.disable_black_screen()?;
            self.unblock_input()?;
            Ok(())
        }

        pub fn is_black_screen_active(&self) -> bool {
            self.black_screen.load(Ordering::SeqCst)
        }

        pub fn is_input_blocked(&self) -> bool {
            self.input_blocked.load(Ordering::SeqCst)
        }

        unsafe fn create_overlay(&self) -> Result<HWND> {
            let class = w!("SecureDeskOverlay");
            let module = GetModuleHandleW(None)?;

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(overlay_proc),
                hInstance: module.into(),
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let w = GetSystemMetrics(SM_CXSCREEN);
            let h = GetSystemMetrics(SM_CYSCREEN);

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!("Remote Session"),
                WS_POPUP | WS_VISIBLE,
                0, 0, w, h,
                None, None, module, None,
            );

            if hwnd.0 == 0 {
                anyhow::bail!("Failed to create overlay window");
            }

            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
            Ok(hwnd)
        }
    }

    unsafe extern "system" fn overlay_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let _ = FillRect(hdc, &rect, HBRUSH(GetStockObject(BLACK_BRUSH).0));

                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FFFFFF));
                let text = w!("Remote Support Session Active");
                let _ = DrawTextW(hdc, &mut text.as_wide().to_vec(), &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_CLOSE => LRESULT(0),
            WM_KEYDOWN => {
                // Allow Ctrl+Shift+Esc to exit
                let ctrl = GetAsyncKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetAsyncKeyState(VK_SHIFT.0 as i32) < 0;
                if ctrl && shift && wparam.0 == VK_ESCAPE.0 as usize {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe extern "system" fn kb_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && INPUT_BLOCKED.load(Ordering::SeqCst) {
            // Allow Ctrl+Shift+Esc
            let ctrl = GetAsyncKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetAsyncKeyState(VK_SHIFT.0 as i32) < 0;
            let kb = lparam.0 as *const KBDLLHOOKSTRUCT;
            if !kb.is_null() && ctrl && shift && (*kb).vkCode == VK_ESCAPE.0 as u32 {
                return CallNextHookEx(None, code, wparam, lparam);
            }
            return LRESULT(1); // Block
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

#[cfg(windows)]
pub use windows_privacy::PrivacyMode;

#[cfg(not(windows))]
pub struct PrivacyMode {
    black_screen: AtomicBool,
    input_blocked: AtomicBool,
}

#[cfg(not(windows))]
impl PrivacyMode {
    pub fn new() -> Self {
        Self {
            black_screen: AtomicBool::new(false),
            input_blocked: AtomicBool::new(false),
        }
    }

    pub fn enable_black_screen(&mut self) -> Result<()> {
        self.black_screen.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn disable_black_screen(&mut self) -> Result<()> {
        self.black_screen.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn block_input(&mut self) -> Result<()> {
        self.input_blocked.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn unblock_input(&mut self) -> Result<()> {
        self.input_blocked.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn disable_all(&mut self) -> Result<()> {
        self.disable_black_screen()?;
        self.unblock_input()?;
        Ok(())
    }

    pub fn is_black_screen_active(&self) -> bool {
        self.black_screen.load(Ordering::SeqCst)
    }

    pub fn is_input_blocked(&self) -> bool {
        self.input_blocked.load(Ordering::SeqCst)
    }
}
