# Sample a process (including child processes) without modifying its working set.
# Usage: .\sample-process.ps1 -ProcessName DictationHotkey -Seconds 300 -CsvPath idle.csv
param(
    [Parameter(Mandatory=$true)][string]$ProcessName,
    [int]$Seconds = 300,
    [string]$CsvPath = 'memory.csv'
)
$rows = for ($i = 0; $i -lt $Seconds; $i++) {
    $processes = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue)
    foreach ($p in $processes) {
        try {
            $p.Refresh()
            [PSCustomObject]@{
                TimeUtc = (Get-Date).ToUniversalTime().ToString('o')
                Id = $p.Id
                Name = $p.ProcessName
                PrivateBytes = $p.PrivateMemorySize64
                WorkingSetBytes = $p.WorkingSet64
                Threads = $p.Threads.Count
                Handles = $p.HandleCount
                CpuSeconds = $p.CPU
            }
        } catch { Write-Warning "Process exited during sampling: $($_.Exception.Message)" }
    }
    Start-Sleep -Seconds 1
}
$rows | Export-Csv -Path $CsvPath -NoTypeInformation
Write-Host "Wrote $($rows.Count) observations to $CsvPath. Record OS/build, device, model, session steps, artifact hash and process-tree relationships separately."
