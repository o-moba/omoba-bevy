#!/usr/bin/env python3
"""Xcode build phase: compile Rust and stage only engine-owned app contents.

Xcode owns Info.plist processing, icons, signing and archive/export. No credentials
are read here. The existing device builder remains an independent CLI workflow.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

from build_device import (ROOT, app_info, inspect_macho, ios_build_environment,
                          sha256, tracked_assets, validate_dsym)
from package_licenses import collect_legal_notices, copy_legal_notices
from prepare_testflight import validate_build_number


def settings(env: dict[str, str]) -> dict:
    if env.get('PLATFORM_NAME') != 'iphoneos':
        raise ValueError('Choose Any iOS Device (arm64) or a physical iPhone. Use build_simulator.py for Simulator.')
    if env.get('ARCHS', 'arm64').split() != ['arm64']:
        raise ValueError('OMOBA device builds require arm64 only.')
    profile = env.get('OMOBA_CARGO_PROFILE', 'dev')
    if profile not in ('dev', 'release'):
        raise ValueError('OMOBA_CARGO_PROFILE must be dev or release.')
    build = validate_build_number(env['CURRENT_PROJECT_VERSION'])
    # Resolve from this checked-in script, not an external working directory.
    bundle = Path(env['TARGET_BUILD_DIR']) / env['FULL_PRODUCT_NAME']
    executable = Path(env['EXECUTABLE_PATH'])
    if bundle.suffix != '.app' or executable.parts != (bundle.name, 'client'):
        raise ValueError('Expected a single Omoba application with executable client.')
    return dict(profile=profile, build=build, bundle=bundle,
                cache=Path(env.get('OMOBA_CARGO_TARGET_DIR') or ROOT / 'target/iphone-cargo').expanduser().resolve(),
                derived=Path(env['DERIVED_FILE_DIR']),
                symbols=Path(env['DWARF_DSYM_FOLDER_PATH']) / env['DWARF_DSYM_FILE_NAME'])


def source_identity() -> dict:
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()
    return {'revision': git('rev-parse', 'HEAD'), 'tracked_diff': git('diff', 'HEAD', '--', 'Cargo.toml', 'Cargo.lock', 'client', 'shared', 'passport', 'skills', 'mobile/ios', 'scripts')}


def build(env: dict[str, str]) -> None:
    cfg = settings(env)
    before = source_identity()
    build_env = ios_build_environment(cfg['cache'], env['SDKROOT'], env.get('OMOBA_GAME_SERVER') or None)
    # Xcode may set architecture/linker flags for its own toolchain. Cargo owns
    # the executable and the device builder's retained-symbol configuration.
    subprocess.run(['cargo', 'build', '--locked', '-p', 'client', '--bin', 'client',
                    '--target', 'aarch64-apple-ios', '--profile', cfg['profile']],
                   cwd=ROOT, env=build_env, check=True)
    binary = cfg['cache'] / 'aarch64-apple-ios' / ('debug' if cfg['profile'] == 'dev' else 'release') / 'client'
    macho = inspect_macho(binary)
    dsym = Path(str(binary) + '.dSYM').resolve()
    symbols = validate_dsym(binary, dsym)
    if before != source_identity():
        raise ValueError('Source changed while Cargo was building. Build again from a stable checkout.')
    bundle = cfg['bundle']
    bundle.mkdir(parents=True, exist_ok=True)
    # Cargo may reuse an older unsigned binary. Preserve fresh output mtime so
    # Xcode invalidates CodeSign after we overwrite its previously signed file.
    shutil.copyfile(binary, bundle / 'client')
    (bundle / 'client').chmod(0o755)
    # This directory belongs exclusively to this phase. Replacement removes
    # deleted source assets on incremental builds without touching Xcode output.
    assets = bundle / 'assets'
    if assets.is_symlink():
        raise ValueError('Refusing a symlink at the generated assets directory.')
    if assets.exists():
        shutil.rmtree(assets)
    for source, relative in tracked_assets(ROOT):
        destination = assets / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    copy_legal_notices(collect_legal_notices(ROOT), assets / 'legal')
    retained = cfg['symbols']
    if retained.suffix != '.dSYM' or retained.is_symlink():
        raise ValueError('Expected an Xcode-generated dSYM output directory.')
    if retained.exists():
        shutil.rmtree(retained)
    retained.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(dsym, retained)
    validate_dsym(bundle / 'client', retained)
    info = app_info(ROOT, macho, env['PRODUCT_BUNDLE_IDENTIFIER'])
    info.pop('UIDeviceFamily', None)  # Xcode derives this from TARGETED_DEVICE_FAMILY.
    info['CFBundleVersion'] = cfg['build']
    cfg['derived'].mkdir(parents=True, exist_ok=True)
    (cfg['derived'] / 'Omoba-Info.plist').write_bytes(plistlib.dumps(info))
    report = {'source_revision': before['revision'], 'source_dirty': bool(before['tracked_diff']),
              'version': info['CFBundleShortVersionString'], 'build': cfg['build'],
              'cargo_profile': cfg['profile'], 'binary_sha256': sha256(bundle / 'client'),
              'debug_symbols': symbols, 'signing_owner': 'Xcode', 'uploaded': False}
    (assets / 'legal/XCODE-BUILD.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f"OMOBA {info['CFBundleShortVersionString']} ({cfg['build']}): Rust executable, assets and matching dSYM staged.")


if __name__ == '__main__':
    try:
        build(dict(os.environ))
    except (KeyError, OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'error: OMOBA iOS build failed: {error}', file=sys.stderr)
        sys.exit(1)
