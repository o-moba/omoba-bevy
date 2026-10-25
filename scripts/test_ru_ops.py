"""The RU host kit (ops/vps/ru) describes one topology (README.md#topology, LR-02):
the portal calls the loopback Account API with OMOBA_TRUST_LOOPBACK_PROXY=1 on both
sides, Caddy overwrites X-Real-IP on the portal site (omoba.ru), and the Account API site
(account.omoba.ru) serves only native routes and YooKassa's webhook (LR-67)."""
from __future__ import annotations

import re
import unittest
from pathlib import Path

import markets

RU = Path(__file__).resolve().parents[1] / "ops" / "vps" / "ru"
# Signed routes the RU game calls (docs/architecture/system.md, "Account API ingress allowlists").
RU_NATIVE = ["/v1/auth/pairings/lookup", "/v1/auth/pairings/approve", "/v1/auth/pairings/deny",
             "/v1/auth/devices/create", "/v1/auth/devices/status", "/v1/auth/devices/recover",
             "/v1/auth/devices/complete", "/v1/auth/social/link"]
# YooKassa's HTTP notifications (LR-67): the shop's webhook URL is this path on
# account.omoba.ru (ops/vps/ru/README.md, "YooKassa").
RU_YOOKASSA = ["/v1/supporter/yookassa/notifications"]


def env_file(name: str) -> dict[str, str]:
    values = {}
    for line in (RU / name).read_text().splitlines():
        if line.strip() and not line.lstrip().startswith("#"):
            key, _, value = line.partition("=")
            values[key.strip()] = value.strip()
    return values


def site(caddyfile: str, host: str) -> str:
    """The `host { … }` block, braces balanced. The site address starts a line, so the
    portal `omoba.ru` never matches inside `account.omoba.ru`."""
    found = re.search(rf"(?m)^{re.escape(host)} \{{", caddyfile)
    if found is None:
        raise AssertionError(host + " has no site block")
    start = found.start()
    depth = 0
    for index in range(caddyfile.index("{", start), len(caddyfile)):
        depth += {"{": 1, "}": -1}.get(caddyfile[index], 0)
        if depth == 0:
            return caddyfile[start:index + 1]
    raise AssertionError(host + " block is not closed")


class RuTopologyTest(unittest.TestCase):
    def setUp(self):
        self.portal = env_file("portal.env.example")
        self.api = env_file("account-api.env.example")
        caddyfile = (RU / "Caddyfile.ru").read_text()
        self.account = site(caddyfile, "account.omoba.ru")
        self.app = site(caddyfile, "omoba.ru")

    def test_the_portal_calls_the_loopback_api_and_both_trust_the_loopback_proxy(self):
        self.assertEqual(self.portal["OMOBA_ACCOUNT_API_URL"], "http://" + self.api["OMOBA_ACCOUNT_BIND"])
        self.assertEqual(self.api["OMOBA_ACCOUNT_BIND"], "127.0.0.1:40550")
        self.assertEqual(self.portal["OMOBA_TRUST_LOOPBACK_PROXY"], "1")
        self.assertEqual(self.api["OMOBA_TRUST_LOOPBACK_PROXY"], "1")
        # No portal ingress key exists on RU: nothing to leak, rotate or forget.
        self.assertNotIn("OMOBA_ACCOUNT_INGRESS_KEY", self.portal)
        self.assertEqual(self.portal["OMOBA_ACCOUNT_MODE"], "enabled")
        self.assertEqual(self.portal["OMOBA_MARKET"], "ru")
        self.assertEqual(self.portal["OMOBA_PORTAL_ORIGIN"], markets.MARKETS["ru"].portal)
        self.assertEqual(self.api["OMOBA_PORTAL_ORIGIN"], markets.MARKETS["ru"].portal)

    def test_app_ru_overwrites_x_real_ip_for_the_loopback_portal(self):
        self.assertTrue(markets.MARKETS["ru"].portal.endswith("//omoba.ru"))
        proxy = re.search(r"reverse_proxy\s+127\.0\.0\.1:(\d+)\s*\{(.*?)\n\s*\}", self.app, re.S)
        self.assertIsNotNone(proxy, self.app)
        self.assertEqual(proxy.group(1), self.portal["PORT"])
        self.assertRegex(proxy.group(2), r"header_up\s+X-Real-IP\s+\{remote_host\}")

    def test_account_ru_serves_only_native_routes_and_health(self):
        self.assertTrue(markets.MARKETS["ru"].account_api.endswith("//account.omoba.ru"))
        native = re.search(r"@native\s*\{\s*method POST\s+path ([^\n]+)\s*\}", self.account)
        self.assertIsNotNone(native, self.account)
        self.assertEqual(native.group(1).split(), RU_NATIVE)
        self.assertNotIn("X-Omoba-Portal-Key", self.account)
        self.assertNotIn("PORTAL_KEY", self.account)
        handles = re.findall(r"handle(?: (@\w+))? \{", self.account)
        self.assertEqual(handles, ["@native", "@yookassa", "@health", ""],
                         "native, the YooKassa webhook, health, then 403 for the rest")
        self.assertRegex(self.account, r'handle \{\s*respond "Forbidden" 403\s*\}')
        # The Account API keys native limits on the game's address, never on Caddy's.
        native_proxy = re.search(r"handle @native \{\s*reverse_proxy 127\.0\.0\.1:40550 \{(.*?)\n\s*\}", self.account, re.S)
        self.assertIsNotNone(native_proxy, self.account)
        self.assertRegex(native_proxy.group(1), r"header_up\s+X-Real-IP\s+\{remote_host\}")

    def test_account_ru_passes_only_the_yookassa_webhook_as_its_own_public_handle(self):
        # A POST of exactly the notification path, not a prefix: checkout and the payment
        # list stay portal-only (loopback), like every other Supporter route.
        webhook = re.search(r"@yookassa\s*\{\s*method POST\s+path ([^\n]+)\s*\}", self.account)
        self.assertIsNotNone(webhook, self.account)
        self.assertEqual(webhook.group(1).split(), RU_YOOKASSA)
        self.assertFalse(any("*" in path for path in webhook.group(1).split()))
        self.assertTrue(set(RU_YOOKASSA).isdisjoint(RU_NATIVE), "its own handle, not the native list")
        # Per-IP notification limits use YooKassa's address, overwritten by Caddy.
        proxy = re.search(r"handle @yookassa \{\s*reverse_proxy 127\.0\.0\.1:40550 \{(.*?)\n\s*\}", self.account, re.S)
        self.assertIsNotNone(proxy, self.account)
        self.assertRegex(proxy.group(1), r"header_up\s+X-Real-IP\s+\{remote_host\}")


if __name__ == "__main__":
    unittest.main()
