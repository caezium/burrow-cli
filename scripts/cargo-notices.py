#!/usr/bin/env python3
"""Copy or verify original notices for every locked Cargo dependency and target.
Run after `cargo fetch --locked`: python3 scripts/cargo-notices.py [--check].
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.request

# These crates omit license files from their published archives. Each current
# notice is pinned to the archive's .cargo_vcs_info revision. The earlier full
# MIT text preserves the upstream copyright removed when LICENSE.md replaced it.
OBJC_REVISIONS = {
    ('objc2', '0.6.4'): '8852b424193ca41602281b3d7540d7c8ed51e49a',
    ('objc2-encode', '4.1.0'): '8d214f5477365ffcbcbb7de058c86ed9a518efb7',
    ('objc2-foundation', '0.3.2'): '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b',
}
OBJC_NOTICE_HASH = '7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54'
OBJC_MIT_REVISION = '9961247c1a82027d6edbe6c516011b1363b9354c'
OBJC_MIT_HASH = 'e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5'
NOTICE_PREFIXES = ('LICENSE', 'LICENCE', 'COPYING', 'COPYRIGHT', 'NOTICE', 'UNLICENSE')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='verify without writing or fetching supplemental notices')
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    root = args.root.resolve()
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=root))
    inventory = []
    downloads = {}
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['id'] in metadata['workspace_members']:
            continue
        source = Path(package['manifest_path']).parent
        target = root / 'LICENSES' / 'cargo' / (package['name'] + '-' + package['version'])
        candidates = {p for p in source.iterdir() if p.is_file() and p.name.upper().startswith(NOTICE_PREFIXES)}
        if package.get('license_file'):
            candidates.add(source / package['license_file'])
        texts = []
        for path in sorted(candidates):
            destination = target / path.relative_to(source)
            original = path.read_bytes()
            if args.check:
                if not destination.is_file() or destination.read_bytes() != original:
                    raise ValueError('Missing or changed original notice: ' + str(destination.relative_to(root)))
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(original)
            texts.append({'file': destination.relative_to(root).as_posix(), 'sha256': digest(original)})

        revision = OBJC_REVISIONS.get((package['name'], package['version']))
        if revision:
            vcs = json.loads((source / '.cargo_vcs_info.json').read_text())
            if vcs['git']['sha1'] != revision or package['license'] != 'MIT':
                raise ValueError('objc2 source/license changed; audit supplemental notices again')
            for filename, commit, upstream_file, expected in (
                ('LICENSE.md', revision, 'LICENSE.md', OBJC_NOTICE_HASH),
                ('LICENSE-MIT-original.txt', OBJC_MIT_REVISION, 'LICENSE.txt', OBJC_MIT_HASH),
            ):
                url = 'https://raw.githubusercontent.com/madsmtm/objc2/' + commit + '/' + upstream_file
                destination = target / filename
                if args.check:
                    original = destination.read_bytes()
                else:
                    if expected not in downloads:
                        with urllib.request.urlopen(url, timeout=30) as response:
                            downloads[expected] = response.read()
                    original = downloads[expected]
                if digest(original) != expected:
                    raise ValueError('Supplemental notice hash mismatch: ' + filename)
                if not args.check:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes(original)
                texts.append({'file': destination.relative_to(root).as_posix(), 'sha256': expected, 'source': url})
        if not texts:
            raise ValueError('No original license text found: ' + package['name'] + ' ' + package['version'])
        record = {key: package[key] for key in ('name', 'version', 'license', 'repository')}
        record['texts'] = sorted(texts, key=lambda item: item['file'])
        inventory.append(record)

    path = root / 'LICENSES' / 'cargo-packages.json'
    if args.check:
        if not path.is_file() or json.loads(path.read_text()) != inventory:
            raise ValueError('Notice inventory differs from locked dependencies; regenerate and audit it')
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(inventory, indent=2) + '\n')
    print('Cargo notices verified' if args.check else 'Cargo notices copied',
          'for', len(inventory), 'packages and', sum(len(p['texts']) for p in inventory), 'original texts')


if __name__ == '__main__':
    main()
