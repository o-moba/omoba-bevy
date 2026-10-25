"""Build markets: what a global, Russian or Chinese build of OMOBA ships with.

Mirrors shared/src/market.rs (the compile-time `OMOBA_MARKET` and
`OMOBA_FEATURE_*` the game reads); scripts/test_release.py keeps the two in
step. One source for scripts/release.py and mobile/android/build.py.

A market is a preset of features plus the market's own servers:

- **global** (default): Europe and the rest of the world, today's product: Ekza
  ecosystem on, Hero Supporter purchases on, game <-> website pairing.
- **ru**: a free game on Russian servers. Ekza off, purchases off, Russian by
  default, sign-in with VK ID (VK, OK, Mail.ru) on the RU player portal. Its own
  settings folder (`omoba-bevy-ru`), package names (`Omoba-<v>-ru-*`) and Android
  app (`space.ekza.omoba.ru`), so it never shares device keys with a global
  install: each market has its own account service.
- **cn**: a free game for mainland China, Simplified Chinese by default. Its
  servers and sign-in providers are not decided, so it cannot be packaged yet.

Each market also carries its legal table (`Legal`, LR-06): documents to accept, source
link, support and community channels and age sign; global and cn have none.

The contour (prod / dev, scripts/contours.py) is a separate axis: `--market ru
--contour dev` is the OMOBA Dev tester app with RU features.
"""
from __future__ import annotations

from dataclasses import dataclass, field

MARKET_ENV = "OMOBA_MARKET"
FEATURES = ("ekza", "supporter", "vk_sign_in")
FEATURE_ENVS = {name: f"OMOBA_FEATURE_{name.upper()}" for name in FEATURES}


@dataclass(frozen=True)
class Legal:
    """A market's legal documents, source link, player channels and age sign (LR-06, LR-07,
    LR-21): shared/src/market.rs `Market::legal`, field for field. None is "not provided" and
    is not shown (the README skips it, the game hides it)."""
    version: str | None = None
    """Version of the user agreement and privacy policy the game asks a player to accept."""
    privacy: str | None = None
    terms: str | None = None
    source: str | None = None
    """Public source tree of a release; `{version}` stands for the release (`0.45.0`)."""
    support_email: str | None = None
    community_vk: str | None = None
    community_telegram: str | None = None
    age_rating: str | None = None
    """Age category sign (436-FZ), for example `12+`."""

    def source_for(self, version: str) -> str | None:
        return self.source.replace("{version}", version) if self.source else None


@dataclass(frozen=True)
class Market:
    key: str
    features: dict[str, bool]
    """The preset (shared/src/market.rs Market::default_features)."""
    locale: str
    """First-launch UI language (client/i18n/<locale>/)."""
    storage_suffix: str | None
    """Settings-folder part: omoba-bevy-<suffix>; None keeps omoba-bevy."""
    lobby: str | None = None
    """Production lobby host:port compiled in; None keeps the global public lobby."""
    account_api: str | None = None
    """The market's Account API origin (OMOBA_ACCOUNT_API_URL)."""
    portal: str | None = None
    """The market's player portal origin (OMOBA_PORTAL_ORIGIN): VK sign-in, devices."""
    platforms: tuple[str, ...] = ("macos", "windows", "linux", "android", "ios")
    """Platforms this market is packaged for."""
    zones: tuple[str, ...] = ()
    """DNS zones of the market's own servers; other markets' builds refuse them."""
    blocked: str | None = None
    """Why the market cannot be packaged yet."""
    android: dict[str, str] = field(default_factory=dict)
    """Production Android identity override: bundle_id, display_name."""
    downloads_path: str | None = None
    """The market's own download host: this path on its portal origin (LR-23). `release.py
    publish` uploads the packages there, the download catalog links them and IPFS is the
    mirror. None: the catalog links the IPFS copies."""
    release_label: str = "playtest"
    """The release status players see next to every download (catalog `test_status`)."""
    legal: Legal = Legal()
    """Legal documents, source link, player channels and age sign; global has none."""

    @property
    def label(self) -> str:
        """Inserted after the version in package and dist names (`-ru`); global adds none."""
        return "" if self.key == "global" else f"-{self.key}"

    @property
    def region(self) -> str:
        """The account region this market's servers serve, as the download catalog names it.
        LR-68 turns the market into a runtime region; this stays the one place that says so."""
        return self.key

    @property
    def downloads(self) -> str | None:
        """Base URL of the market's packages (`<portal><downloads_path>`), or None."""
        return f"{self.portal}{self.downloads_path}" if self.portal and self.downloads_path else None


MARKETS = {
    "global": Market("global", {"ekza": True, "supporter": True, "vk_sign_in": False}, "en", None),
    # The RU servers are planned under omoba.ru (REG.RU; docs/markets.md: DNS, hosting in
    # Russia, VK ID app). A build bakes these names in; they answer once deployed.
    "ru": Market("ru", {"ekza": False, "supporter": False, "vk_sign_in": True}, "ru", "ru",
                 lobby="lobby.omoba.ru:4000",
                 account_api="https://account.omoba.ru",
                 portal="https://omoba.ru",
                 # No RU App Store record yet: iOS RU is not packaged.
                 platforms=("macos", "windows", "linux", "android"),
                 zones=("omoba.ru",),
                 android={"bundle_id": "space.ekza.omoba.ru", "display_name": "OMOBA"},
                 # LR-23: packages on the RU host (Caddy, ops/vps/ru/Caddyfile.ru), IPFS mirror.
                 downloads_path="/downloads",
                 release_label="Открытая бета",
                 # Drafts (LR-04) until the counsel-approved texts bring a new version; support
                 # and community wait for the owner (LR-21); 12+ is a placeholder (LR-07).
                 legal=Legal(version="0-draft",
                             privacy="https://omoba.ru/privacy",
                             terms="https://omoba.ru/terms",
                             source="https://github.com/o-moba/omoba-bevy/tree/v{version}-ru",
                             age_rating="12+")),
    "cn": Market("cn", {"ekza": False, "supporter": False, "vk_sign_in": False}, "zh-Hans", "cn",
                 platforms=(), zones=("cn.omoba.io",),
                 blocked="the CN market has no servers or sign-in providers yet (docs/markets.md)"),
}


def market(key: str) -> Market:
    if key not in MARKETS:
        raise SystemExit(f"--market must be one of {', '.join(MARKETS)}, got {key!r}")
    return MARKETS[key]


def parse_feature_overrides(values: list[str] | None) -> dict[str, bool]:
    """`--feature ekza=on --feature supporter=off` → {"ekza": True, "supporter": False}."""
    overrides: dict[str, bool] = {}
    for value in values or []:
        name, _, state = value.partition("=")
        if name not in FEATURES or state not in ("on", "off"):
            raise SystemExit(f"--feature must be <{'|'.join(FEATURES)}>=on|off, got {value!r}")
        if name in overrides:
            raise SystemExit(f"--feature {name} given twice")
        overrides[name] = state == "on"
    return overrides


def resolve_features(key: str, overrides: dict[str, bool] | None = None) -> dict[str, bool]:
    features = dict(market(key).features)
    features.update(overrides or {})
    return features


def market_env(key: str, overrides: dict[str, bool] | None = None) -> dict[str, str]:
    """The compile-time market environment one build bakes in (shared/src/market.rs)."""
    env = {} if key == "global" else {MARKET_ENV: key}
    preset = market(key).features
    for name, value in (overrides or {}).items():
        if value != preset[name]:
            env[FEATURE_ENVS[name]] = "on" if value else "off"
    return env


def config_dir(market_key: str, contour_suffix: str | None) -> str:
    """Mirrors client/src/platform.rs: omoba-bevy[-<market>][-<contour>]."""
    parts = ["omoba-bevy"]
    if suffix := market(market_key).storage_suffix:
        parts.append(suffix)
    if contour_suffix:
        parts.append(contour_suffix)
    return "-".join(parts)
