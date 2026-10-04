//! Native settings: themed controls, clear sections, DPI-aware layout and tab navigation.
use crate::{config::Config, ui};
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
    Graphics::Gdi::{InvalidateRect, COLOR_WINDOW},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};
const SAVE: usize = 500;
const CANCEL: usize = 501;
const TAB: i32 = 600;
const TEXT: &[(&str, &str)] = &[
    ("&API key", "api_key"),
    ("&Custom shortcut", "hotkey_custom"),
    ("Language (not sent)", "language"),
    ("Realtime model", "model"),
    ("Batch model", "offline_model"),
    ("Realtime URL", "base_url"),
];
const CHECKS: &[(&str, &str)] = &[
    ("Replace Win+&H", "hotkey_win_h"),
    ("Copilot combinations", "hotkey_copilot"),
    ("Use &batch transcription", "offline_mode"),
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
use ui::w;

// Labels use 1000 + their control ID; headers/notes use shared typography IDs.
const GENERAL: &[i32] = &[
    901, 100, 1100, 902, 101, 1101, 200, 201, 903, 110, 1110, 111, 1111, 203,
];
const SERVICE: &[i32] = &[
    904, 202, 910, 905, 103, 1103, 104, 1104, 105, 1105, 102, 1102, 911,
];
unsafe fn page(hwnd: HWND) {
    let general = SendMessageW(GetDlgItem(hwnd, TAB), TCM_GETCURSEL, 0, 0) == 0;
    for (ids, visible) in [(GENERAL, general), (SERVICE, !general)] {
        for &id in ids {
            ShowWindow(
                GetDlgItem(hwnd, id),
                if visible { SW_SHOW } else { SW_HIDE },
            );
        }
    }
    InvalidateRect(hwnd, null(), 1);
}
unsafe fn layout(hwnd: HWND) {
    for (id, x, y, width, height) in [
        (900, 24, 18, 632, 34),
        (912, 24, 57, 632, 24),
        (TAB, 24, 96, 632, 36),
        (901, 24, 152, 632, 26),
        (902, 24, 240, 632, 26),
        (903, 24, 370, 632, 26),
        (904, 24, 152, 632, 26),
        (905, 24, 288, 632, 26),
        (200, 24, 316, 300, 28),
        (201, 336, 316, 320, 28),
        (203, 24, 500, 632, 28),
        (202, 24, 192, 632, 28),
        (910, 24, 232, 632, 44),
        (911, 24, 500, 632, 44),
        (SAVE as i32, 456, 576, 96, 32),
        (CANCEL as i32, 560, 576, 96, 32),
    ] {
        ui::move_control(hwnd, id, x, y, width, height);
    }
    for (id, y) in [
        (100, 184),
        (101, 272),
        (110, 406),
        (111, 448),
        (103, 324),
        (104, 366),
        (105, 408),
        (102, 450),
    ] {
        ui::move_control(hwnd, 1000 + id, 24, y + 5, 184, 24);
        ui::move_control(
            hwnd,
            id,
            216,
            y,
            440,
            if id == 110 || id == 111 { 180 } else { 30 },
        );
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = ui::colors(msg, wp, lp) {
        return result;
    }
    match msg {
        WM_NOTIFY if !((lp as *const NMHDR).is_null()) => {
            let notification = &*(lp as *const NMHDR);
            if notification.idFrom == TAB as usize && notification.code == TCN_SELCHANGE {
                page(hwnd);
            }
            return 0;
        }
        WM_SIZE => {
            layout(hwnd);
            return 0;
        }
        WM_DPICHANGED => {
            ui::dpi_changed(hwnd, lp);
            ui::client_size(hwnd, 680, 624);
            layout(hwnd);
            return 0;
        }
        WM_PAINT => {
            ui::paint_footer(hwnd, ui::scale(hwnd, 560));
            return 0;
        }
        WM_NCDESTROY => {
            ui::detach(hwnd);
        }
        _ => {}
    }
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
                let content = w(&e.to_string());
                if TaskDialog(
                    hwnd,
                    null_mut(),
                    w("Dictation Hotkey").as_ptr(),
                    w("Check your settings").as_ptr(),
                    content.as_ptr(),
                    TDCBF_OK_BUTTON,
                    TD_ERROR_ICON,
                    null_mut(),
                ) < 0
                {
                    MessageBoxW(
                        hwnd,
                        content.as_ptr(),
                        w("Invalid settings").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
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
        ui::init();
        let class = w("DictationHotkeyNativeSettings");
        let instance = GetModuleHandleW(null());
        REGISTER.call_once(|| {
            let mut wc: WNDCLASSW = zeroed();
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            wc.lpfnWndProc = Some(proc);
            wc.hbrBackground = (COLOR_WINDOW + 1) as _;
            wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
            RegisterClassW(&wc);
        });
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            w("Dictation Hotkey — Settings").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            680,
            616,
            parent,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        ui::attach(hwnd, false);
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        STATE.with(|cell| *cell.borrow_mut() = Some((config.clone(), false)));
        let add = |kind: &str, text: &str, style: u32, id: usize| {
            let child = CreateWindowExW(
                0,
                w(kind).as_ptr(),
                w(text).as_ptr(),
                WS_VISIBLE | WS_CHILD | style,
                0,
                0,
                1,
                1,
                hwnd,
                id as _,
                instance,
                null(),
            );
            ui::style_control(hwnd, child);
            child
        };
        add("STATIC", "Settings", 0, 900);
        add(
            "STATIC",
            "Choose your shortcuts and how dictated text is inserted.",
            0,
            912,
        );
        let tab = add("SysTabControl32", "", WS_TABSTOP, TAB as usize);
        for (index, label) in ["General", "Transcription"].into_iter().enumerate() {
            let mut text = w(label);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: text.as_mut_ptr(),
                ..zeroed()
            };
            SendMessageW(tab, TCM_INSERTITEMW, index, &item as *const _ as isize);
        }
        for (id, text) in [
            (901, "Mistral account"),
            (902, "Keyboard shortcuts"),
            (903, "Text output"),
            (904, "Transcription mode"),
            (905, "Connection settings"),
        ] {
            add("STATIC", text, 0, id);
        }
        let add_edit = |i: usize| {
            let (label, key) = TEXT[i];
            let id = 100 + i;
            add("STATIC", label, 0, 1000 + id);
            let style = WS_TABSTOP
                | WS_BORDER
                | ES_AUTOHSCROLL as u32
                | if key == "api_key" {
                    ES_PASSWORD as u32
                } else {
                    0
                };
            let edit = add("EDIT", config.string(key), style, id);
            SendMessageW(edit, EM_SETLIMITTEXT, 4096, 0);
            if key == "language" {
                EnableWindow(edit, 0);
            }
            if key == "hotkey_custom" {
                SendMessageW(
                    edit,
                    EM_SETCUEBANNER,
                    0,
                    w("For example, Win+Y").as_ptr() as isize,
                );
            }
        };
        add_edit(0);
        add_edit(1);
        let add_check = |i: usize| {
            let (label, key) = CHECKS[i];
            let control = add(
                "BUTTON",
                label,
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                200 + i,
            );
            SendMessageW(control, BM_SETCHECK, usize::from(config.boolean(key)), 0);
        };
        add_check(0);
        add_check(1);
        for (id, label, name, options) in [
            (110, "Typing method", "typing_mode", MODES),
            (111, "Paste shortcut", "paste_shortcut", SHORTCUTS),
        ] {
            add("STATIC", label, 0, 1000 + id);
            let combo = add("COMBOBOX", "", WS_TABSTOP | CBS_DROPDOWNLIST as u32, id);
            for (label, _) in options {
                SendMessageW(combo, CB_ADDSTRING, 0, w(label).as_ptr() as isize);
            }
            let index = options
                .iter()
                .position(|(_, value)| *value == config.string(name))
                .unwrap_or(0);
            SendMessageW(combo, CB_SETCURSEL, index, 0);
        }
        // Create controls in visual order so Tab and label mnemonics are natural.
        add_check(3);
        add_check(2);
        for i in [3, 4, 5, 2] {
            add_edit(i);
        }
        add("STATIC", "Realtime inserts text as you speak. Batch sends the recording when you stop.\r\nBoth modes require an internet connection and a Mistral API key.", 0, 910);
        add("STATIC", "Defaults work for most accounts. Language is kept for compatibility\r\nand is not sent to the transcription service.", 0, 911);
        add(
            "BUTTON",
            "&Save",
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            SAVE,
        );
        add("BUTTON", "Cancel", WS_TABSTOP, CANCEL);
        ui::client_size(hwnd, 680, 624);
        layout(hwnd);
        page(hwnd);
        ui::center(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(GetDlgItem(hwnd, 100));
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
                // Let an open combo handle Enter/Escape before saving/canceling.
                if (msg.hwnd == GetDlgItem(hwnd, 110) || msg.hwnd == GetDlgItem(hwnd, 111))
                    && SendMessageW(msg.hwnd, CB_GETDROPPEDSTATE, 0, 0) != 0
                {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                    continue;
                }
                SendMessageW(
                    hwnd,
                    WM_COMMAND,
                    if msg.wParam == 0x0d { SAVE } else { CANCEL },
                    0,
                );
                continue;
            }
            if !crate::logs_ui::dialog_message(&msg) && IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // GetMessage can stop before the window is closed (e.g. system shutdown).
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        STATE.with(|cell| {
            cell.borrow_mut()
                .take()
                .and_then(|(cfg, accepted)| if accepted { Some(cfg) } else { None })
        })
    }
}
