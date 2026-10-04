#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(not(windows))]
fn main() {
    eprintln!("Dictation Hotkey is Windows-only; cargo test runs the portable core tests.");
}

#[cfg(windows)]
mod app {
    use dictation_hotkey_native::{
        bounded::Queue,
        clipboard,
        config::Config,
        hotkey::{self, Action, Matcher},
        logs_ui, output,
        overlay::{Overlay, STOP_MESSAGE},
        paths,
        runtime::{self, Control, Event},
        session::{Controller, Phase, Ticket},
        settings_ui, startup,
    };
    use std::{
        cell::RefCell,
        io,
        mem::{size_of, zeroed},
        path::PathBuf,
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
            Arc, Mutex, OnceLock,
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM,
        },
        System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT},
            Shell::*,
            WindowsAndMessaging::*,
        },
    };
    const CLASS: &str = "DictationHotkeyNativeController";
    const TRAY: u32 = WM_APP + 1;
    const TOGGLE: u32 = WM_APP + 2;
    const RESULT: u32 = WM_APP + 3;
    const SETTINGS: usize = 1;
    const LOGS: usize = 2;
    const COPY: usize = 3;
    const BATCH: usize = 4;
    const QUIT: usize = 5;
    static TASKBAR: AtomicU32 = AtomicU32::new(0);
    static CONTROLLER: AtomicUsize = AtomicUsize::new(0);
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static WIN_H: AtomicBool = AtomicBool::new(false);
    static COPILOT: AtomicBool = AtomicBool::new(false);
    static DIALOG: AtomicBool = AtomicBool::new(false);
    static POSTED: AtomicBool = AtomicBool::new(false);
    static EVENTS: OnceLock<Mutex<Queue<Event>>> = OnceLock::new();
    thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::default()); }
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn queue() -> &'static Mutex<Queue<Event>> {
        EVENTS.get_or_init(|| Mutex::new(Queue::default()))
    }
    fn post(hwnd: HWND, event: Event) -> bool {
        let (size, terminal) = match &event {
            Event::Delta(_, s) => (s.len() + 64, false),
            Event::BatchDone(_, s) | Event::Failed(_, s) => (s.len() + 64, true),
            _ => (64, true),
        };
        let mut queue = queue().lock().unwrap_or_else(|e| e.into_inner());
        if !queue.push(event, size, terminal) {
            return false;
        }
        if !POSTED.swap(true, Ordering::AcqRel) && unsafe { PostMessageW(hwnd, RESULT, 0, 0) } == 0
        {
            queue.clear();
            POSTED.store(false, Ordering::Release);
            return false;
        }
        true
    }
    unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if code >= 0
            && matches!(
                wp as u32,
                WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
            )
        {
            let key = &*(lp as *const KBDLLHOOKSTRUCT);
            let down = wp as u32 == WM_KEYDOWN || wp as u32 == WM_SYSKEYDOWN;
            let (suppress, action) = MATCHER.with(|cell| {
                let mut matcher = cell.borrow_mut();
                let modifiers = matcher.modifiers();
                matcher.event(
                    key.vkCode,
                    down,
                    key.dwExtraInfo == output::MARK,
                    modifiers,
                    WIN_H.load(Ordering::Relaxed),
                    COPILOT.load(Ordering::Relaxed),
                    ACTIVE.load(Ordering::Acquire),
                )
            });
            if action == Some(Action::MaskWindowsRelease) {
                // Swallow the original release only if its replacement was queued.
                // The replacement puts an unused key before Win-up in one batch;
                // injecting a mask then forwarding the original up races the shell.
                if output::mask_windows_release(key.vkCode as u16, key.scanCode as u16).is_ok() {
                    return 1;
                }
                return CallNextHookEx(null_mut(), code, wp, lp);
            }
            if let Some(action) = action {
                if !DIALOG.load(Ordering::Relaxed) {
                    PostMessageW(
                        CONTROLLER.load(Ordering::Relaxed) as HWND,
                        if action == Action::Stop {
                            STOP_MESSAGE
                        } else {
                            TOGGLE
                        },
                        0,
                        0,
                    );
                }
            }
            if suppress {
                return 1;
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
    fn sound(on: bool) {
        use windows_sys::Win32::Media::Audio::{
            PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT,
        };
        let path =
            PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
                .join("Media")
                .join(if on {
                    "Speech On.wav"
                } else {
                    "Speech Off.wav"
                });
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            PlaySoundW(
                path.as_ptr(),
                null_mut(),
                SND_ASYNC | SND_FILENAME | SND_NODEFAULT,
            );
        }
    }
    fn tray(hwnd: HWND, operation: u32, status: &str, state: u16) {
        unsafe {
            let mut data: NOTIFYICONDATAW = zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = TRAY;
            let dpi = GetDpiForWindow(hwnd);
            data.hIcon = LoadImageW(
                GetModuleHandleW(null()),
                state as usize as _,
                IMAGE_ICON,
                GetSystemMetricsForDpi(SM_CXSMICON, dpi),
                GetSystemMetricsForDpi(SM_CYSMICON, dpi),
                LR_DEFAULTCOLOR,
            );
            let text: Vec<u16> = status.encode_utf16().take(126).collect();
            data.szTip[..text.len()].copy_from_slice(&text);
            if text.last().is_some_and(|c| (0xD800..=0xDBFF).contains(c)) {
                data.szTip[text.len() - 1] = 0;
            }
            if Shell_NotifyIconW(operation, &data) == 0 && operation == NIM_ADD {
                logs_ui::log("Tray icon creation failed");
            }
            if !data.hIcon.is_null() {
                DestroyIcon(data.hIcon);
            }
        }
    }
    struct App {
        hwnd: HWND,
        overlay: Overlay,
        config: Config,
        config_path: PathBuf,
        frozen: Option<Config>,
        core: Controller,
        worker: Option<JoinHandle<()>>,
        control: Option<Control>,
        hook: HHOOK,
        custom_id: i32,
        status: String,
        pending: String,
        terminal: Option<Ticket>,
        pending_copy: Option<String>,
        busy_since: Option<Instant>,
        next_output: Instant,
        error: bool,
        partial_fallback: bool,
        quitting: bool,
    }
    impl App {
        fn capturing(&self) -> bool {
            matches!(
                self.core.phase,
                Phase::RecordingRealtime | Phase::RecordingBatch | Phase::RecordingFallback
            ) && !self.quitting
        }
        fn busy(&self) -> bool {
            self.worker.is_some()
                || self.terminal.is_some()
                || !self.pending.is_empty()
                || self.pending_copy.is_some()
        }
        fn show(&mut self, status: &str, persistent: bool) {
            self.status = status.chars().take(512).collect();
            logs_ui::log(&self.status);
            self.overlay
                .show(&self.status, self.capturing(), persistent);
            tray(
                self.hwnd,
                NIM_MODIFY,
                &self.status,
                if self.capturing() {
                    102
                } else if self.busy() {
                    103
                } else {
                    101
                },
            );
        }
        fn timer(&self) {
            unsafe {
                SetTimer(self.hwnd, 1, 50, None);
            }
        }
        fn fail_output(&mut self, reason: &str) {
            self.error = true;
            self.pending.clear();
            self.pending_copy = None;
            self.busy_since = None;
            self.core.fail(self.core.ticket, "output failed");
            if !self.core.text.is_empty() {
                self.core.last_text.clone_from(&self.core.text);
            }
            if let Some(control) = &self.control {
                control.cancel();
            }
            ACTIVE.store(false, Ordering::Release);
            self.show(reason, true);
            self.timer();
        }
        fn begin(&mut self, clipboard_only: bool) {
            if self.quitting || DIALOG.load(Ordering::Relaxed) {
                return;
            }
            if self.busy() {
                self.show("Finishing previous session...", true);
                return;
            }
            self.core.reset();
            if let Err(e) = self.config.validate() {
                self.show(&format!("Settings need correction: {e}"), true);
                unsafe {
                    PostMessageW(self.hwnd, WM_COMMAND, SETTINGS, 0);
                }
                return;
            }
            if self.config.string("api_key").is_empty() {
                unsafe {
                    PostMessageW(self.hwnd, WM_COMMAND, SETTINGS, 0);
                }
                return;
            }
            let Some(ticket) = self
                .core
                .start(self.config.boolean("offline_mode"), clipboard_only)
            else {
                return;
            };
            self.error = false;
            self.partial_fallback = false;
            self.pending.clear();
            self.terminal = None;
            self.next_output = Instant::now();
            self.frozen = Some(self.config.clone());
            let control = Control::default();
            let hwnd = self.hwnd as usize;
            match runtime::spawn(
                self.config.clone(),
                ticket,
                control.clone(),
                Arc::new(move |event| post(hwnd as HWND, event)),
            ) {
                Ok(worker) => {
                    self.worker = Some(worker);
                    self.control = Some(control);
                    ACTIVE.store(true, Ordering::Release);
                    sound(true);
                    self.show(
                        if clipboard_only {
                            "Recording to clipboard - click or press Escape to stop"
                        } else {
                            "Recording - hotkey or Escape to stop"
                        },
                        true,
                    );
                    self.timer();
                }
                Err(_) => {
                    self.core.fail(ticket, "cannot start worker");
                    self.show("Unable to start capture worker", true);
                }
            }
        }
        fn stop(&mut self) {
            if self.core.stop() {
                if let Some(control) = &self.control {
                    control.stop.store(true, Ordering::Release);
                }
                if ACTIVE.swap(false, Ordering::AcqRel) {
                    sound(false);
                }
                self.show("Finishing transcription...", true);
            }
        }
        fn toggle(&mut self, clipboard_only: bool) {
            if self.capturing() {
                self.stop();
            } else if !self.busy() {
                self.begin(clipboard_only);
            }
        }
        fn event(&mut self, event: Event) {
            let ticket = event.ticket();
            if self.quitting || ticket.session != self.core.ticket.session {
                return;
            }
            if matches!(event, Event::MicStopped(_)) {
                self.stop();
                return;
            }
            if ticket != self.core.ticket {
                return;
            }
            match event {
                Event::Delta(_, text) => {
                    let before = self.core.text.len();
                    if self.core.delta(ticket, &text) {
                        if before == 0 {
                            logs_ui::log("First transcript delta received (content omitted)");
                        }
                        self.pending.push_str(&self.core.text[before..]);
                        self.timer();
                    }
                    if self.core.phase == Phase::Failed && !self.error {
                        self.fail_output("Transcript limit reached (256 KiB)");
                    }
                }
                Event::Fallback(_) => {
                    // Unsubmitted deltas must not be injected after fallback. A prefix that reached
                    // SendInput makes full-batch insertion unsafe; preserve/copy it instead.
                    self.pending.clear();
                    self.busy_since = None;
                    self.core.realtime_failure(ticket);
                    self.show(
                        if self.capturing() {
                            "Realtime unavailable - recording continues for batch"
                        } else {
                            "Realtime unavailable - submitting cloud batch"
                        },
                        true,
                    );
                }
                Event::RealtimeDone(_) => {
                    self.terminal = Some(ticket);
                    if self.core.clipboard_only && !self.core.text.is_empty() {
                        self.pending_copy = Some(self.core.text.clone());
                    }
                    self.timer();
                }
                Event::BatchDone(_, text) => {
                    let insert = self.core.batch_result(ticket, text);
                    self.terminal = Some(ticket);
                    if self.core.phase == Phase::Failed {
                        self.fail_output("Transcript limit reached (256 KiB)");
                        return;
                    }
                    self.partial_fallback = self.core.injected_prefix && !self.core.clipboard_only;
                    if insert {
                        self.pending.clone_from(&self.core.text);
                    } else if (self.core.clipboard_only || self.partial_fallback)
                        && !self.core.text.is_empty()
                    {
                        self.pending_copy = Some(self.core.text.clone());
                    }
                    self.timer();
                }
                Event::Failed(_, error) => {
                    self.terminal = Some(ticket);
                    if !self.error {
                        self.core.fail(ticket, "session failed");
                        self.error = true;
                        self.pending.clear();
                        self.show(&format!("Session failed: {error}"), true);
                    }
                    if ACTIVE.swap(false, Ordering::AcqRel) {
                        sound(false);
                    }
                    self.timer();
                }
                Event::MicStopped(_) => (),
            }
        }
        fn tick(&mut self) {
            if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
                if let Some(worker) = self.worker.take() {
                    if worker.join().is_err() {
                        self.error = true;
                        self.terminal = Some(self.core.ticket);
                        self.show("Session worker crashed", true);
                    }
                }
                self.control = None;
                if self.error && self.terminal.is_none() {
                    self.terminal = Some(self.core.ticket);
                }
            }
            if self.quitting {
                if self.worker.is_none() {
                    unsafe {
                        PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
                    }
                }
                return;
            }
            let now = Instant::now();
            if now >= self.next_output {
                let result = if !self.pending.is_empty() {
                    let cfg = self.frozen.as_ref().unwrap_or(&self.config);
                    let mut length =
                        self.pending
                            .len()
                            .min(if cfg.string("typing_mode") == "keystrokes" {
                                4096
                            } else {
                                32768
                            });
                    while !self.pending.is_char_boundary(length) {
                        length -= 1;
                    }
                    match output::type_text(
                        self.hwnd,
                        &self.pending[..length],
                        cfg.string("typing_mode"),
                        cfg.string("paste_shortcut"),
                    ) {
                        Ok(()) => {
                            self.pending.drain(..length);
                            self.core.output_succeeded(self.core.ticket);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else if let Some(text) = &self.pending_copy {
                    match output::copy(self.hwnd, text) {
                        Ok(()) => {
                            self.pending_copy = None;
                            if self.terminal.is_none() {
                                self.show("Last transcript copied", false);
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    Ok(())
                };
                match result {
                    Ok(()) => { self.busy_since = None; self.next_output = now + Duration::from_millis(150); }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        let started = *self.busy_since.get_or_insert(now);
                        if started.elapsed() >= Duration::from_secs(2) { self.fail_output("Clipboard or modifiers stayed busy; Copy Last Text to recover"); }
                    }
                    Err(_) => self.fail_output("Input failed (possibly an elevated target/UIPI); Copy Last Text to recover"),
                }
            }
            if self.terminal.is_some()
                && self.worker.is_none()
                && self.pending.is_empty()
                && self.pending_copy.is_none()
            {
                let ticket = self.terminal.take().unwrap();
                if self.error {
                    if !self.core.text.is_empty() {
                        self.core.last_text.clone_from(&self.core.text);
                    }
                } else {
                    self.core.finish(ticket);
                    self.show(
                        if self.partial_fallback {
                            "Partial text already inserted; complete result copied (not reinserted)"
                        } else if self.core.text.is_empty() {
                            "No speech detected"
                        } else if self.core.clipboard_only {
                            "Transcript copied to clipboard"
                        } else {
                            "Done"
                        },
                        self.partial_fallback,
                    );
                }
                self.frozen = None;
                tray(self.hwnd, NIM_MODIFY, &self.status, 101);
            }
            if !self.busy() {
                unsafe {
                    KillTimer(self.hwnd, 1);
                }
            }
        }
        fn quit(&mut self) {
            if self.quitting {
                return;
            }
            self.quitting = true;
            self.pending.clear();
            self.pending_copy = None;
            ACTIVE.store(false, Ordering::Release);
            if let Some(control) = &self.control {
                control.cancel();
            }
            self.show("Closing microphone and network...", true);
            self.timer();
        }
        fn custom_config(&mut self, updated: Config) {
            let previous = hotkey::parse(self.config.string("hotkey_custom"));
            let next = hotkey::parse(updated.string("hotkey_custom"));
            let next_id = if self.custom_id == 1 { 2 } else { 1 };
            if previous != next {
                if let Some(key) = next {
                    if unsafe {
                        RegisterHotKey(self.hwnd, next_id, key.modifiers | MOD_NOREPEAT, key.vk)
                    } == 0
                    {
                        self.show("Shortcut is in use; no settings changed", true);
                        return;
                    }
                }
            }
            let changed_startup =
                self.config.boolean("start_with_windows") != updated.boolean("start_with_windows");
            let old_shortcut = if changed_startup {
                match startup::snapshot() {
                    Ok(s) => Some(s),
                    Err(_) => {
                        if previous != next {
                            unsafe {
                                UnregisterHotKey(self.hwnd, next_id);
                            }
                        }
                        self.show("Cannot back up Startup shortcut; settings unchanged", true);
                        return;
                    }
                }
            } else {
                None
            };
            let result = (|| -> io::Result<()> {
                if changed_startup {
                    startup::set(updated.boolean("start_with_windows"))?;
                }
                updated.save(&self.config_path)
            })();
            if result.is_err() {
                if previous != next {
                    unsafe {
                        UnregisterHotKey(self.hwnd, next_id);
                    }
                }
                if let Some(old) = old_shortcut {
                    if startup::restore(old).is_err() {
                        self.show(
                            "Settings save failed; Startup rollback also failed - check shortcut",
                            true,
                        );
                        return;
                    }
                }
                self.show(
                    "Unable to save settings; previous configuration retained",
                    true,
                );
                return;
            }
            if previous != next {
                unsafe {
                    UnregisterHotKey(self.hwnd, self.custom_id);
                }
                self.custom_id = next_id;
            }
            WIN_H.store(updated.boolean("hotkey_win_h"), Ordering::Relaxed);
            COPILOT.store(updated.boolean("hotkey_copilot"), Ordering::Relaxed);
            self.config = updated;
            self.show("Settings saved", false);
        }
    }
    fn with_app(f: impl FnOnce(&mut App)) {
        APP.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                if let Some(app) = guard.as_mut() {
                    f(app);
                }
            }
        });
    }
    fn menu(hwnd: HWND) {
        let snapshot = APP.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|app| (app.status.clone(), app.config.boolean("offline_mode")))
        });
        let Some((status, batch)) = snapshot else {
            return;
        };
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(menu, MF_STRING | MF_DISABLED, 0, w(&status).as_ptr());
            for (id, text) in [
                (COPY, "Copy Last Text"),
                (SETTINGS, "Settings"),
                (LOGS, "View Logs"),
            ] {
                AppendMenuW(menu, MF_STRING, id, w(text).as_ptr());
            }
            AppendMenuW(
                menu,
                MF_STRING | if batch { MF_CHECKED } else { 0 },
                BATCH,
                w("Batch transcription (cloud)").as_ptr(),
            );
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, QUIT, w("Quit").as_ptr());
            let mut point: POINT = zeroed();
            GetCursorPos(&mut point);
            SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                hwnd,
                null(),
            );
            DestroyMenu(menu);
            if command != 0 {
                PostMessageW(hwnd, WM_COMMAND, command as usize, 0);
            }
            PostMessageW(hwnd, WM_NULL, 0, 0);
        }
    }
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == RESULT {
            loop {
                let event = {
                    let mut queue = queue().lock().unwrap_or_else(|e| e.into_inner());
                    let event = queue.pop();
                    if event.is_none() {
                        POSTED.store(false, Ordering::Release);
                    }
                    event
                };
                let Some(event) = event else {
                    break;
                };
                with_app(|app| app.event(event));
            }
            return 0;
        }
        if msg == TASKBAR.load(Ordering::Relaxed) && msg != 0 {
            with_app(|app| {
                tray(
                    hwnd,
                    NIM_ADD,
                    &app.status,
                    if app.capturing() { 102 } else { 101 },
                )
            });
            return 0;
        }
        match msg {
            TOGGLE | WM_HOTKEY => {
                with_app(|app| app.toggle(false));
                0
            }
            STOP_MESSAGE => {
                with_app(|app| {
                    if app.capturing() {
                        app.stop();
                    } else if !app.busy() {
                        app.overlay.hide();
                    }
                });
                0
            }
            TRAY if lp as u32 == WM_LBUTTONUP => {
                with_app(|app| app.toggle(true));
                0
            }
            TRAY if lp as u32 == WM_RBUTTONUP => {
                menu(hwnd);
                0
            }
            WM_TIMER if wp == 1 => {
                with_app(App::tick);
                0
            }
            WM_TIMER if wp == clipboard::RESTORE_TIMER => {
                if let Err(error) = clipboard::restore(hwnd) {
                    if error.kind() != io::ErrorKind::WouldBlock {
                        logs_ui::log("Clipboard restoration failed; retrying");
                    }
                }
                0
            }
            WM_COMMAND if wp & 0xffff == SETTINGS => {
                if DIALOG.load(Ordering::Relaxed) {
                    return 0;
                }
                let config = APP.with(|cell| {
                    cell.borrow()
                        .as_ref()
                        .filter(|app| !app.busy() && !app.quitting)
                        .map(|app| app.config.clone())
                });
                if let Some(config) = config {
                    DIALOG.store(true, Ordering::Relaxed);
                    let updated = settings_ui::open(hwnd, &config);
                    DIALOG.store(false, Ordering::Relaxed);
                    if let Some(updated) = updated {
                        with_app(|app| app.custom_config(updated));
                    }
                } else {
                    with_app(|app| app.show("Finish dictation before opening Settings", true));
                }
                0
            }
            WM_COMMAND if wp & 0xffff == LOGS => {
                logs_ui::open(hwnd);
                0
            }
            WM_COMMAND if wp & 0xffff == COPY => {
                with_app(|app| {
                    if app.busy() {
                        app.show("Finish dictation before copying the last result", true);
                    } else if app.core.last_text.is_empty() {
                        app.show("No completed transcript to copy", false);
                    } else {
                        app.pending_copy = Some(app.core.last_text.clone());
                        app.next_output = Instant::now();
                        app.timer();
                        app.show("Copying last text...", false);
                    }
                });
                0
            }
            WM_COMMAND if wp & 0xffff == BATCH => {
                with_app(|app| {
                    let mut updated = app.config.clone();
                    updated.set_bool("offline_mode", !updated.boolean("offline_mode"));
                    if updated.save(&app.config_path).is_ok() {
                        app.config = updated;
                        logs_ui::log("Batch mode setting saved (next session)");
                    } else {
                        app.show("Unable to save batch mode setting", true);
                    }
                });
                0
            }
            WM_COMMAND if wp & 0xffff == QUIT => {
                with_app(App::quit);
                0
            }
            WM_CLOSE => {
                let ready = APP.with(|cell| {
                    cell.borrow()
                        .as_ref()
                        .is_none_or(|app| app.quitting && app.worker.is_none())
                });
                if ready {
                    DestroyWindow(hwnd);
                } else {
                    with_app(App::quit);
                }
                0
            }
            WM_DESTROY => {
                let _ = clipboard::restore(hwnd);
                with_app(|app| {
                    if !app.hook.is_null() {
                        UnhookWindowsHookEx(app.hook);
                        app.hook = null_mut();
                    }
                    UnregisterHotKey(hwnd, 1);
                    UnregisterHotKey(hwnd, 2);
                    KillTimer(hwnd, 1);
                    tray(hwnd, NIM_DELETE, "", 101);
                });
                queue().lock().unwrap_or_else(|e| e.into_inner()).clear();
                POSTED.store(false, Ordering::Release);
                PostQuitMessage(0);
                0
            }
            WM_ENDSESSION if wp != 0 => {
                with_app(|app| {
                    if let Some(control) = &app.control {
                        control.cancel();
                    }
                });
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
    pub fn run() -> Result<(), String> {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
            .ok()
            .map_err(|e| format!("COM: {e}"))?;
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    windows::Win32::System::Com::CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        unsafe {
            let mutex = CreateMutexW(null(), 0, w("Local\\DictationHotkeyNative").as_ptr());
            if mutex.is_null() {
                return Err("single-instance mutex failed".into());
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let existing = FindWindowW(w(CLASS).as_ptr(), null());
                if !existing.is_null() {
                    PostMessageW(existing, WM_COMMAND, SETTINGS, 0);
                }
                CloseHandle(mutex);
                return Ok(());
            }
            struct MutexHandle(windows_sys::Win32::Foundation::HANDLE);
            impl Drop for MutexHandle {
                fn drop(&mut self) {
                    unsafe {
                        CloseHandle(self.0);
                    }
                }
            }
            let _mutex = MutexHandle(mutex);
            let path = paths::config().map_err(|e| format!("configuration directory: {e}"))?;
            let (config,warning) = match Config::load(&path) {Ok(cfg)=>(cfg,None),Err(_)=>(Config::default(),Some("Configuration could not be read; original file preserved. Correct it or save new settings."))};
            TASKBAR.store(
                RegisterWindowMessageW(w("TaskbarCreated").as_ptr()),
                Ordering::Relaxed,
            );
            let instance = GetModuleHandleW(null());
            let class = w(CLASS);
            let mut wc: WNDCLASSW = zeroed();
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            wc.lpfnWndProc = Some(proc);
            if RegisterClassW(&wc) == 0 {
                return Err("register controller failed".into());
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                w("Dictation Hotkey").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err("create controller failed".into());
            }
            CONTROLLER.store(hwnd as usize, Ordering::Relaxed);
            let overlay = Overlay::create(hwnd)?;
            WIN_H.store(config.boolean("hotkey_win_h"), Ordering::Relaxed);
            COPILOT.store(config.boolean("hotkey_copilot"), Ordering::Relaxed);
            let custom = hotkey::parse(config.string("hotkey_custom"));
            let missing_key = config.string("api_key").is_empty();
            APP.with(|cell| {
                *cell.borrow_mut() = Some(App {
                    hwnd,
                    overlay,
                    config,
                    config_path: path,
                    frozen: None,
                    core: Controller::default(),
                    worker: None,
                    control: None,
                    hook: null_mut(),
                    custom_id: 1,
                    status: "Ready".into(),
                    pending: String::new(),
                    terminal: None,
                    pending_copy: None,
                    busy_since: None,
                    next_output: Instant::now(),
                    error: false,
                    partial_fallback: false,
                    quitting: false,
                })
            });
            tray(hwnd, NIM_ADD, "Dictation Hotkey - ready", 101);
            if let Some(key) = custom {
                if RegisterHotKey(hwnd, 1, key.modifiers | MOD_NOREPEAT, key.vk) == 0 {
                    with_app(|app| {
                        app.show(
                            "Custom shortcut is in use; choose another in Settings",
                            true,
                        )
                    });
                }
            }
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), instance, 0);
            with_app(|app| {
                app.hook = hook;
                if hook.is_null() {
                    app.show(
                        "Win+H/Copilot hook unavailable; use a custom shortcut",
                        true,
                    );
                }
                if let Some(warning) = warning {
                    app.show(warning, true);
                }
            });
            logs_ui::log("Native app ready; microphone and network are inactive");
            if missing_key {
                PostMessageW(hwnd, WM_COMMAND, SETTINGS, 0);
            }
            let mut msg: MSG = zeroed();
            loop {
                let result = GetMessageW(&mut msg, null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            APP.with(|cell| {
                cell.borrow_mut().take();
            });
            CONTROLLER.store(0, Ordering::Relaxed);
        }
        Ok(())
    }
}
#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "Dictation Hotkey".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}
