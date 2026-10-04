# Launch the native release, dismiss first-run Settings without saving, sample idle
# memory, then request coordinated shutdown. Never changes credentials/settings.
param(
    [string]$ExePath = "$PSScriptRoot\..\native\target\x86_64-pc-windows-msvc\release\dictation-hotkey-native.exe",
    [ValidateRange(1, 3600)][int]$Seconds = 30,
    [string]$OutputDirectory = "$PSScriptRoot\..\benchmarks\windows"
)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DictationProbe {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    private static extern IntPtr FindWindow(string cls, string title);
    public static IntPtr FindByClass(string cls) { return FindWindow(cls, null); }
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr hwnd, uint msg, UIntPtr wp, IntPtr lp);
}
'@
if ([DictationProbe]::FindByClass('DictationHotkeyNativeController') -ne [IntPtr]::Zero) {
    throw 'Close the existing native app before measurement.'
}
$exe = (Resolve-Path $ExePath).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$process = Start-Process $exe -PassThru
try {
    $deadline = (Get-Date).AddSeconds(10)
    do {
        $controller = [DictationProbe]::FindByClass('DictationHotkeyNativeController')
        if ((Get-Date) -gt $deadline -or $process.HasExited) { throw 'Controller failed to start.' }
        Start-Sleep -Milliseconds 50
    } while ($controller -eq [IntPtr]::Zero)
    Start-Sleep -Seconds 1
    $settings = [DictationProbe]::FindByClass('DictationHotkeyNativeSettings')
    if ($settings -ne [IntPtr]::Zero) {
        [void][DictationProbe]::PostMessage($settings, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
    }
    Start-Sleep -Seconds 2
    $os = Get-CimInstance Win32_OperatingSystem
    $modules = @($process.Modules | ForEach-Object FileName)
    $metadata = [ordered]@{
        TimeUtc = (Get-Date).ToUniversalTime().ToString('o')
        OS = $os.Caption
        Version = $os.Version
        Build = $os.BuildNumber
        ExePath = $exe
        ExeBytes = (Get-Item $exe).Length
        SHA256 = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
        Action = 'Idle tray, Settings dismissed without saving; no capture/network session'
        Modules = $modules
    }
    $metadata | ConvertTo-Json -Depth 3 | Set-Content "$OutputDirectory\idle-metadata.json" -Encoding UTF8
    $rows = @(for ($i = 0; $i -lt $Seconds; $i++) {
        $process.Refresh()
        if ($process.HasExited) { throw 'App exited during idle measurement.' }
        [PSCustomObject]@{
            TimeUtc = (Get-Date).ToUniversalTime().ToString('o')
            Id = $process.Id
            PrivateBytes = $process.PrivateMemorySize64
            WorkingSetBytes = $process.WorkingSet64
            Threads = $process.Threads.Count
            Handles = $process.HandleCount
            CpuSeconds = $process.CPU
        }
        Start-Sleep -Seconds 1
    })
    $rows | Export-Csv "$OutputDirectory\idle.csv" -NoTypeInformation -Encoding UTF8
    $rows | Measure-Object PrivateBytes,WorkingSetBytes,Threads,Handles -Minimum -Maximum -Average |
        Format-Table Property,Minimum,Maximum,Average
    [void][DictationProbe]::PostMessage($controller, 0x111, ([UIntPtr]::new(5)), [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) { throw 'Coordinated shutdown timed out.' }
    if ($process.ExitCode -ne 0) { throw "App exit code: $($process.ExitCode)" }
    Write-Host 'Idle measurement and coordinated shutdown passed.'
} finally {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
    $process.Dispose()
}
