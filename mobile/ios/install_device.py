#!/usr/bin/env python3
"""Install a signed development app on its enrolled physical iPhone, then launch it."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile


def enrolled_devices(profile, now=None):
    now = now or datetime.now(timezone.utc)
    expiry = profile.get('ExpirationDate')
    if not isinstance(expiry, datetime) or expiry.replace(tzinfo=timezone.utc) <= now:
        raise ValueError('Development profile expired; rebuild and sign with a valid existing profile.')
    devices = profile.get('ProvisionedDevices', [])
    if not devices or profile.get('Entitlements', {}).get('get-task-allow') is not True:
        raise ValueError('This helper needs a device development profile.')
    return {value.casefold() for value in devices}


def choose_device(devices, allowed, requested=None):
    eligible = []
    for device in devices:
        hardware = device.get('hardwareProperties', {})
        if hardware.get('platform') != 'iOS' or hardware.get('reality') != 'physical':
            continue
        if hardware.get('deviceType') != 'iPhone' or hardware.get('udid', '').casefold() not in allowed:
            continue
        identifiers = {str(device.get('identifier', '')).casefold(), hardware.get('udid', '').casefold()}
        if requested and requested.casefold() not in identifiers:
            continue
        if device.get('connectionProperties', {}).get('tunnelState') == 'unavailable':
            continue
        eligible.append(device)
    if not eligible:
        raise ValueError('No available enrolled iPhone. Connect it by USB, unlock it, tap Trust, and enable Developer Mode in Settings > Privacy & Security. Then run this helper again.')
    if len(eligible) != 1:
        choices = ', '.join(str(d['identifier']) for d in eligible)
        raise ValueError('Several enrolled iPhones are available. Choose one with --device: ' + choices)
    result = eligible[0]
    if result.get('deviceProperties', {}).get('developerModeStatus') != 'enabled':
        raise ValueError('Enable Developer Mode on the iPhone, restart it when prompted, then try again.')
    return result['identifier']


def execute(app, device, bundle_id, check=False, run=subprocess.run):
    if check:
        print('Signed app and enrolled iPhone are ready. No installation was requested.')
        return
    # Install is an update; never uninstall or erase the existing app sandbox.
    run(['xcrun','devicectl','device','install','app','--device',device,str(app),'--timeout','60'],check=True)
    run(['xcrun','devicectl','device','process','launch','--device',device,bundle_id,'--timeout','30'],check=True)
    print('Installed and launch command succeeded. Confirm rendering and controls on the phone.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', required=True, type=Path)
    parser.add_argument('--device', help='Explicit CoreDevice identifier or enrolled hardware UDID')
    parser.add_argument('--check', action='store_true', help='Check readiness without installing or launching')
    args = parser.parse_args(argv)
    app = args.app.expanduser().resolve()
    if not app.is_dir() or app.suffix != '.app':
        raise ValueError('Supply the signed OmobaBeta.app directory.')
    subprocess.run(['codesign','--verify','--strict',str(app)],check=True)
    decoded = subprocess.check_output(['security','cms','-D','-i',str(app/'embedded.mobileprovision')],stderr=subprocess.DEVNULL)
    allowed = enrolled_devices(plistlib.loads(decoded))
    with (app/'Info.plist').open('rb') as source:
        bundle_id = plistlib.load(source)['CFBundleIdentifier']
    with tempfile.TemporaryDirectory(prefix='omoba-device-check-') as temp:
        report = Path(temp)/'devices.json'
        subprocess.run(['xcrun','devicectl','list','devices','--json-output',str(report),'--timeout','30'],
                       check=True,stdout=subprocess.DEVNULL)
        device = choose_device(json.loads(report.read_text())['result']['devices'],allowed,args.device)
    execute(app,device,bundle_id,args.check)
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print('iPhone setup:', error)
        raise SystemExit(1)
