//! Native, on-demand settings with standard controls, password masking and tab navigation.
use crate::config::Config;
use std::{
    cell::RefCell,
    mem::zeroed,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Once,
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT},
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};
const SAVE: usize = 500;
const CANCEL: usize = 501;
const TEXT: &[(&str, &str)] = &[
    ("&API key", "api_key"),
    ("&Custom hotkey (e.g. Win+Y)", "hotkey_custom"),
    ("Language (unsupported)", "language"),
    ("Realtime model", "model"),
    ("Batch model", "offline_model"),
    ("Realtime URL", "base_url"),
];
const CHECKS: &[(&str, &str)] = &[
    ("Replace Win+&H", "hotkey_win_h"),
    ("Copilot combinations", "hotkey_copilot"),
    ("&Batch transcription (cloud)", "offline_mode"),
    ("Start with Windows", "start_with_windows"),
];
const MODES: &[(&str, &str)] = &[
    ("Clipboard paste", "paste"),
    ("Unicode keystrokes", "keystrokes"),
];
const SHORTCUTS: &[(&str, &str)] = &[
    ("Shift+Insert", "shift_insert"),
    ("Ctrl+V", "ctrl_v"),
    ("Ctrl+Shift+V (terminals)", "ctrl_shift_v"),
];
thread_local! { static STATE: RefCell<Option<(Config, bool)>> = const { RefCell::new(None) }; }
static REGISTER: Once = Once::new();
static WINDOW: AtomicUsize = AtomicUsize::new(0);
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COMMAND && wp & 0xffff == SAVE {
        let updated = STATE.with(|cell| cell.borrow().as_ref().map(|(cfg, _)| cfg.clone()));
        if let Some(mut cfg) = updated {
            for (i, (_, name)) in TEXT.iter().enumerate() {
                let mut data = [0u16; 4097];
                let len =
                    GetDlgItemTextW(hwnd, i as i32 + 100, data.as_mut_ptr(), data.len() as i32);
                let value = String::from_utf16_lossy(&data[..len as usize]);
                if *name != "language" {
                    let _ = cfg.set_string(name, value.trim());
                }
            }
            for (i, (_, name)) in CHECKS.iter().enumerate() {
                cfg.set_bool(
                    name,
                    SendMessageW(GetDlgItem(hwnd, i as i32 + 200), BM_GETCHECK, 0, 0) == 1,
                );
            }
            for (id, name, options) in [
                (110, "typing_mode", MODES),
                (111, "paste_shortcut", SHORTCUTS),
            ] {
                let index = SendMessageW(GetDlgItem(hwnd, id), CB_GETCURSEL, 0, 0);
                if let Some((_, value)) = options.get(index as usize) {
                    let _ = cfg.set_string(name, value);
                }
            }
            if let Err(e) = cfg.validate() {
                MessageBoxW(
                    hwnd,
                    w(&e.to_string()).as_ptr(),
                    w("Invalid settings").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
                return 0;
            }
            STATE.with(|cell| *cell.borrow_mut() = Some((cfg, true)));
        }
        DestroyWindow(hwnd);
        return 0;
    }
    if msg == WM_CLOSE || msg == WM_COMMAND && wp & 0xffff == CANCEL {
        DestroyWindow(hwnd);
        return 0;
    }
    if msg == WM_DESTROY {
        WINDOW.store(0, Ordering::Relaxed);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn open(parent: HWND, config: &Config) -> Option<Config> {
    unsafe {
        let existing = WINDOW.load(Ordering::Relaxed) as HWND;
        if !existing.is_null() {
            SetForegroundWindow(existing);
            return None;
        }
        let class = w("DictationHotkeyNativeSettings");
        let instance = GetModuleHandleW(null());
        REGISTER.call_once(|| {
            let mut wc: WNDCLASSW = zeroed();
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            wc.lpfnWndProc = Some(proc);
            wc.hbrBackground = (windows_sys::Win32::Graphics::Gdi::COLOR_BTNFACE + 1) as _;
            wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
            RegisterClassW(&wc);
        });
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            w("Dictation Hotkey Settings").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            690,
            610,
            parent,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        STATE.with(|cell| *cell.borrow_mut() = Some((config.clone(), false)));
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |v: i32| (v as i64 * dpi as i64 / 96) as i32;
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            scale(690),
            scale(610),
            SWP_NOMOVE | SWP_NOZORDER,
        );
        let font = GetStockObject(DEFAULT_GUI_FONT);
        let add = |kind: &str, text: &str, x, y, width, height, style, id: usize| {
            let child = CreateWindowExW(
                0,
                w(kind).as_ptr(),
                w(text).as_ptr(),
                WS_VISIBLE | WS_CHILD | style,
                scale(x),
                scale(y),
                scale(width),
                scale(height),
                hwnd,
                id as _,
                instance,
                null(),
            );
            SendMessageW(child, WM_SETFONT, font as usize, 1);
            child
        };
        for (i, &(label, key)) in TEXT.iter().enumerate() {
            let y = 15 + i as i32 * 38;
            add("STATIC", label, 12, y + 4, 230, 24, 0, 0);
            let style = WS_TABSTOP
                | WS_BORDER
                | ES_AUTOHSCROLL as u32
                | if key == "api_key" {
                    ES_PASSWORD as u32
                } else {
                    0
                };
            let edit = add("EDIT", config.string(key), 245, y, 405, 27, style, 100 + i);
            SendMessageW(edit, 0xC5, 4096, 0); // EM_LIMITTEXT
            if key == "language" {
                EnableWindow(edit, 0);
            }
        }
        for (i, &(label, key)) in CHECKS.iter().enumerate() {
            let control = add(
                "BUTTON",
                label,
                20,
                248 + i as i32 * 29,
                630,
                26,
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                200 + i,
            );
            SendMessageW(control, BM_SETCHECK, usize::from(config.boolean(key)), 0);
        }
        for (id, label, name, options, y) in [
            (110, "Typing method", "typing_mode", MODES, 376),
            (111, "Paste shortcut", "paste_shortcut", SHORTCUTS, 414),
        ] {
            add("STATIC", label, 12, y + 3, 225, 26, 0, 0);
            let combo = add(
                "COMBOBOX",
                "",
                245,
                y,
                405,
                140,
                WS_TABSTOP | CBS_DROPDOWNLIST as u32,
                id,
            );
            for (label, _) in options {
                SendMessageW(combo, CB_ADDSTRING, 0, w(label).as_ptr() as isize);
            }
            let index = options
                .iter()
                .position(|(_, value)| *value == config.string(name))
                .unwrap_or(0);
            SendMessageW(combo, CB_SETCURSEL, index, 0);
        }
        add(
            "STATIC",
            "Language is retained for compatibility, not sent. All transcription uses the cloud.",
            15,
            461,
            640,
            40,
            0,
            0,
        );
        add(
            "BUTTON",
            "&Save",
            440,
            512,
            96,
            32,
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            SAVE,
        );
        add("BUTTON", "Cancel", 548, 512, 96, 32, WS_TABSTOP, CANCEL);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(GetDlgItem(hwnd, 100));
        let mut msg: MSG = zeroed();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                if result == 0 {
                    PostQuitMessage(0);
                }
                break;
            }
            if msg.message == WM_KEYDOWN
                && (msg.wParam == 0x0d || msg.wParam == 0x1b)
                && (msg.hwnd == hwnd || IsChild(hwnd, msg.hwnd) != 0)
            {
                SendMessageW(
                    hwnd,
                    WM_COMMAND,
                    if msg.wParam == 0x0d { SAVE } else { CANCEL },
                    0,
                );
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        STATE.with(|cell| {
            cell.borrow_mut()
                .take()
                .and_then(|(cfg, accepted)| if accepted { Some(cfg) } else { None })
        })
    }
}
