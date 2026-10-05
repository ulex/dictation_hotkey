// Apple frameworks stay behind this C ABI; sessions, queues and provider parsing live in Rust.
import AppKit
import AVFoundation
import Carbon
import ServiceManagement

private let inputMark: Int64 = 0x44484354
private typealias Action = @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?) -> Void
private var shell: Shell?

// A retained, synchronized network object. Caller releases only after all workers join.
private final class Network: NSObject, URLSessionDataDelegate, URLSessionTaskDelegate {
    let condition = NSCondition()
    var session: URLSession!
    var task: URLSessionTask!
    var response = Data()
    var completed = false
    var failed = false
    var closed = false
    override init() {
        super.init()
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 30
        config.timeoutIntervalForResource = 3660
        config.httpCookieStorage = nil
        config.urlCredentialStorage = nil
        let queue = OperationQueue()
        queue.maxConcurrentOperationCount = 1
        session = URLSession(configuration: config, delegate: self, delegateQueue: queue)
    }
    func close() {
        condition.lock(); closed = true; condition.broadcast(); condition.unlock()
        task.cancel()
        session.invalidateAndCancel()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask,
                    didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        completionHandler((response as? HTTPURLResponse)?.statusCode == 200 ? .allow : .cancel)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        condition.lock()
        if response.count + data.count <= 327680 { response.append(data) }
        else { failed = true; dataTask.cancel() }
        condition.unlock()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        condition.lock(); failed = failed || error != nil
        if !(task is URLSessionWebSocketTask) {
            failed = failed || (task.response as? HTTPURLResponse)?.statusCode != 200
        }
        completed = true; condition.broadcast(); condition.unlock()
    }
}
private func network(_ pointer: UnsafeMutableRawPointer) -> Network {
    Unmanaged<Network>.fromOpaque(pointer).takeUnretainedValue()
}
@_cdecl("dh_ws_new")
public func wsNew(_ url: UnsafePointer<CChar>, _ key: UnsafePointer<CChar>) -> UnsafeMutableRawPointer? {
    guard let address = URL(string: String(cString: url)) else { return nil }
    let net = Network()
    var request = URLRequest(url: address)
    request.setValue("Bearer \(String(cString: key))", forHTTPHeaderField: "Authorization")
    let task = net.session.webSocketTask(with: request)
    task.maximumMessageSize = 65536
    net.task = task; task.resume()
    return Unmanaged.passRetained(net).toOpaque()
}
@_cdecl("dh_ws_send")
public func wsSend(_ pointer: UnsafeMutableRawPointer, _ text: UnsafePointer<CChar>) -> Int32 {
    let net = network(pointer)
    let wait = NSCondition()
    var done = false; var failed = false
    (net.task as! URLSessionWebSocketTask).send(.string(String(cString: text))) { error in
        wait.lock(); failed = error != nil; done = true; wait.broadcast(); wait.unlock()
    }
    wait.lock()
    let deadline = Date().addingTimeInterval(15)
    while !done && wait.wait(until: deadline) {}
    let ok = done && !failed
    wait.unlock()
    if !ok { net.close() }
    return ok ? 0 : -1
}
@_cdecl("dh_ws_receive")
public func wsReceive(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafeMutablePointer<UInt8>, _ capacity: Int) -> Int {
    let net = network(pointer)
    let wait = NSCondition()
    var done = false; var result: Data?
    (net.task as! URLSessionWebSocketTask).receive { message in
        wait.lock()
        if case .success(.string(let text)) = message { result = text.data(using: .utf8) }
        done = true; wait.broadcast(); wait.unlock()
    }
    wait.lock()
    while !done {
        _ = wait.wait(until: Date().addingTimeInterval(0.1))
        net.condition.lock(); let closed = net.closed; net.condition.unlock()
        if closed { break }
    }
    let data = result
    wait.unlock()
    guard let data = data, data.count <= capacity else { return -1 }
    data.copyBytes(to: bytes, count: data.count)
    return data.count
}
@_cdecl("dh_http_new")
public func httpNew(_ path: UnsafePointer<CChar>, _ boundary: UnsafePointer<CChar>, _ key: UnsafePointer<CChar>) -> UnsafeMutableRawPointer? {
    let net = Network()
    var request = URLRequest(url: URL(string: "https://api.mistral.ai/v1/audio/transcriptions")!)
    request.httpMethod = "POST"
    request.setValue("Bearer \(String(cString: key))", forHTTPHeaderField: "Authorization")
    request.setValue("multipart/form-data; boundary=\(String(cString: boundary))", forHTTPHeaderField: "Content-Type")
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    net.task = net.session.uploadTask(with: request, fromFile: URL(fileURLWithPath: String(cString: path)))
    net.task.resume()
    return Unmanaged.passRetained(net).toOpaque()
}
@_cdecl("dh_http_receive")
public func httpReceive(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafeMutablePointer<UInt8>, _ capacity: Int) -> Int {
    let net = network(pointer)
    net.condition.lock()
    while !net.completed && !net.closed { net.condition.wait() }
    defer { net.condition.unlock() }
    guard net.completed && !net.closed && !net.failed && net.response.count <= capacity else { return -1 }
    net.response.copyBytes(to: bytes, count: net.response.count)
    return net.response.count
}
@_cdecl("dh_net_close")
public func netClose(_ pointer: UnsafeMutableRawPointer) { network(pointer).close() }
@_cdecl("dh_net_free")
public func netFree(_ pointer: UnsafeMutableRawPointer) {
    network(pointer).close()
    Unmanaged<Network>.fromOpaque(pointer).release()
}

@_cdecl("dh_capture")
public func capture(_ context: UnsafeMutableRawPointer?,
             _ stopped: @convention(c) (UnsafeMutableRawPointer?) -> Int32,
             _ emit: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<UInt8>?, Int) -> Int32) -> Int32 {
    if AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined {
        let wait = DispatchSemaphore(value: 0)
        AVCaptureDevice.requestAccess(for: .audio) { _ in wait.signal() }
        while wait.wait(timeout: .now() + 0.05) == .timedOut {
            if stopped(context) != 0 { return 0 }
        }
    }
    guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else { return 2 }
    if stopped(context) != 0 { return 0 }
    let engine = AVAudioEngine()
    let input = engine.inputNode
    let source = input.outputFormat(forBus: 0)
    guard source.sampleRate > 0, source.channelCount > 0,
          let target = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16000, channels: 1, interleaved: true),
          let converter = AVAudioConverter(from: source, to: target) else { return 1 }
    let lock = NSLock()
    var failed = false
    var active = true
    // The callback emits <=100ms frames; warmup is generated by Rust and never enters the WAV.
    input.installTap(onBus: 0, bufferSize: 1024, format: source) { buffer, _ in
        lock.lock(); defer { lock.unlock() }
        if failed || !active { return }
        let capacity = AVAudioFrameCount(ceil(Double(buffer.frameLength) * 16000 / source.sampleRate) + 64)
        guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { failed = true; return }
        var supplied = false
        var error: NSError?
        let status = converter.convert(to: output, error: &error) { _, state in
            if supplied { state.pointee = .noDataNow; return nil }
            supplied = true; state.pointee = .haveData; return buffer
        }
        if error != nil || status == .error { failed = true; return }
        guard let data = output.int16ChannelData else { failed = true; return }
        let bytes = UnsafeRawPointer(data[0]).assumingMemoryBound(to: UInt8.self)
        let length = Int(output.frameLength) * 2
        var offset = 0
        while offset < length {
            let count = min(3200, length - offset)
            if emit(context, bytes.advanced(by: offset), count) == 0 { failed = true; break }
            offset += count
        }
    }
    do { try engine.start() } catch { input.removeTap(onBus: 0); return 1 }
    while true {
        lock.lock(); let stop = stopped(context) != 0 || failed; lock.unlock()
        if stop { break }
        Thread.sleep(forTimeInterval: 0.02)
    }
    lock.lock(); active = false; lock.unlock()
    engine.stop(); input.removeTap(onBus: 0)
    // Drain any in-flight tap before Rust drops its stack-owned callback context.
    lock.lock(); defer { lock.unlock() }
    return failed ? 1 : 0
}

// Event-posting permission is the permission CGEvent.post actually needs. Check it
// afresh: approval can change while Settings is open or after a local rebuild.
@_cdecl("dh_can_insert")
public func canInsert() -> Int32 { CGPreflightPostEventAccess() ? 1 : 0 }

@_cdecl("dh_output")
public func output(_ text: UnsafePointer<CChar>, _ mode: Int32) -> Int32 {
    let value = String(cString: text)
    if mode != 2 && canInsert() == 0 { return 2 }
    // Don't inherit held shortcut modifiers from the user's keyboard state.
    let source = CGEventSource(stateID: .privateState)
    if mode == 0 || mode == 2 {
        let board = NSPasteboard.general
        board.clearContents()
        guard board.setString(value, forType: .string) else { return 1 }
        if mode == 2 { return 0 }
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: false) else { return 1 }
        for event in [down, up] {
            event.flags = .maskCommand; event.setIntegerValueField(.eventSourceUserData, value: inputMark)
            event.post(tap: .cghidEventTap)
        }
    } else {
        let units = Array(value.utf16)
        var start = 0
        while start < units.count {
            var end = min(start + 20, units.count)
            if end < units.count && (0xD800...0xDBFF).contains(units[end - 1]) { end -= 1 }
            let chunk = Array(units[start..<end])
            start = end
            guard let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
                  let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else { return 1 }
            for event in [down, up] {
                event.flags = []
                event.keyboardSetUnicodeString(stringLength: chunk.count, unicodeString: chunk)
                event.setIntegerValueField(.eventSourceUserData, value: inputMark)
                event.post(tap: .cghidEventTap)
            }
        }
    }
    return 0
}

private func hotkey(_ value: String) -> (UInt32, UInt32)? {
    let codes: [String: UInt32] = [
        "A":0,"S":1,"D":2,"F":3,"H":4,"G":5,"Z":6,"X":7,"C":8,"V":9,"B":11,
        "Q":12,"W":13,"E":14,"R":15,"Y":16,"T":17,"1":18,"2":19,"3":20,"4":21,
        "6":22,"5":23,"9":25,"7":26,"8":28,"0":29,"O":31,"U":32,"I":34,"P":35,
        "L":37,"J":38,"K":40,"N":45,"M":46,
        "F1":122,"F2":120,"F3":99,"F4":118,"F5":96,"F6":97,"F7":98,"F8":100,
        "F9":101,"F10":109,"F11":103,"F12":111,"F13":105,"F14":107,"F15":113,
        "F16":106,"F17":64,"F18":79,"F19":80,"F20":90]
    var modifiers: UInt32 = 0
    var key: UInt32?
    for token in value.uppercased().split(separator: "+", omittingEmptySubsequences: false).map({ $0.trimmingCharacters(in: .whitespaces) }) {
        let flag: UInt32
        switch token {
        case "CTRL", "CONTROL": flag = UInt32(controlKey)
        case "ALT", "OPTION": flag = UInt32(optionKey)
        case "CMD", "COMMAND": flag = UInt32(cmdKey)
        case "SHIFT": flag = UInt32(shiftKey)
        default: flag = 0
        }
        if flag != 0 {
            if modifiers & flag != 0 { return nil }
            modifiers |= flag
        } else {
            guard key == nil, let code = codes[token] else { return nil }
            key = code
        }
    }
    guard modifiers != 0, let key = key else { return nil }
    // Command+V is reserved for our injected paste; Carbon hotkeys cannot inspect CGEvent tags.
    if key == 9 && modifiers == UInt32(cmdKey) { return nil }
    return (key, modifiers)
}
private final class Shell: NSObject, NSApplicationDelegate {
    let context: UnsafeMutableRawPointer?
    let action: Action
    var config: [String: Any]
    var item: NSStatusItem!
    var statusItem: NSMenuItem!
    var batchItem: NSMenuItem!
    var insertionItem: NSMenuItem!
    var insertionAllowed: Bool?
    var nextPermissionCheck = Date.distantPast
    var activeShortcut = ""
    var hotkeyRef: EventHotKeyRef?
    var handler: EventHandlerRef?
    var timer: Timer?
    var inAction = false
    var busy = false
    var dialog = false
    var logs: [String] = []
    var escapeMonitor: Any?
    var localMonitor: Any?
    var overlay: NSPanel?
    var overlayLabel: NSTextField?
    init(_ context: UnsafeMutableRawPointer?, _ action: @escaping Action, _ config: [String: Any]) {
        self.context = context; self.action = action; self.config = config
    }
    func invoke(_ code: Int32, _ text: String = "") {
        guard !inAction else { return }
        inAction = true; defer { inAction = false }
        text.withCString { action(context, code, $0) }
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        item.button?.title = ""
        if let url = Bundle.main.url(forResource: "menu-icon", withExtension: "png"),
           let icon = NSImage(contentsOf: url) {
            icon.size = NSSize(width: 18, height: 18)
            icon.isTemplate = false
            item.button?.image = icon
        } else {
            item.button?.image = NSImage(systemSymbolName: "mic.fill", accessibilityDescription: "Dictation Hotkey")
        }
        item.button?.imagePosition = .imageOnly
        item.button?.setAccessibilityLabel("Dictation Hotkey")
        item.button?.toolTip = "Dictation Hotkey"
        let menu = NSMenu()
        statusItem = NSMenuItem(title: "Ready", action: nil, keyEquivalent: "")
        menu.addItem(statusItem)
        add(menu, "Start / Stop Dictation", #selector(toggle))
        add(menu, "Record to Clipboard", #selector(clipboard))
        add(menu, "Stop Recording", #selector(stop))
        add(menu, "Copy Last Text", #selector(copyLast))
        menu.addItem(.separator())
        batchItem = add(menu, "Batch Mode", #selector(batch))
        batchItem.state = config["offline_mode"] as? Bool == true ? .on : .off
        add(menu, "Settings…", #selector(settings))
        add(menu, "Logs…", #selector(showLogs))
        insertionItem = add(menu, "Enable Text Insertion…", #selector(accessibility))
        refreshInsertionPermission()
        menu.addItem(.separator())
        add(menu, "Quit", #selector(quit))
        item.menu = menu
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, _, _ in
            if let shell = shell, !shell.dialog { shell.invoke(1) }
            return noErr
        }, 1, &spec, nil, &handler)
        if !register(config["hotkey_macos"] as? String ?? "Ctrl+Alt+D") {
            error("Cannot register the shortcut. Choose another shortcut in Settings.")
        }
        escapeMonitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { event in
            if event.keyCode == 53 && self.busy && !self.dialog { self.invoke(2) }
        }
        localMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            if event.keyCode == 53 && self.busy && !self.dialog { self.invoke(2) }
            return event
        }
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { _ in
            if Date() >= self.nextPermissionCheck {
                self.refreshInsertionPermission()
                self.nextPermissionCheck = Date().addingTimeInterval(1)
            }
            self.invoke(6)
        }
        if (config["api_key"] as? String ?? "").isEmpty { settings() }
    }
    @discardableResult func add(_ menu: NSMenu, _ title: String, _ selector: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: "")
        item.target = self; menu.addItem(item); return item
    }
    func register(_ value: String) -> Bool {
        guard let (key, modifiers) = hotkey(value) else { return false }
        if let current = hotkey(activeShortcut), current.0 == key && current.1 == modifiers && hotkeyRef != nil {
            activeShortcut = value; return true
        }
        var next: EventHotKeyRef?
        let id = EventHotKeyID(signature: 0x44484354, id: 1)
        guard RegisterEventHotKey(key, modifiers, id, GetApplicationEventTarget(), 0, &next) == noErr else { return false }
        if let previous = hotkeyRef { UnregisterEventHotKey(previous) }
        hotkeyRef = next; activeShortcut = value
        return true
    }
    func apply(_ next: [String: Any]) -> Bool {
        let oldHotkey = config["hotkey_macos"] as? String ?? "Ctrl+Alt+D"
        guard register(next["hotkey_macos"] as? String ?? "Ctrl+Alt+D") else {
            error("Invalid or unavailable shortcut. Use Control, Option, Command or Shift + a letter, digit or F1–F20.")
            return false
        }
        let oldLogin = config["start_at_login"] as? Bool ?? false
        let newLogin = next["start_at_login"] as? Bool ?? false
        if oldLogin != newLogin {
            do {
                if newLogin { try SMAppService.mainApp.register() }
                else { try SMAppService.mainApp.unregister() }
            } catch {
                _ = register(oldHotkey)
                self.error("Login item change failed. Run the installed .app from Applications and check System Settings > Login Items.")
                return false
            }
        }
        config = next
        batchItem?.state = config["offline_mode"] as? Bool == true ? .on : .off
        return true
    }
    func status(_ message: String, _ active: Bool) {
        busy = active
        statusItem?.title = message
        item?.button?.toolTip = "Dictation Hotkey: \(message)"
        if logs.last != message { logs.append(message) }
        if logs.count > 100 { logs.removeFirst() }
        if active {
            if overlay == nil {
                let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 440, height: 72),
                    styleMask: [.titled, .nonactivatingPanel], backing: .buffered, defer: false)
                panel.title = "Dictation Hotkey"
                panel.level = .floating
                panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
                panel.isReleasedWhenClosed = false
                let label = NSTextField(labelWithString: message)
                label.frame = NSRect(x: 12, y: 28, width: 320, height: 30)
                label.lineBreakMode = .byTruncatingTail
                panel.contentView?.addSubview(label)
                let stop = NSButton(title: "Stop", target: self, action: #selector(self.stop))
                stop.frame = NSRect(x: 344, y: 20, width: 80, height: 30)
                panel.contentView?.addSubview(stop)
                if let frame = NSScreen.main?.visibleFrame {
                    panel.setFrameOrigin(NSPoint(x: frame.midX - 220, y: frame.minY + 40))
                }
                overlay = panel; overlayLabel = label
            }
            overlayLabel?.stringValue = message
            overlay?.orderFrontRegardless()
        } else { overlay?.orderOut(nil) }
    }
    func error(_ message: String) {
        logs.append(message); if logs.count > 100 { logs.removeFirst() }
        let alert = NSAlert(); alert.messageText = "Dictation Hotkey"; alert.informativeText = message
        NSApp.activate(ignoringOtherApps: true); alert.runModal()
    }
    @objc func toggle() { if !dialog { invoke(1) } }
    @objc func clipboard() { if !dialog { invoke(8) } }
    @objc func stop() { invoke(2) }
    @objc func copyLast() { invoke(3) }
    @objc func batch() { invoke(4) }
    @objc func quit() {
        invoke(7)
        timer?.invalidate()
        if let ref = hotkeyRef { UnregisterEventHotKey(ref) }
        if let handler = handler { RemoveEventHandler(handler) }
        if let monitor = escapeMonitor { NSEvent.removeMonitor(monitor) }
        if let monitor = localMonitor { NSEvent.removeMonitor(monitor) }
        NSApp.stop(nil)
        NSApp.postEvent(NSEvent.otherEvent(with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0)!, atStart: false)
    }
    @objc func accessibility() {
        if canInsert() == 0 {
            _ = CGRequestPostEventAccess()
            if canInsert() == 0,
               let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
                NSWorkspace.shared.open(url)
            }
        }
        refreshInsertionPermission()
    }
    func refreshInsertionPermission() {
        let allowed = canInsert() != 0
        insertionItem?.state = allowed ? .on : .off
        insertionItem?.title = allowed ? "Text Insertion Enabled" : "Enable Text Insertion…"
        guard insertionAllowed != allowed else { return }
        insertionAllowed = allowed
        logs.append(allowed ? "Text insertion permission verified for the running app" :
            "Text insertion permission missing for the running app. Enable it in Accessibility; after a local rebuild, remove the old entry and add the current app again, then quit and reopen it.")
        if logs.count > 100 { logs.removeFirst() }
    }
    @objc func showLogs() {
        dialog = true; defer { dialog = false }
        let alert = NSAlert(); alert.messageText = "Recent Status and Errors"
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 540, height: 260))
        scroll.hasVerticalScroller = true
        let view = NSTextView(frame: scroll.bounds)
        view.isEditable = false; view.string = logs.joined(separator: "\n")
        scroll.documentView = view; alert.accessoryView = scroll
        NSApp.activate(ignoringOtherApps: true); alert.runModal()
    }
    @objc func settings() {
        if busy { error("Stop recording and wait for transcription before changing settings."); return }
        dialog = true; defer { dialog = false }
        let alert = NSAlert(); alert.messageText = "Dictation Hotkey Settings"
        alert.informativeText = "Both modes send audio to Mistral. Paste uses Command+V. Shortcut uses physical keys."
        alert.addButton(withTitle: "Save"); alert.addButton(withTitle: "Cancel")
        let stack = NSStackView(); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 8
        var fields: [String: NSTextField] = [:]
        for (key, title) in [("api_key", "Mistral API Key"), ("hotkey_macos", "Shortcut (e.g. Ctrl+Alt+D)"),
            ("model", "Realtime Model (blank = default)"), ("offline_model", "Batch Model (blank = default)"),
            ("base_url", "Realtime URL (blank = Mistral)")] {
            stack.addArrangedSubview(NSTextField(labelWithString: title))
            let field = key == "api_key" ? NSSecureTextField() : NSTextField()
            field.stringValue = config[key] as? String ?? ""
            field.widthAnchor.constraint(equalToConstant: 440).isActive = true
            fields[key] = field; stack.addArrangedSubview(field)
        }
        let typing = NSPopUpButton(); typing.addItems(withTitles: ["Paste (Command+V)", "Unicode Keystrokes"])
        typing.selectItem(at: config["typing_mode"] as? String == "keystrokes" ? 1 : 0)
        stack.addArrangedSubview(typing)
        let batch = NSButton(checkboxWithTitle: "Batch transcription", target: nil, action: nil)
        batch.state = config["offline_mode"] as? Bool == true ? .on : .off; stack.addArrangedSubview(batch)
        let login = NSButton(checkboxWithTitle: "Start at login (installed app only)", target: nil, action: nil)
        login.state = config["start_at_login"] as? Bool == true ? .on : .off; stack.addArrangedSubview(login)
        stack.frame = NSRect(x: 0, y: 0, width: 440, height: 400)
        alert.accessoryView = stack
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            var next: [String: Any] = [:]
            for (key, field) in fields { next[key] = field.stringValue }
            next["typing_mode"] = typing.indexOfSelectedItem == 1 ? "keystrokes" : "paste"
            next["offline_mode"] = batch.state == .on
            next["start_at_login"] = login.state == .on
            if let bytes = try? JSONSerialization.data(withJSONObject: next), let text = String(data: bytes, encoding: .utf8) { invoke(5, text) }
        }
    }
}
@_cdecl("dh_run")
public func run(_ context: UnsafeMutableRawPointer?, _ action: @escaping @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?) -> Void,
         _ config: UnsafePointer<CChar>) {
    let bytes = Data(String(cString: config).utf8)
    let settings = (try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any] ?? [:]
    shell = Shell(context, action, settings)
    let app = NSApplication.shared
    app.delegate = shell
    app.run()
    shell = nil
}
@_cdecl("dh_ui_status")
public func uiStatus(_ text: UnsafePointer<CChar>, _ busy: Int32) {
    shell?.status(String(cString: text), busy != 0)
}
@_cdecl("dh_ui_error")
public func uiError(_ text: UnsafePointer<CChar>) { shell?.error(String(cString: text)) }
@_cdecl("dh_ui_config")
public func uiConfig(_ config: UnsafePointer<CChar>) -> Int32 {
    guard let next = (try? JSONSerialization.jsonObject(with: Data(String(cString: config).utf8))) as? [String: Any] else { return 0 }
    return shell?.apply(next) == true ? 1 : 0
}
