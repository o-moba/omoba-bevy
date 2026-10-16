"""Build contours: what a production build and a dev-contour build are.

One source for scripts/release.py, mobile/android/build.py and the iOS Xcode
build phase (mobile/ios/xcode_build.py), so the two apps never share an identity:

- **prod**: the public OMOBA app. iOS/Android `space.ekza.omoba.beta`
  ("Omoba Beta"), desktop settings folder `omoba-bevy`. Nothing is compiled in
  unless a release asks for it; the client shows no server selection.
- **dev**: the separate OMOBA Dev tester app. iOS/Android `space.ekza.omoba.dev`
  ("OMOBA Dev", the icon with the DEV ribbon from scripts/make_dev_icons.py),
  settings folder `omoba-bevy-dev`, the dev lobby, dev registry and the closed
  passport compiled in as its Dev connection profile. It installs beside the
  production app on one phone and offers the Dev / Local / Custom connection
  profiles (client/src/connection_profile.rs).
"""
from __future__ import annotations

from dataclasses import dataclass

CONTOURS = ("prod", "dev")

# Compile-time client defaults (client/src/session_config.rs,
# passport/src/community.rs, passport/src/lib.rs, client/src/platform.rs).
SERVER_DEFAULT_ENV = "OMOBA_DEFAULT_GAME_SERVER_ADDR"
REGISTRY_DEFAULT_ENV = "OMOBA_DEFAULT_REGISTRY_URL"
PASSPORT_DEFAULT_ENV = "OMOBA_DEFAULT_PASSPORT_URL"
CONFIG_SUFFIX_ENV = "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX"
ACCOUNT_API_ENV = "OMOBA_ACCOUNT_API_URL"
PORTAL_ORIGIN_ENV = "OMOBA_PORTAL_ORIGIN"

# The DEV contour: ops/vps/omoba-lobby-dev.service on the dedicated dev VPS
# (lobby.dev.omoba.io) and registry.dev.ekza.io. There is no dev Ekza passport yet
# and https://avatar.ekza.io/api/passport (the SDK default) is production, so dev
# builds use a closed loopback port: wallet pairing and protected-avatar tickets
# fail closed. Free and shipped avatars still work.
DEV_LOBBY = "lobby.dev.omoba.io:4100"
DEV_REGISTRY = "https://registry.dev.ekza.io"
DEV_PASSPORT = "http://127.0.0.1:9/api/passport"
DEV_CONFIG_SUFFIX = "dev"
DEV_ACCOUNT_API = "https://account.dev.omoba.io"
DEV_PORTAL_ORIGIN = "https://app.dev.omoba.io"


@dataclass(frozen=True)
class MobileIdentity:
    """One phone app: its store identity, name, icon and Android resources."""
    bundle_id: str
    """iOS CFBundleIdentifier and Android applicationId/package."""
    display_name: str
    """Home-screen name (CFBundleDisplayName/CFBundleName, android:label)."""
    ios_app_icon: str
    """App icon set in mobile/ios/Assets.xcassets (ASSETCATALOG_COMPILER_APPICON_NAME)."""
    android_res: str
    """Resource folder under mobile/android (launcher icons)."""


MOBILE = {
    "prod": MobileIdentity("space.ekza.omoba.beta", "Omoba Beta", "AppIcon", "res"),
    "dev": MobileIdentity("space.ekza.omoba.dev", "OMOBA Dev", "AppIconDev", "res-dev"),
}


def mobile_identity(contour: str) -> MobileIdentity:
    if contour not in MOBILE:
        raise ValueError(f"unknown contour {contour!r}; use one of {', '.join(CONTOURS)}")
    return MOBILE[contour]


def contour_of_xcode_setting(value: str | None) -> str:
    """The contour an `OMOBA_CONTOUR` Xcode build setting names: empty is prod."""
    value = (value or "").strip()
    if value in ("", "prod"):
        return "prod"
    if value == "dev":
        return "dev"
    raise ValueError(f"OMOBA_CONTOUR must be empty or dev, got {value!r}")


def compile_defaults(contour: str, server: str | None = None) -> dict[str, str]:
    """The compile-time client defaults a phone build of `contour` bakes in.

    Production bakes in only an explicit server. Dev bakes in its own lobby
    (or `server`), registry, closed passport and settings-folder suffix.
    """
    if contour == "prod":
        return {SERVER_DEFAULT_ENV: server} if server else {}
    if contour == "dev":
        return {SERVER_DEFAULT_ENV: server or DEV_LOBBY, REGISTRY_DEFAULT_ENV: DEV_REGISTRY,
                PASSPORT_DEFAULT_ENV: DEV_PASSPORT, CONFIG_SUFFIX_ENV: DEV_CONFIG_SUFFIX,
                ACCOUNT_API_ENV: DEV_ACCOUNT_API, PORTAL_ORIGIN_ENV: DEV_PORTAL_ORIGIN}
    raise ValueError(f"unknown contour {contour!r}")
