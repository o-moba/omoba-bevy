import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
import zipfile

import android_diagnose as diagnose


class AndroidDiagnosticsTests(unittest.TestCase):
    def test_android_12_alone_is_not_a_diagnosis(self):
        findings = diagnose.assess({"sdk": "31", "abis": "arm64-v8a,armeabi-v7a",
                                   "features": "feature:android.hardware.vulkan.level=1"},
                                  ["arm64-v8a"], "", "")
        self.assertIn("No cause established", findings[0])

    def test_apk_abis_and_device_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            apk = Path(directory) / "app.apk"
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("lib/arm64-v8a/libclient.so", b"test")
                archive.writestr("assets/lib/x86_64/libclient.so", b"not native")
            abis = diagnose.apk_abis(apk)
        self.assertEqual(abis, ["arm64-v8a"])
        self.assertIn("no native ABI", diagnose.assess({"abis": "armeabi-v7a"}, abis, "", "")[0])

    def test_filters_unrelated_crashes_and_sensitive_lines(self):
        def line(pid, text):
            return f"10-10 13:00:00.001 {pid} 99 E AndroidRuntime: {text}"
        log = "\n".join([line(12, "Process: another.app, PID: 12"), line(12, "unrelated data"),
                         line(34, "Process: space.ekza.omoba.beta, PID: 34"),
                         line(34, 'java.lang.UnsatisfiedLinkError: cannot locate symbol "__cxa_pure_virtual"'),
                         line(34, "Authorization: secret"), line(35, "Cmdline: space.ekza.omoba.beta.fake")])
        excerpt = diagnose.crash_excerpt(log)
        self.assertIn("__cxa_pure_virtual", excerpt)
        for forbidden in ("unrelated", "another.app", "secret", ".fake"):
            self.assertNotIn(forbidden, excerpt)
        self.assertIn("loader failure", diagnose.assess({}, [], excerpt, "")[0])

    def test_native_crash_and_low_memory_evidence(self):
        log = "10-10 13:00:00.001 45 45 F DEBUG: Cmdline: space.ekza.omoba.beta\n"
        log += "10-10 13:00:00.002 45 45 F DEBUG: VK_ERROR_DEVICE_LOST"
        findings = diagnose.assess({}, [], diagnose.crash_excerpt(log), "reason=3 (LOW_MEMORY)")
        self.assertEqual(len(findings), 2)

    def test_vulkan_level_zero_is_not_level_one(self):
        findings = diagnose.assess({"features": "feature:android.hardware.vulkan.level=0"}, [], "", "")
        self.assertIn("below the APK's declared level 1", findings[0])

    def test_no_device_fails_without_launch_or_file_access(self):
        with patch.object(diagnose.subprocess, "run") as run:
            run.return_value.returncode = 0
            run.return_value.stdout = "List of devices attached\n\n"
            with self.assertRaisesRegex(RuntimeError, "exactly one"):
                diagnose.collect("adb", launch=True)
            self.assertEqual(run.call_count, 1)

    def test_readonly_collection_never_starts_or_clears(self):
        commands = []

        def run(command, **kwargs):
            from subprocess import CompletedProcess
            commands.append(command)
            out = "List of devices attached\nprivate-serial\tdevice\n" if command[-1] == "devices" else ""
            return CompletedProcess(command, 0, out, "")

        with patch.object(diagnose.subprocess, "run", side_effect=run):
            report = diagnose.collect("adb")
        self.assertFalse(report["launch_requested"])
        self.assertNotIn("private-serial", str(report))
        self.assertFalse(any("start" in cmd or "-c" in cmd or "install" in cmd for cmd in commands))


if __name__ == "__main__":
    unittest.main()
