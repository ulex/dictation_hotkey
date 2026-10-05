#!/usr/bin/env python3
"""Regenerate compact macOS icons using ImageMagick and Apple's iconutil."""
from pathlib import Path
import subprocess
import tempfile


def main():
    resources = Path(__file__).resolve().parent.parent / 'resources' / 'macos'
    source = resources / 'app-icon.png'
    subprocess.run(['magick', str(source), '-resize', '36x36', '-strip',
                    '+dither', '-colors', '256', f'PNG8:{resources / "menu-icon.png"}'], check=True)
    with tempfile.TemporaryDirectory(prefix='dictation-icon-') as temporary:
        iconset = Path(temporary) / 'DictationHotkey.iconset'
        iconset.mkdir()
        for size in [16, 32, 128, 256, 512]:
            for scale in [1, 2]:
                pixels = size * scale
                suffix = '@2x' if scale == 2 else ''
                destination = iconset / f'icon_{size}x{size}{suffix}.png'
                subprocess.run(['magick', str(source), '-resize', f'{pixels}x{pixels}',
                                '-strip', '+dither', '-colors', '256', f'PNG8:{destination}'], check=True)
        subprocess.run(['iconutil', '-c', 'icns', str(iconset),
                        '-o', str(resources / 'DictationHotkey.icns')], check=True)


if __name__ == '__main__':
    main()
