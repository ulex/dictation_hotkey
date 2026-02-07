import os
import _sounddevice_data

# Locate PortAudio DLLs bundled with sounddevice
_pa_dir = os.path.join(os.path.dirname(_sounddevice_data.__file__), "portaudio-binaries")
_pa_binaries = [(os.path.join(_pa_dir, f), "_sounddevice_data/portaudio-binaries")
                for f in os.listdir(_pa_dir) if f.endswith(".dll")]

a = Analysis(
    ["main.py"],
    pathex=[],
    binaries=_pa_binaries,
    datas=[],
    hiddenimports=["_sounddevice_data"],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="DictationHotkey",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    icon=None,
)
