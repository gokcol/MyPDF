#!/usr/bin/env python3
"""Remove obsolete MyPDF registrations from Finder's Open With menu.

Preview by default; --apply unregisters only noninstalled dev.mypdf.* bundles.
App bundles, signed backups, PDFs, user data and other applications are untouched.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import plistlib
import re
import subprocess
import sys

REGISTER = Path('/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister')
INSTALLED = Path('/Applications/MyPDF.app')


def registrations(dump: str) -> list[dict[str, str]]:
    records = []
    for block in re.split(r'\n-{20,}\n', dump):
        if not block.strip().startswith('bundle id:'):
            continue
        fields = dict(re.findall(r'^(path|name|identifier|versionString):\s*(.*?)\s*$', block, re.MULTILINE))
        if fields.get('path') and fields.get('identifier'):
            fields['path'] = re.sub(r' \(0x[0-9a-f]+\)$', '', fields['path'])
            records.append(fields)
    return records


def candidates(records: list[dict[str, str]], installed: Path) -> list[Path]:
    # Match our bundle namespace, not app display names or PDF bindings. In
    # particular, retain other MyPDF products and Windows/Parallels launchers.
    return sorted({
        Path(record['path']) for record in records
        if record['identifier'].startswith('dev.mypdf.')
        and Path(record['path']) != installed
        and Path(record['path']).suffix.lower() == '.app'
    }, key=str)


def dump_database() -> str:
    return subprocess.run([str(REGISTER), '-dump'], capture_output=True, text=True, check=True).stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true', help='Unregister the previewed copies; do not delete any files')
    parser.add_argument('--report', type=Path, help='Save before/after MyPDF registrations as JSON')
    args = parser.parse_args()
    if sys.platform != 'darwin' or not REGISTER.is_file():
        parser.error('This maintenance command is for macOS.')
    info = plistlib.loads((INSTALLED / 'Contents/Info.plist').read_bytes())
    if info.get('CFBundleIdentifier') != 'dev.mypdf.desktop':
        parser.error('The installed MyPDF could not be verified; no registrations changed.')
    before = registrations(dump_database())
    if not any(r['path'] == str(INSTALLED) and r['identifier'] == 'dev.mypdf.desktop' for r in before):
        parser.error('The installed app is not visible in Launch Services; run from your normal user session.')
    selected = candidates(before, INSTALLED)
    report = {
        'installed_app': str(INSTALLED), 'applied': args.apply,
        'selected_paths': [str(path) for path in selected],
        'before': [r for r in before if r['identifier'].startswith('dev.mypdf.')],
    }
    print(f"{'Unregistering' if args.apply else 'Would unregister'} {len(selected)} noninstalled MyPDF paths.")
    if args.apply:
        # Exact paths only. Never reset/delete the Launch Services database or
        # change the user's default PDF handler. Re-register the installed copy
        # last so the shared production identifier resolves to the current app.
        if selected:
            result = subprocess.run([str(REGISTER), '-u', *map(str, selected)], capture_output=True, text=True)
            report['unregister_exit_code'] = result.returncode
            report['unregister_output'] = result.stdout + result.stderr
        subprocess.run([str(REGISTER), '-f', str(INSTALLED)], capture_output=True, text=True, check=True)
        after = registrations(dump_database())
        report['after'] = [r for r in after if r['identifier'].startswith('dev.mypdf.')]
        remaining = candidates(after, INSTALLED)
        report['remaining_paths'] = [str(path) for path in remaining]
        report['remaining_existing_paths'] = [str(path) for path in remaining if path.is_dir()]
        foreign = lambda records: {(r['identifier'], r['path']) for r in records if not r['identifier'].startswith('dev.mypdf.')}
        report['unrelated_registrations_removed'] = sorted(foreign(before) - foreign(after))
        print(f"Remaining existing noninstalled MyPDF paths: {len(report['remaining_existing_paths'])}")
        print('Installed app retained. No app bundles or user files were modified.')
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    if args.apply and (report['remaining_existing_paths'] or report['unrelated_registrations_removed']):
        print('Inspect the report: some registrations need further attention.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
