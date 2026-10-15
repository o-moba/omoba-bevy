"""Android packager: the production and the OMOBA Dev app never share an identity."""
from pathlib import Path
import sys
import unittest
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build  # noqa: E402

ANDROID = "{http://schemas.android.com/apk/res/android}"
TEMPLATE = Path(build.__file__).with_name("AndroidManifest.xml").read_text()


class ManifestTests(unittest.TestCase):
    def manifest(self, contour):
        return ET.fromstring(build.manifest_for(TEMPLATE, "0.45.0", 450009, contour))

    def test_production_identity_is_unchanged(self):
        root = self.manifest("prod")
        self.assertEqual(root.get("package"), "space.ekza.omoba.beta")
        self.assertEqual(root.find("application").get(ANDROID + "label"), "Omoba Beta")
        self.assertEqual(root.get(ANDROID + "versionName"), "0.45.0")
        self.assertEqual(root.get(ANDROID + "versionCode"), "450009")
        self.assertEqual(build.resource_dir("prod").name, "res")

    def test_the_dev_app_installs_beside_production(self):
        prod, dev = self.manifest("prod"), self.manifest("dev")
        self.assertEqual(dev.get("package"), "space.ekza.omoba.dev")
        self.assertEqual(dev.find("application").get(ANDROID + "label"), "OMOBA Dev")
        self.assertNotEqual(dev.get("package"), prod.get("package"))
        # Everything else (permissions, activity, launcher) is the same app.
        for element in (prod, dev):
            element.attrib.pop("package")
            element.find("application").attrib.pop(ANDROID + "label")
        self.assertEqual(ET.tostring(prod), ET.tostring(dev))

    def test_the_dev_app_has_its_own_launcher_icons_for_every_density(self):
        prod, dev = build.resource_dir("prod"), build.resource_dir("dev")
        self.assertEqual(dev.name, "res-dev")
        densities = sorted(path.relative_to(prod) for path in prod.rglob("*.png"))
        self.assertTrue(densities)
        self.assertEqual(sorted(path.relative_to(dev) for path in dev.rglob("*.png")), densities)
        for relative in densities:
            self.assertNotEqual((prod / relative).read_bytes(), (dev / relative).read_bytes(), relative)

    def test_a_changed_template_fails_instead_of_shipping_the_production_identity(self):
        with self.assertRaises(RuntimeError):
            build.manifest_for(TEMPLATE.replace('android:label="Omoba Beta"', 'android:label="X"'),
                               "0.45.0", 1, "dev")

    def test_apk_names_never_collide(self):
        self.assertEqual(build.apk_name("0.45.0", "arm64"), "omoba-0.45.0-android-arm64-debug.apk")
        self.assertEqual(build.apk_name("0.45.0", "arm64", "dev"), "omoba-0.45.0-dev-android-arm64-debug.apk")


class EnvironmentTests(unittest.TestCase):
    SHELL = {"PATH": "/bin", "OMOBA_DEFAULT_REGISTRY_URL": "https://registry.ekza.io",
             "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "stale", "OMOBA_DEFAULT_SOMETHING": "x",
             "EKZA_DEV_HTTP_HOST": "192.168.1.71"}

    def test_the_dev_app_compiles_only_its_own_defaults(self):
        env = build.contour_env(dict(self.SHELL), "dev", None)
        self.assertEqual({k: v for k, v in env.items() if k != "PATH"}, {
            "OMOBA_DEFAULT_GAME_SERVER_ADDR": "lobby.dev.omoba.io:4100",
            "OMOBA_DEFAULT_REGISTRY_URL": "https://registry.dev.ekza.io",
            "OMOBA_DEFAULT_PASSPORT_URL": "http://127.0.0.1:9/api/passport",
            "OMOBA_DEFAULT_CONFIG_DIR_SUFFIX": "dev"})
        by_ip = build.contour_env(dict(self.SHELL), "dev", "203.0.113.10:4100")
        self.assertEqual(by_ip["OMOBA_DEFAULT_GAME_SERVER_ADDR"], "203.0.113.10:4100")

    def test_the_production_app_never_takes_a_settings_suffix(self):
        env = build.contour_env(dict(self.SHELL), "prod", "192.0.2.1:4000")
        self.assertNotIn("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX", env)
        self.assertEqual(env["OMOBA_DEFAULT_GAME_SERVER_ADDR"], "192.0.2.1:4000")
        self.assertNotIn("OMOBA_DEFAULT_GAME_SERVER_ADDR", build.contour_env({"PATH": "/bin"}, "prod", None))


if __name__ == "__main__":
    unittest.main()
