# Rebuild the application/tray icons with only Windows' built-in System.Drawing.
# Each ICO contains independently rendered, antialiased 16..256px PNG frames.
param([string]$OutputDirectory = (Join-Path $PSScriptRoot '..\resources'))
Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null

function New-IconFrame([int]$Size, [string]$Color, [string]$State) {
    $scale = 4
    $canvas = [Drawing.Bitmap]::new($Size * $scale, $Size * $scale)
    $g = [Drawing.Graphics]::FromImage($canvas)
    $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.ScaleTransform($Size * $scale / 64.0, $Size * $scale / 64.0)
    $path = [Drawing.Drawing2D.GraphicsPath]::new()
    # Soft rounded-square silhouette, with generous transparent edge padding.
    $path.AddArc(4, 4, 24, 24, 180, 90)
    $path.AddArc(36, 4, 24, 24, 270, 90)
    $path.AddArc(36, 36, 24, 24, 0, 90)
    $path.AddArc(4, 36, 24, 24, 90, 90)
    $path.CloseFigure()
    $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml($Color))
    $white = [Drawing.SolidBrush]::new([Drawing.Color]::White)
    $pen = [Drawing.Pen]::new([Drawing.Color]::White, 3.5)
    $pen.StartCap = $pen.EndCap = [Drawing.Drawing2D.LineCap]::Round
    $g.FillPath($brush, $path)
    # Microphone capsule and rounded cradle (not an exclamation mark).
    $mic = [Drawing.Drawing2D.GraphicsPath]::new()
    $mic.AddArc(25, 14, 14, 14, 180, 180)
    $mic.AddArc(25, 24, 14, 14, 0, 180)
    $mic.CloseFigure()
    $g.FillPath($white, $mic)
    $g.DrawArc($pen, 20, 23, 24, 22, 0, 180)
    $g.DrawLine($pen, 20, 29, 20, 34)
    $g.DrawLine($pen, 44, 29, 44, 34)
    $g.DrawLine($pen, 32, 45, 32, 50)
    $g.DrawLine($pen, 26, 50, 38, 50)
    if ($State -eq 'recording') {
        $g.FillEllipse($white, 47, 12, 6, 6)
    } elseif ($State -eq 'processing') {
        $g.DrawLine($pen, 48, 15, 53, 15)
        $g.DrawLine($pen, 50.5, 12.5, 50.5, 17.5)
    }
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
    $mic.Dispose(); $pen.Dispose(); $white.Dispose(); $brush.Dispose()
    $path.Dispose(); $g.Dispose(); $canvas.Dispose()
    return ,$bytes
}

foreach ($state in @('idle', 'recording', 'processing')) {
    $color = @{ idle = '#4263EB'; recording = '#E5484D'; processing = '#8B5CF6' }[$state]
    $sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
    $frames = @($sizes | ForEach-Object { New-IconFrame $_ $color $state })
    $file = [IO.File]::Create((Join-Path $OutputDirectory "tray-$state.ico"))
    $writer = [IO.BinaryWriter]::new($file)
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $dimension = if ($sizes[$i] -eq 256) { 0 } else { $sizes[$i] }
        $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
        $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$frames[$i].Length); $writer.Write([uint32]$offset)
        $offset += $frames[$i].Length
    }
    foreach ($frame in $frames) { $writer.Write([byte[]]$frame) }
    $writer.Dispose()
    Write-Host "Generated tray-$state.ico ($($sizes -join ', ') px)"
}
