#!/usr/bin/env python3
"""Version generated scripts by their final bytes, including wasm URL substitutions."""
import hashlib
from pathlib import Path
import re
import sys


def finalize(directory: Path) -> None:
    index = directory / 'index.html'
    content = index.read_text()

    def version(match: re.Match) -> str:
        url = match.group(2).split('?', 1)[0]
        script = directory / url.lstrip('/')
        if not script.is_file():
            raise ValueError(f'Generated script not found: {url}')
        digest = hashlib.sha256(script.read_bytes()).hexdigest()[:20]
        return f'{match.group(1)}{url}?v={digest}{match.group(3)}'

    content, count = re.subn(r'((?:src|href)=")(/?(?:\./)?assets/[^"?]+\.js(?:\?[^"]*)?)(")', version, content)
    if count == 0:
        raise ValueError('No generated scripts found in index.html')
    index.write_text(content)


if __name__ == '__main__':
    finalize(Path(sys.argv[1]))
