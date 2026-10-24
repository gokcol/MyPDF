#!/usr/bin/env python3
"""Remove obsolete MyPDF registrations from Finder's Open With menu.

Preview by default; --apply unregisters only noninstalled dev.mypdf.* bundles.
--apply --archive also makes verified ZIP backups and retires obsolete managed
app copies without deleting their contents, preventing Finder re-registration.
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shlex
import stat
import subprocess
import sys
import tempfile
import uuid

REGISTER = Path('/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister')
INSTALLED = Path('/Applications/MyPDF.app')
REPOSITORY = Path(__file__).resolve().parents[1]


def managed_roots(repository: Path = REPOSITORY, home: Path | None = None) -> list[Path]:
    """Only development outputs and our explicitly named app-backup directory."""
    home = home or Path.home()
    mirror = Path('/Volumes/OG/Works') / repository.name
    return [repository / 'artifacts', repository / 'dist', mirror / 'artifacts',
            mirror / 'dist', home / 'Library/Application Support/MyPDF Backups']


def within(path: Path, root: Path) -> bool:
    return path == root or root in path.parents


def no_symlink_ancestors(path: Path) -> bool:
    return not any(p.is_symlink() for p in (path, *path.parents))


def discover_bundles(roots: list[Path]) -> list[Path]:
    found: set[Path] = set()
    for root in roots:
        if not root.is_dir() or not no_symlink_ancestors(root):
            continue
        for current, directories, _ in os.walk(root, followlinks=False):
            retained = []
            for name in directories:
                path = Path(current) / name
                if path.is_symlink():
                    continue
                if path.suffix.lower() == '.app':
                    found.add(path)
                elif name not in {'.git', 'target', '__MACOSX'}:
                    retained.append(name)
            directories[:] = retained
    return sorted(found, key=str)


def process_commands() -> list[str]:
    return subprocess.run(['/bin/ps', '-axww', '-o', 'command='], check=True,
                          capture_output=True, text=True).stdout.splitlines()


def active_build(commands: list[str]) -> bool:
    # Fail closed while outputs may be replaced. This maintenance is run after
    # installation, never concurrently with a release/build/signing pipeline.
    patterns = [r'(?:^|/)(?:cargo|rustc)(?:\s|$)',
                r'scripts/(?:bundle-macos|release-macos)\.sh(?:\s|$)',
                r'(?:^|/)(?:codesign|notarytool|hdiutil)(?:\s|$)']
    return any(re.search(pattern, command) for command in commands for pattern in patterns)


def running_bundle_paths(commands: list[str]) -> set[Path]:
    paths = set()
    for command in commands:
        executable = command.strip().split('/Contents/MacOS/', 1)[0]
        if command.strip().startswith('/') and executable.endswith('.app'):
            paths.add(Path(executable))
    return paths


def tracked_files(path: Path, roots: list[Path]) -> bool:
    for root in roots:
        repository = root.parent
        if root.name not in {'artifacts', 'dist'} or not (repository / '.git').exists():
            continue
        if within(path, root):
            result = subprocess.run(['git', '-C', str(repository), 'ls-files', '-z', '--',
                                     str(path.relative_to(repository))], capture_output=True, check=True)
            if result.stdout:
                return True
    return False


def bundle_info(path: Path) -> dict:
    with (path / 'Contents/Info.plist').open('rb') as source:
        value = plistlib.load(source)
    if not isinstance(value, dict):
        raise ValueError('The app information plist is not a dictionary')
    return value


def archive_eligibility(path: Path, roots: list[Path], installed: Path,
                        commands: list[str]) -> tuple[bool, str]:
    if not path.is_dir() or path.suffix.lower() != '.app':
        return False, 'not an existing app directory'
    if not any(within(path, root) and path != root for root in roots):
        return False, 'outside managed output and backup roots'
    if not no_symlink_ancestors(path):
        return False, 'symlinked app or ancestor'
    if path.resolve() == installed.resolve() or (installed.exists() and os.path.samefile(path, installed)):
        return False, 'installed application'
    if any(path.resolve() == other.resolve() for other in running_bundle_paths(commands)):
        return False, 'running application'
    if active_build(commands):
        return False, 'a build, packaging or signing process is active'
    if tracked_files(path, roots):
        return False, 'contains tracked source'
    info = bundle_info(path)
    if not str(info.get('CFBundleIdentifier', '')).startswith('dev.mypdf.'):
        return False, 'not a MyPDF desktop bundle'
    executable = info.get('CFBundleExecutable')
    icon = info.get('CFBundleIconFile')
    if executable != 'MyPDF' or not isinstance(icon, str) or Path(icon).name != icon:
        return False, 'unrecognized executable or icon metadata'
    if not icon.endswith('.icns'):
        icon += '.icns'
    # These bundles contain just the known executable/icon/signature payload.
    # A document or any other unexpected file must never be removed with one.
    expected_files = {'Contents/Info.plist', 'Contents/MacOS/MyPDF',
                      f'Contents/Resources/{icon}', 'Contents/CodeResources',
                      'Contents/_CodeSignature/CodeResources', 'Contents/PkgInfo'}
    expected_directories = {'Contents', 'Contents/MacOS', 'Contents/Resources', 'Contents/_CodeSignature'}
    for current, directories, files in os.walk(path, followlinks=False):
        for name in [*directories, *files]:
            item = Path(current) / name
            relative = str(item.relative_to(path))
            metadata = item.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                if not within(item.resolve(), path.resolve()):
                    return False, f'external symlink: {relative}'
                if relative not in expected_files:
                    return False, f'unexpected symlink: {relative}'
            elif stat.S_ISDIR(metadata.st_mode):
                if relative not in expected_directories:
                    return False, f'unexpected directory: {relative}'
            elif stat.S_ISREG(metadata.st_mode):
                if relative not in expected_files or metadata.st_nlink != 1:
                    return False, f'unexpected file or hard link: {relative}'
            else:
                return False, f'special file: {relative}'
    if not (path / 'Contents/MacOS/MyPDF').is_file():
        return False, 'missing app executable'
    return True, ''


def acl(path: Path) -> list[str]:
    result = subprocess.run(['/bin/ls', '-lde', str(path)], check=True, capture_output=True, text=True)
    return [line.strip() for line in result.stdout.splitlines() if re.match(r'^\s*\d+:', line)]


def xattrs(path: Path) -> dict[str, str]:
    if hasattr(os, 'listxattr'):
        return {name: base64.b64encode(os.getxattr(path, name, follow_symlinks=False)).decode('ascii')
                for name in sorted(os.listxattr(path, follow_symlinks=False))}
    # Python exposes os.*xattr only on Linux. Use the documented Darwin libc
    # interface without following symlinks, keeping this tool dependency-free.
    library = ctypes.CDLL(None, use_errno=True)
    library.listxattr.argtypes = [ctypes.c_char_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
    library.listxattr.restype = ctypes.c_ssize_t
    library.getxattr.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p,
                                ctypes.c_size_t, ctypes.c_uint32, ctypes.c_int]
    library.getxattr.restype = ctypes.c_ssize_t
    encoded = os.fsencode(path)
    options = 1  # XATTR_NOFOLLOW
    size = library.listxattr(encoded, None, 0, options)
    if size < 0:
        raise OSError(ctypes.get_errno(), 'Cannot list extended attributes', str(path))
    names = ctypes.create_string_buffer(size)
    length = library.listxattr(encoded, names, size, options)
    if length < 0:
        raise OSError(ctypes.get_errno(), 'Cannot read attribute names', str(path))
    result = {}
    for name in sorted(filter(None, names.raw[:length].split(b'\0'))):
        size = library.getxattr(encoded, name, None, 0, 0, options)
        if size < 0:
            raise OSError(ctypes.get_errno(), 'Cannot size extended attribute', str(path))
        value = ctypes.create_string_buffer(size)
        length = library.getxattr(encoded, name, value, size, 0, options)
        if length < 0:
            raise OSError(ctypes.get_errno(), 'Cannot read extended attribute', str(path))
        result[os.fsdecode(name)] = base64.b64encode(value.raw[:length]).decode('ascii')
    return result


def tree_snapshot(path: Path) -> dict[str, dict]:
    snapshot = {}
    paths = [path]
    for current, directories, files in os.walk(path, followlinks=False):
        paths.extend(Path(current) / name for name in [*directories, *files])
    for item in sorted(paths):
        metadata = item.lstat()
        value = {'mode': stat.S_IMODE(metadata.st_mode), 'uid': metadata.st_uid,
                 'gid': metadata.st_gid, 'flags': getattr(metadata, 'st_flags', 0),
                 'mtime_ns': metadata.st_mtime_ns,
                 'birthtime_ns': getattr(metadata, 'st_birthtime_ns', None),
                 'acl': acl(item), 'xattrs': xattrs(item)}
        if stat.S_ISLNK(metadata.st_mode):
            value.update(kind='symlink', target=os.readlink(item))
        elif stat.S_ISDIR(metadata.st_mode):
            value['kind'] = 'directory'
        elif stat.S_ISREG(metadata.st_mode):
            with item.open('rb') as stream:
                digest = hashlib.file_digest(stream, 'sha256').hexdigest()
            value.update(kind='file', sha256=digest, bytes=metadata.st_size)
        else:
            raise ValueError(f'Unsupported file in bundle: {item}')
        snapshot[str(item.relative_to(path))] = value
    return snapshot


def archive_comparable(snapshot: dict[str, dict]) -> dict[str, dict]:
    # ZIP timestamps may have less precision than the source filesystem. The
    # precise source timestamps remain in the manifest; content, permissions,
    # symlink targets, flags, ownership, xattrs and ACLs must round-trip exactly.
    return {path: {key: value for key, value in fields.items() if key not in {'mtime_ns', 'birthtime_ns'}}
            for path, fields in snapshot.items()}


def archive_restore_comparison(before: dict, restored: dict) -> tuple[dict, dict, list[dict]]:
    original, copy = archive_comparable(before), archive_comparable(restored)
    exceptions = []
    original_root = original.get('.', {}).get('xattrs', {})
    copy_root = copy.get('.', {}).get('xattrs', {})
    # ditto may omit the OS-managed root access grant on ZIP extraction. Do
    # not waive changed MACL values, child MACLs, or any other attribute. The
    # original inode and every attribute remain in the inert directory, whose
    # later full-snapshot verification has NO exceptions.
    attribute = 'com.apple.macl'
    if attribute in original_root and attribute not in copy_root and '.' in copy:
        copy['.']['xattrs'] = dict(copy_root, **{attribute: original_root[attribute]})
        exceptions.append({'path': '.', 'attribute': attribute, 'change': 'missing from restored ZIP',
                           'preserved_in': 'retained original directory and source_snapshot'})
    return original, copy, exceptions


def code_signature(path: Path) -> dict:
    verify = subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(path)],
                            capture_output=True, text=True)
    return {'valid': verify.returncode == 0}


def archive_bundle(path: Path, roots: list[Path], installed: Path) -> dict:
    """Archive and disable discovery of a managed app, preserving all its bytes."""
    eligible, reason = archive_eligibility(path, roots, installed, process_commands())
    if not eligible:
        return {'path': str(path), 'archived': False, 'reason': reason}
    token = uuid.uuid4().hex[:12]
    archive = path.with_name(path.name + f'.backup-{token}.zip')
    manifest = archive.with_suffix(archive.suffix + '.manifest.json')
    partial = path.with_name(f'.mypdf-archive-{token}.zip')
    retired = path.with_name(path.name + f'.retired-{token}')
    report = {'path': str(path), 'archive': str(archive), 'manifest': str(manifest),
              'retired_directory': str(retired), 'archived': False}
    before = tree_snapshot(path)
    root_atime_ns = path.stat().st_atime_ns
    signature = code_signature(path)
    moved = False
    disabled = False
    try:
        subprocess.run(['/usr/bin/ditto', '-c', '-k', '--rsrc', '--extattr', '--acl', '--qtn',
                        '--sequesterRsrc', '--keepParent', str(path), str(partial)], check=True,
                       capture_output=True, text=True)
        with tempfile.TemporaryDirectory(prefix='mypdf-archive-check-') as temporary:
            subprocess.run(['/usr/bin/ditto', '-x', '-k', '--rsrc', '--extattr', '--acl', '--qtn',
                            str(partial), temporary], check=True, capture_output=True, text=True)
            restored = Path(temporary) / path.name
            after = tree_snapshot(restored)
            original, copy, metadata_exceptions = archive_restore_comparison(before, after)
            if original != copy:
                differences = [f"{name}: {', '.join(key for key in sorted(set(original.get(name, {})) | set(copy.get(name, {}))) if original.get(name, {}).get(key) != copy.get(name, {}).get(key))}"
                               for name in sorted(set(original) | set(copy)) if original.get(name) != copy.get(name)]
                raise ValueError('Archive restore changed content or protected metadata: ' + '; '.join(differences))
            if signature['valid'] and not code_signature(restored)['valid']:
                raise ValueError('The restored app did not retain its valid code signature')
        if before != tree_snapshot(path):
            raise ValueError('The original bundle changed during archival')
        eligible, reason = archive_eligibility(path, roots, installed, process_commands())
        if not eligible:
            raise ValueError(f'Bundle is no longer safe to archive: {reason}')
        with partial.open('rb') as stream:
            archive_digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        restore_folder = path.parent / f'MyPDF-restored-{token}'
        report.update(archive_sha256=archive_digest,
                      source_signature=signature, source_snapshot=before,
                      archive_metadata_exceptions=metadata_exceptions,
                      full_metadata_restore='Use the retained original directory and restore_retired_steps for all original metadata; ZIP restoration has only the explicitly listed portable-metadata exceptions.',
                      restore_command=shlex.join(['/usr/bin/ditto', '-x', '-k', '--rsrc', '--extattr',
                                                 '--acl', '--qtn', str(archive), str(restore_folder)]),
                      restore_retired_steps=[
                          f'Only proceed while the original path is absent: {path}',
                          f'Rename {retired / "ArchivedContents"} to {retired / "Contents"}.',
                          f'Rename {retired} to {path}.',
                          'If the second rename fails, rename Contents back to ArchivedContents.'
                      ],
                      timestamp_note='ZIP timestamps may have reduced precision; original nanoseconds are recorded above.')
        # Publish and fsync the verified backup/manifest before retiring the
        # exposed app. Never modify Info.plist or re-sign the preserved copy.
        os.rename(partial, archive)
        manifest.write_text(json.dumps(report, indent=2) + '\n')
        for saved in (archive, manifest):
            with saved.open('rb') as stream:
                os.fsync(stream.fileno())
        # Finder may track moved directory inodes. Remove both original and
        # retired registrations explicitly, without resetting its database.
        unregister = subprocess.run([str(REGISTER), '-u', str(path)], capture_output=True, text=True)
        report['unregister_original_exit_code'] = unregister.returncode
        os.rename(path, retired)
        moved = True
        os.rename(retired / 'Contents', retired / 'ArchivedContents')
        disabled = True
        os.utime(retired, ns=(root_atime_ns, before['.']['mtime_ns']))
        retained = tree_snapshot(retired)
        retained = {re.sub(r'^ArchivedContents(?=/|$)', 'Contents', name): fields
                    for name, fields in retained.items()}
        if before != retained:
            raise ValueError('Retirement changed original payload or protected metadata')
        # Keep every original byte in the inert directory as well as in ZIP.
        # The absent Contents layout prevents recognizing this as an app even
        # if Finder inspects it. Signed payload files themselves never change.
        unregister = subprocess.run([str(REGISTER), '-u', str(retired)], capture_output=True, text=True)
        report['unregister_retired_exit_code'] = unregister.returncode
        report['archived'] = True
        manifest.write_text(json.dumps(report, indent=2) + '\n')
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        # These are only directory renames, so either operation can be undone
        # without reconstructing or deleting any original payload file.
        try:
            if disabled:
                os.rename(retired / 'ArchivedContents', retired / 'Contents')
            if moved:
                os.rename(retired, path)
                os.utime(path, ns=(root_atime_ns, before['.']['mtime_ns']))
        except OSError as rollback_error:
            report['rollback_error'] = str(rollback_error)
            report['original_payload_preserved_at'] = str(retired if retired.exists() else path)
        report['archived'] = False
        report['reason'] = str(error)
        if manifest.exists():
            manifest.write_text(json.dumps(report, indent=2) + '\n')
    finally:
        if partial.exists():
            partial.unlink()
    return report


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


def unrelated_removals(before: list[dict], after: list[dict]) -> tuple[list[tuple], list[tuple]]:
    foreign = lambda records: {(record['identifier'], record['path']) for record in records
                               if not record['identifier'].startswith('dev.mypdf.')}
    removed = sorted(foreign(before) - foreign(after))
    # Launch Services may independently expire records for unavailable volumes
    # or deleted apps. Keep the full audit, but alert for existing apps only.
    existing = [record for record in removed if Path(record[1]).exists()]
    return removed, existing


def dump_database() -> str:
    return subprocess.run([str(REGISTER), '-dump'], capture_output=True, text=True, check=True).stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true', help='Unregister the previewed copies; do not delete any files')
    parser.add_argument('--archive', action='store_true',
                        help='With --apply, archive obsolete managed app copies after verified restoration')
    parser.add_argument('--report', type=Path, help='Save before/after MyPDF registrations as JSON')
    args = parser.parse_args()
    if args.archive and not args.apply:
        parser.error('--archive requires --apply; the default preview does not change files')
    if sys.platform != 'darwin' or not REGISTER.is_file():
        parser.error('This maintenance command is for macOS.')
    info = plistlib.loads((INSTALLED / 'Contents/Info.plist').read_bytes())
    if info.get('CFBundleIdentifier') != 'dev.mypdf.desktop':
        parser.error('The installed MyPDF could not be verified; no registrations changed.')
    before = registrations(dump_database())
    if not any(r['path'] == str(INSTALLED) and r['identifier'] == 'dev.mypdf.desktop' for r in before):
        parser.error('The installed app is not visible in Launch Services; run from your normal user session.')
    selected = candidates(before, INSTALLED)
    commands = process_commands() if args.apply else []
    if args.archive and active_build(commands):
        parser.error('Finish active build, packaging and signing processes before archiving app copies')
    running = running_bundle_paths(commands)
    protected_running = [path for path in selected if path in running]
    selected = [path for path in selected if path not in running]
    report = {
        'installed_app': str(INSTALLED), 'applied': args.apply,
        'selected_paths': [str(path) for path in selected],
        'protected_running_paths': [str(path) for path in protected_running],
        'before': [r for r in before if r['identifier'].startswith('dev.mypdf.')],
    }
    print(f"{'Unregistering' if args.apply else 'Would unregister'} {len(selected)} noninstalled MyPDF paths.")
    if args.apply:
        if args.archive:
            roots = managed_roots()
            archive_paths = sorted(set(selected) | set(discover_bundles(roots)), key=str)
            report['managed_archive_roots'] = [str(root) for root in roots]
            report['archives'] = []
            for path in archive_paths:
                if not path.exists():
                    continue
                try:
                    result = archive_bundle(path, roots, INSTALLED)
                except (OSError, ValueError, subprocess.CalledProcessError) as error:
                    result = {'path': str(path), 'archived': False, 'reason': str(error)}
                report['archives'].append(result)
            print(f"Archived {sum(result['archived'] for result in report['archives'])} verified managed app copies.")
            report['archive_failures'] = [result for result in report['archives']
                                          if 'archive' in result and not result['archived']]
            print(f"Archive verification failures: {len(report['archive_failures'])}; originals retained.")
            # Include discovered copies that Finder had not registered yet.
            selected = sorted(set(selected) | {Path(result[key]) for result in report['archives']
                                               if result['archived']
                                               for key in ('path', 'retired_directory')}, key=str)
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
        # Renamed bundle records can follow the original inode. Inspect every
        # path in our namespace, including ones no longer ending in .app.
        remaining = sorted({Path(record['path']) for record in after
                            if record['identifier'].startswith('dev.mypdf.')
                            and Path(record['path']) != INSTALLED}, key=str)
        report['remaining_paths'] = [str(path) for path in remaining]
        report['remaining_existing_paths'] = [str(path) for path in remaining if path.is_dir() and path not in running]
        all_removed, existing_removed = unrelated_removals(before, after)
        report['unrelated_registrations_removed'] = all_removed
        report['unrelated_existing_registrations_removed'] = existing_removed
        print(f"Remaining existing noninstalled MyPDF paths: {len(report['remaining_existing_paths'])}")
        print('Installed app retained. Other applications and user data were not modified.')
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    if args.apply and (report['remaining_existing_paths'] or report['unrelated_existing_registrations_removed']
                       or report.get('archive_failures')):
        print('Inspect the report: some registrations need further attention.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
