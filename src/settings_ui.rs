//! Native settings: themed controls, clear sections, DPI-aware layout and tab navigation.
use crate::{config::Config, ui, wire};
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
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        DrawFocusRect, DrawTextW, FillRect, GetDC, GetSysColor, GetSysColorBrush, GetTextMetricsW,
        InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor, COLOR_3DLIGHT,
        COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, DT_CENTER, DT_LEFT,
        DT_SINGLELINE, DT_VCENTER, TEXTMETRICW, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, SystemServices::SS_OWNERDRAW},
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, SetFocus},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const SAVE: usize = 500;
const CANCEL: usize = 501;
const TAB_GENERAL: i32 = 600;
const TAB_TRANSCRIPTION: i32 = 601;
static ACTIVE_TAB: AtomicUsize = AtomicUsize::new(0);
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

unsafe fn draw_label(hwnd: HWND, item: &DRAWITEMSTRUCT) {
    let mut text = [0u16; 128];
    let length = GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32);
    FillRect(item.hDC, &item.rcItem, GetSysColorBrush(COLOR_WINDOW));
    let previous = SelectObject(item.hDC, SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as _);
    SetBkMode(item.hDC, TRANSPARENT as i32);
    SetTextColor(
        item.hDC,
        GetSysColor(if IsWindowEnabled(item.hwndItem) != 0 {
            COLOR_WINDOWTEXT
        } else {
            COLOR_GRAYTEXT
        }),
    );
    let mut rect = item.rcItem;
    rect.left += ui::scale(hwnd, 2);
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        length,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    if !previous.is_null() {
        SelectObject(item.hDC, previous);
    }
}

unsafe fn draw_tab(hwnd: HWND, item: &DRAWITEMSTRUCT) {
    let selected = (item.CtlID as i32 == TAB_GENERAL && ACTIVE_TAB.load(Ordering::Relaxed) == 0)
        || (item.CtlID as i32 == TAB_TRANSCRIPTION && ACTIVE_TAB.load(Ordering::Relaxed) == 1);
    let rect = item.rcItem;
    FillRect(item.hDC, &rect, GetSysColorBrush(COLOR_WINDOW));

    let previous = SelectObject(item.hDC, SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as _);
    SetBkMode(item.hDC, TRANSPARENT as i32);
    SetTextColor(
        item.hDC,
        GetSysColor(if selected && !ui::high_contrast() {
            COLOR_HIGHLIGHT
        } else {
            COLOR_WINDOWTEXT
        }),
    );
    let mut text_rect = rect;
    text_rect.left += ui::scale(hwnd, 4);
    text_rect.right -= ui::scale(hwnd, 4);
    text_rect.bottom -= ui::scale(hwnd, 3);
    let label = w(if item.CtlID as i32 == TAB_GENERAL {
        "General"
    } else {
        "Transcription"
    });
    DrawTextW(
        item.hDC,
        label.as_ptr(),
        -1,
        &mut text_rect,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );

    let mut underline = rect;
    underline.top = rect.bottom - ui::scale(hwnd, 1).max(1);
    FillRect(item.hDC, &underline, GetSysColorBrush(COLOR_3DLIGHT));
    if selected {
        underline.left += ui::scale(hwnd, 12);
        underline.right -= ui::scale(hwnd, 12);
        underline.top = rect.bottom - ui::scale(hwnd, 2).max(2);
        FillRect(item.hDC, &underline, GetSysColorBrush(COLOR_HIGHLIGHT));
    }
    if item.itemState & ODS_FOCUS != 0 {
        let mut focus = rect;
        focus.left += ui::scale(hwnd, 4);
        focus.right -= ui::scale(hwnd, 4);
        focus.top += ui::scale(hwnd, 3);
        focus.bottom -= ui::scale(hwnd, 4);
        DrawFocusRect(item.hDC, &focus);
    }
    if !previous.is_null() {
        SelectObject(item.hDC, previous);
    }
}

// Labels use 1000 + their control ID; headers/notes use shared typography IDs.
const GENERAL: &[i32] = &[
    901, 100, 1100, 902, 101, 1101, 200, 201, 903, 110, 1110, 111, 1111, 203,
];
const SERVICE: &[i32] = &[
    904, 202, 910, 905, 103, 1103, 104, 1104, 105, 1105, 102, 1102, 911,
];
// A multiline edit's formatting rectangle controls the text, cue banner, and caret baseline.
// ES_AUTOHSCROLL keeps these fields horizontally scrolling instead of wrapping.
struct CueBanner {
    text: Vec<u16>,
    show_focused: bool,
}

unsafe extern "system" fn edit_subclass(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let result = DefSubclassProc(hwnd, msg, wp, lp);
    if msg == WM_PAINT {
        let cue = &*(data as *const CueBanner);
        if GetWindowTextLengthW(hwnd) == 0 && (cue.show_focused || GetFocus() != hwnd) {
            let dc = GetDC(hwnd);
            if !dc.is_null() {
                let font = SendMessageW(hwnd, WM_GETFONT, 0, 0);
                let previous = SelectObject(dc, font as _);
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, GetSysColor(COLOR_GRAYTEXT));
                let mut rect: RECT = zeroed();
                SendMessageW(hwnd, EM_GETRECT, 0, &mut rect as *mut RECT as isize);
                DrawTextW(
                    dc,
                    cue.text.as_ptr(),
                    -1,
                    &mut rect,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );
                if !previous.is_null() {
                    SelectObject(dc, previous);
                }
                ReleaseDC(hwnd, dc);
            }
        }
    }
    if matches!(
        msg,
        WM_SETFOCUS | WM_KILLFOCUS | WM_CHAR | WM_KEYUP | WM_PASTE | WM_CUT | WM_CLEAR | WM_SETTEXT
    ) {
        InvalidateRect(hwnd, null(), 1);
    }
    if msg == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(edit_subclass), id);
        drop(Box::from_raw(data as *mut CueBanner));
    }
    result
}

unsafe fn center_edit(hwnd: HWND, id: i32) {
    let edit = GetDlgItem(hwnd, id);
    if edit.is_null() {
        return;
    }
    let mut rect: RECT = zeroed();
    if GetClientRect(edit, &mut rect) == 0 {
        return;
    }
    let dc = GetDC(edit);
    if dc.is_null() {
        return;
    }
    let font = SendMessageW(edit, WM_GETFONT, 0, 0);
    let previous = SelectObject(dc, font as _);
    let mut metrics: TEXTMETRICW = zeroed();
    let text_height = if GetTextMetricsW(dc, &mut metrics) != 0 {
        metrics.tmHeight
    } else {
        ui::scale(hwnd, 15)
    };
    if !previous.is_null() {
        SelectObject(dc, previous);
    }
    ReleaseDC(edit, dc);

    let height = rect.bottom - rect.top;
    let vertical_inset = ((height - text_height).max(0)) / 2;
    rect.left = ui::scale(hwnd, 10);
    rect.right -= ui::scale(hwnd, 10);
    rect.top = vertical_inset;
    rect.bottom -= vertical_inset;
    SendMessageW(edit, EM_SETRECTNP, 0, &rect as *const RECT as isize);
}

unsafe fn page(hwnd: HWND) {
    let general = ACTIVE_TAB.load(Ordering::Relaxed) == 0;
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
        (TAB_GENERAL, 24, 96, 140, 36),
        (TAB_TRANSCRIPTION, 164, 96, 140, 36),
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
        ui::move_control(hwnd, 1000 + id, 24, y, 182, 30);
        ui::move_control(
            hwnd,
            id,
            216,
            y,
            440,
            if id == 110 || id == 111 { 180 } else { 30 },
        );
        if id != 110 && id != 111 {
            center_edit(hwnd, id);
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = ui::colors(msg, wp, lp) {
        return result;
    }
    match msg {
        WM_DRAWITEM if !((lp as *const DRAWITEMSTRUCT).is_null()) => {
            let item = &*(lp as *const DRAWITEMSTRUCT);
            let id = item.CtlID as i32;
            if (1100..=1111).contains(&id) {
                draw_label(hwnd, item);
                return 1;
            }
            if id == TAB_GENERAL || id == TAB_TRANSCRIPTION {
                draw_tab(hwnd, item);
                return 1;
            }
        }
        WM_SIZE => {
            layout(hwnd);
            return 0;
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MINMAXINFO);
            info.ptMinTrackSize.x = ui::scale(hwnd, 680);
            info.ptMinTrackSize.y = ui::scale(hwnd, 624);
            return 0;
        }
        WM_DPICHANGED => {
            ui::dpi_changed(hwnd, lp);
            ui::client_size(hwnd, 680, 624);
            layout(hwnd);
            return 0;
        }
        WM_PAINT => {
            let general = ACTIVE_TAB.load(Ordering::Relaxed) == 0;
            ui::paint_settings(
                hwnd,
                if general {
                    &[901, 902, 903]
                } else {
                    &[904, 905]
                },
            );
            return 0;
        }
        WM_NCDESTROY => {
            ui::detach(hwnd);
        }
        _ => {}
    }
    if msg == WM_COMMAND
        && (wp & 0xffff == TAB_GENERAL as usize || wp & 0xffff == TAB_TRANSCRIPTION as usize)
    {
        ACTIVE_TAB.store(
            usize::from(wp & 0xffff == TAB_TRANSCRIPTION as usize),
            Ordering::Relaxed,
        );
        page(hwnd);
        InvalidateRect(GetDlgItem(hwnd, TAB_GENERAL), null(), 1);
        InvalidateRect(GetDlgItem(hwnd, TAB_TRANSCRIPTION), null(), 1);
        return 0;
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
        ACTIVE_TAB.store(0, Ordering::Relaxed);
        add(
            "BUTTON",
            "General",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            TAB_GENERAL as usize,
        );
        add(
            "BUTTON",
            "Transcription",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            TAB_TRANSCRIPTION as usize,
        );
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
            add("STATIC", label, SS_OWNERDRAW, 1000 + id);
            let style = WS_TABSTOP
                | WS_BORDER
                | ES_MULTILINE as u32
                | ES_LEFT as u32
                | ES_AUTOHSCROLL as u32
                | if key == "api_key" {
                    ES_PASSWORD as u32
                } else {
                    0
                };
            let edit = add("EDIT", config.string(key), style, id);
            SendMessageW(edit, EM_SETLIMITTEXT, 4096, 0);
            let hint = match key {
                "api_key" => "Enter your Mistral API key",
                "hotkey_custom" => "Optional · for example, Win+Y",
                "language" => "Automatic detection (not sent)",
                "model" => wire::DEFAULT_MODEL,
                "offline_model" => wire::DEFAULT_BATCH_MODEL,
                "base_url" => wire::DEFAULT_URL,
                _ => "",
            };
            let cue = Box::new(CueBanner {
                text: w(hint),
                show_focused: key == "api_key",
            });
            let cue = Box::into_raw(cue);
            if SetWindowSubclass(edit, Some(edit_subclass), id, cue as usize) == 0 {
                drop(Box::from_raw(cue));
            }
            if key == "language" {
                EnableWindow(edit, 0);
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
            add("STATIC", label, SS_OWNERDRAW, 1000 + id);
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
        add("STATIC", "Blank model and URL fields use the defaults shown as hints.\r\nBatch uses the Mistral API URL, not the realtime URL above.", 0, 911);
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
