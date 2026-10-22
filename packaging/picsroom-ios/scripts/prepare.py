#!/usr/bin/env python3
"""Fetch reproducible build inputs; never include personal data or credentials."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]
PIN = '8d840eab5962b41e4e15b115107eef8384648fb5'

def main():
    vendor = ROOT / 'vendor/photocraft'
    if not vendor.exists():
        # The archive is created by Git from this public repository's immutable commit.
        with tempfile.TemporaryDirectory(dir=ROOT) as scratch:
            archive = Path(scratch) / 'engine.tar'
            subprocess.run(['git', '-C', str(REPO), 'archive', PIN, '-o', str(archive)], check=True)
            unpacked = Path(scratch) / 'engine'
            unpacked.mkdir()
            subprocess.run(['tar', '-xf', str(archive), '-C', str(unpacked)], check=True)
            vendor.parent.mkdir(exist_ok=True)
            unpacked.rename(vendor)
    models = ROOT / 'App/Models'
    for entry in json.loads((models / 'provenance.json').read_text()):
        destination = models / entry['file']
        if destination.exists() and hashlib.sha256(destination.read_bytes()).hexdigest() == entry['sha256']:
            continue
        url = 'https://huggingface.co/{repo}/resolve/{revision}/{file}'.format(**entry)
        print('Downloading', entry['file'], flush=True)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as tmp:
            temporary = Path(tmp.name)
            try:
                digest = hashlib.sha256()
                with urllib.request.urlopen(url, timeout=60) as response:
                    if not response.url.startswith('https://'):
                        raise ValueError('Model download must use HTTPS')
                    total = 0
                    while True:
                        block = response.read(1024 * 1024)
                        if not block:
                            break
                        total += len(block)
                        if total > 200 * 1024 * 1024:
                            raise ValueError('Model file exceeds 200 MB')
                        digest.update(block)
                        tmp.write(block)
                if digest.hexdigest() != entry['sha256']:
                    raise ValueError('Checksum mismatch: ' + entry['file'])
                tmp.close()
                temporary.replace(destination)
            finally:
                temporary.unlink(missing_ok=True)
    print('Pinned engine and verified model packages are ready.')

if __name__ == '__main__':
    main()
