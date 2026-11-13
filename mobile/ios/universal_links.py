"""Public domain association shared by the Xcode and CLI package checks."""
from pathlib import Path
import plistlib

DOMAIN_KEY = 'com.apple.developer.associated-domains'
ENTITLEMENTS = {'space.ekza.omoba.beta': 'Omoba.entitlements',
                'space.ekza.omoba.dev': 'OmobaDev.entitlements'}


def associated_domains(bundle_id: str) -> list[str]:
    name = ENTITLEMENTS.get(bundle_id)
    if name is None:
        return []
    return plistlib.loads(Path(__file__).with_name(name).read_bytes())[DOMAIN_KEY]


def require_associated_domains(profile: dict, requested: list[str]) -> dict:
    """Use only requested domains, and only when the existing profile permits them."""
    if not requested:
        return {}
    allowed = profile.get('Entitlements', {}).get(DOMAIN_KEY, [])
    if not isinstance(allowed, list) or not all(domain in allowed or '*' in allowed for domain in requested):
        raise ValueError('Provisioning profile does not permit the required Associated Domains. '
                         'Enable the capability for this App ID and refresh its profile in Xcode.')
    return {DOMAIN_KEY: requested}
