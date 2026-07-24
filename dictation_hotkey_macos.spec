# macOS .app bundle spec.
# Build with: pyinstaller dictation_hotkey_macos.spec

a = Analysis(
    ["main.py"],
    pathex=[],
    binaries=[],
    datas=[],
    hiddenimports=[
        "_cffi_backend",
        # Native macOS provider is imported lazily in main.py
        "speech_macos",
        "objc",
        "Foundation",
        "Quartz",
        "CoreFoundation",
        "AppKit",
        "AVFoundation",
        "Speech",
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    # Windows-only modules that static analysis would otherwise follow
    excludes=["keyboard", "winsound"],
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="DictationHotkey",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="DictationHotkey",
)

app = BUNDLE(
    coll,
    name="DictationHotkey.app",
    icon=None,
    bundle_identifier="com.dictationhotkey.app",
    info_plist={
        "CFBundleName": "Dictation Hotkey",
        "CFBundleDisplayName": "Dictation Hotkey",
        # Menu-bar app: no Dock icon, no menu bar
        "LSUIElement": True,
        "NSHighResolutionCapable": True,
        "NSMicrophoneUsageDescription":
            "Dictation Hotkey needs microphone access to transcribe your speech.",
        "NSSpeechRecognitionUsageDescription":
            "Dictation Hotkey uses on-device speech recognition to transcribe your voice.",
    },
)
