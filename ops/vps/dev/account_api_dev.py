#!/usr/bin/env python3
"""Run on vds-dev as root; prepares only the isolated dev Account API.

No secrets are accepted in argv or printed. Default is a read-only preflight.
The lobby ship command owns migrations and immutable binary releases.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import tempfile
from urllib.parse import urlsplit

ROOT = Path('/opt/omoba-dev')
ENV = ROOT / 'account-api.env'
KEY = ROOT / 'account-api-ingress.key'
CONTAINER = 'supabase_db_ekza-dev'
PORTAL = 'https://app.dev.omoba.io'
API = 'account.dev.omoba.io'
UNIT_PATH = Path('/etc/systemd/system/omoba-account-api-dev.service')
NATIVE = (
    '/v1/auth/pairings/lookup', '/v1/auth/pairings/approve',
    '/v1/auth/pairings/deny', '/v1/auth/devices/create',
    '/v1/auth/devices/status', '/v1/auth/devices/recover',
    '/v1/auth/devices/complete', '/v1/auth/devices/recovery-codes',
    '/v1/supporter/native',
)

def run(args, stdin=None):
    result = subprocess.run(args, input=stdin, text=True, capture_output=True)
    if result.returncode:
        # A SQL or systemd error may contain an environment value. Keep raw
        # diagnostics off stdout; inspect the host locally if this code appears.
        raise RuntimeError('command failed: ' + args[0])
    return result.stdout.strip()

def sql(query):
    return run(['docker', 'exec', '-i', CONTAINER, 'psql', '-X', '-At',
                '-v', 'ON_ERROR_STOP=1', '-U', 'supabase_admin', '-d', 'omoba_dev'], query)

def secure(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) != 0o600:
        raise RuntimeError('refused insecure private file: ' + str(path))
    return path.read_text()

def atomic(path, value, mode=0o600):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, mode='w', delete=False) as f:
        os.chmod(f.name, mode)
        f.write(value)
        f.flush()
        os.fsync(f.fileno())
        staged = f.name
    os.replace(staged, path)

def guard():
    if os.geteuid() != 0 or '146.103.102.18' not in run(['hostname', '-I']).split():
        raise RuntimeError('refused: expected root on the exact dev VPS')
    for name in ('/opt/omoba', '/var/lib/omoba', '/opt/ekza-studio', '/opt/ekza-mirror'):
        if Path(name).exists():
            raise RuntimeError('refused production marker')
    if run(['docker', 'inspect', '-f', '{{.State.Running}}', CONTAINER]) != 'true':
        raise RuntimeError('dev database is not running')
    ports = json.loads(run(['docker', 'inspect', '-f', '{{json .NetworkSettings.Ports}}', CONTAINER]))
    if ports.get('5432/tcp') != [{'HostIp': '127.0.0.1', 'HostPort': '54322'}]:
        raise RuntimeError('refused database exposure')
    roles = sql("SELECT rolname||':'||rolsuper||':'||rolcreatedb||':'||rolcreaterole||':'||rolreplication FROM pg_roles WHERE rolname='omoba_dev_portal';")
    if roles != 'omoba_dev_portal:false:false:false:false':
        raise RuntimeError('refused portal role privileges')
    if sql("SELECT count(*) FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.member WHERE r.rolname='omoba_dev_portal';") != '0':
        raise RuntimeError('refused inherited portal role privileges')

def env_values():
    if not ENV.exists():
        return {'OMOBA_DATABASE_URL': 'postgres://omoba_dev_portal:' + secrets.token_hex(32) + '@127.0.0.1:54322/omoba_dev',
                'OMOBA_PORTAL_SECRET': secrets.token_hex(32),
                'OMOBA_PORTAL_ORIGIN': PORTAL,
                'OMOBA_ACCOUNT_BIND': '127.0.0.1:40550',
                'OMOBA_TRUST_LOOPBACK_PROXY': '1'}
    values = dict(line.split('=', 1) for line in secure(ENV).splitlines() if line)
    url = urlsplit(values.get('OMOBA_DATABASE_URL', ''))
    if (set(values) != {'OMOBA_DATABASE_URL', 'OMOBA_PORTAL_SECRET', 'OMOBA_PORTAL_ORIGIN', 'OMOBA_ACCOUNT_BIND', 'OMOBA_TRUST_LOOPBACK_PROXY'}
            or url.scheme != 'postgres' or url.hostname != '127.0.0.1' or url.port != 54322
            or url.username != 'omoba_dev_portal' or url.path != '/omoba_dev' or url.query or url.fragment
            or not re.fullmatch('[0-9a-f]{64}', url.password or '')
            or not re.fullmatch('[0-9a-f]{64}', values.get('OMOBA_PORTAL_SECRET', ''))
            or values.get('OMOBA_PORTAL_ORIGIN') != PORTAL
            or values.get('OMOBA_ACCOUNT_BIND') != '127.0.0.1:40550'
            or values.get('OMOBA_TRUST_LOOPBACK_PROXY') != '1'):
        raise RuntimeError('refused existing env drift; no credentials rotated')
    return values

def fragment(key):
    return f'''{API} {{
    header X-Robots-Tag "noindex, nofollow"
    header X-Content-Type-Options nosniff
    request_body {{
        max_size 64KB
    }}
    @native {{
        method POST
        path {' '.join(NATIVE)}
    }}
    @portal {{
        path /v1/*
        header X-Omoba-Portal-Key {key}
    }}
    @health {{
        method GET
        path /health/live /health/ready
    }}
    handle @native {{
        reverse_proxy 127.0.0.1:40550 {{
            header_up X-Real-IP {{remote_host}}
            header_up -X-Omoba-Portal-Key
        }}
    }}
    handle @portal {{
        reverse_proxy 127.0.0.1:40550 {{
            header_up X-Real-IP {{remote_host}}
            header_up -X-Omoba-Portal-Key
        }}
    }}
    handle @health {{
        reverse_proxy 127.0.0.1:40550
    }}
    handle {{
        respond "Forbidden" 403
    }}
}}
'''

UNIT = '''[Unit]
Description=OMOBA DEV Account API (isolated test contour)
After=network-online.target docker.service omoba-lobby-dev.service
Wants=network-online.target

[Service]
Type=simple
User=omoba-dev
Group=omoba-dev
EnvironmentFile=/opt/omoba-dev/account-api.env
ExecStart=/opt/omoba-dev/current/omoba-account-api
Restart=on-failure
RestartSec=5
TimeoutStopSec=15
KillSignal=SIGINT
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
CapabilityBoundingSet=
MemoryMax=256M
CPUQuota=50%
TasksMax=64

[Install]
WantedBy=multi-user.target
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--backup')
    parser.add_argument('--activate-revision')
    args = parser.parse_args()
    if args.activate_revision and not args.apply:
        parser.error('--activate-revision requires --apply')
    guard()
    if not args.apply:
        print(json.dumps({'status': 'PREFLIGHT_OK', 'target': API, 'bind': '127.0.0.1:40550', 'schema': sql('SELECT version FROM portal.schema_version ORDER BY version;').splitlines(), 'plan': 'prepare persistent dev credentials, restricted Caddy fragment and Account API unit; activation requires exact migrated release'}))
        return
    backup = Path(args.backup or '')
    if not re.fullmatch(r'/opt/ekza-dev/backups/\d{8}T\d{6}Z', str(backup)):
        raise RuntimeError('explicit completed dev backup required')
    manifest = json.loads(secure(backup / 'MANIFEST.json'))
    if not (backup / 'omoba_dev.dump').is_file() or (backup / 'omoba_dev.dump').stat().st_size == 0 or not manifest:
        raise RuntimeError('career database backup missing')
    # Reject an invalid activation before enabling logins or writing any files.
    if args.activate_revision:
        validate_activation(args.activate_revision)
    values = env_values()
    key = secure(KEY).strip() if KEY.exists() else secrets.token_hex(32)
    if not re.fullmatch('[0-9a-f]{64}', key):
        raise RuntimeError('refused ingress key drift')
    # Persist before enabling the login so retries reuse exactly the same values.
    if not ENV.exists():
        atomic(ENV, ''.join(k + '=' + v + '\n' for k, v in values.items()))
    if not KEY.exists():
        atomic(KEY, key + '\n')
    password = urlsplit(values['OMOBA_DATABASE_URL']).password
    sql("ALTER ROLE omoba_dev_portal LOGIN PASSWORD '" + password + "';")
    atomic(Path('/opt/ekza-dev/secrets/caddy/account-api.caddy'), fragment(key))
    atomic(UNIT_PATH, UNIT, 0o644)
    run(['systemd-analyze', 'verify', str(UNIT_PATH)])
    run(['systemctl', 'daemon-reload'])
    if args.activate_revision:
        run(['systemctl', 'enable', '--now', 'omoba-account-api-dev'])
        run(['systemctl', 'restart', 'omoba-account-api-dev'])
    print(json.dumps({'status': 'PREPARED', 'activated': bool(args.activate_revision), 'api': API, 'secret_values_printed': False}))

def validate_activation(revision):
    if not re.fullmatch('[0-9a-f]{40}', revision):
        raise RuntimeError('exact full revision required')
    build = json.loads((ROOT / 'current/BUILD.json').read_text())
    if build.get('contour') != 'dev' or build.get('revision') != revision:
        raise RuntimeError('current release does not match requested dev revision')
    binary = ROOT / 'current/omoba-account-api'
    if hashlib.sha256(binary.read_bytes()).hexdigest() != build.get('account_api_sha256'):
        raise RuntimeError('Account API binary hash mismatch')
    # Exactly the portal schema this release serves (account-api PORTAL_SCHEMA).
    if sql('SELECT version FROM portal.schema_version ORDER BY version;').splitlines() != ['1','2','3','4','5','6','7']:
        raise RuntimeError('portal migration7 must be applied by lobby ship first')

if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        raise SystemExit(str(error))
