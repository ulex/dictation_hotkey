# Compact, antialiased versions of the original green/red/amber status balls.
# Uses only Windows' built-in System.Drawing; no build/runtime dependency.
param([string]$OutputDirectory = (Join-Path $PSScriptRoot '..\resources'))
Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null

function New-IconFrame([int]$Size, [string]$Color) {
    $canvas = [Drawing.Bitmap]::new($Size * 4, $Size * 4)
    $g = [Drawing.Graphics]::FromImage($canvas)
    $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml($Color))
    # Original silhouette, with a one-pixel transparent margin at 16px.
    $margin = $Size / 4.0
    $g.FillEllipse($brush, $margin, $margin, $Size * 4 - 2 * $margin, $Size * 4 - 2 * $margin)
    $frame = [Drawing.Bitmap]::new($Size, $Size)
    $resize = [Drawing.Graphics]::FromImage($frame)
    $resize.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
    $resize.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $resize.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $resize.DrawImage($canvas, 0, 0, $Size, $Size)
    $stream = [IO.MemoryStream]::new()
    $frame.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
    $bytes = $stream.ToArray()
    $stream.Dispose(); $resize.Dispose(); $frame.Dispose()
    $brush.Dispose(); $g.Dispose(); $canvas.Dispose()
    return ,$bytes
}

foreach ($state in @('idle', 'recording', 'processing')) {
    $color = @{ idle = '#32AF6E'; recording = '#F04141'; processing = '#F5AF28' }[$state]
    # Native small-icon sizes for 100..300% DPI, plus a compact 64px shell frame.
    $sizes = @(16, 20, 24, 32, 40, 48, 64)
    $frames = @($sizes | ForEach-Object { New-IconFrame $_ $color })
    $file = [IO.File]::Create((Join-Path $OutputDirectory "tray-$state.ico"))
    $writer = [IO.BinaryWriter]::new($file)
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $writer.Write([byte]$sizes[$i]); $writer.Write([byte]$sizes[$i])
        $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$frames[$i].Length); $writer.Write([uint32]$offset)
        $offset += $frames[$i].Length
    }
    foreach ($frame in $frames) { $writer.Write([byte[]]$frame) }
    $writer.Dispose()
    Write-Host "Generated tray-$state.ico ($offset bytes)"
}
