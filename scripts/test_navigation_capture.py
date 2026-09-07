#!/usr/bin/env python3
"""Tests for independent native navigation evidence, without a GUI process."""
import copy
import json
from pathlib import Path
import select
import socket
import tempfile
import time
import unittest

from capture_verdant import FRAME_HEADER, SnapshotObserver, verify_navigation


def evidence():
    names = ("minimap_rmb_input", "minimap_order_accepted", "minimap_arrival", "world_rmb_input",
             "world_order_accepted", "world_arrival", "alt_world_blocked", "alt_minimap_blocked",
             "shop_world_blocked", "shop_minimap_blocked", "pause_world_blocked", "pause_minimap_blocked", "lmb_minimap_pan")
    events = [{"event": name, "snapshot_tick": tick, "detail": {}}
              for name, tick in zip(names, (1, 2, 10, 20, 21, 30, 32, 34, 36, 38, 40, 42, 44))]
    events[0]["detail"]["destination"] = [7, 0, 0]
    events[1]["detail"]["waypoints"] = [[-4, 0, 0], [-2.83, 0, 2.83], [0, 0, 4], [2.83, 0, 2.83], [4, 0, 0], [7, 0, 0]]
    events[2]["detail"]["destination"] = [7, 0, 0]
    events[5]["detail"]["destination"] = [12, 0, -4]
    summary = dict(pass_=True, player_id=7, start=[-7, 0, 0], obstacle=dict(center=[0, 0, 0], collision_radius=3.7),
                   events=events, scripted_input=True)
    summary["pass"] = summary.pop("pass_")
    positions = ((1, -7, 0), (2, -4, 0), (3, -2.83, 2.83), (4, 0, 4), (5, 2.83, 2.83),
                 (6, 4, 0), (10, 7, 0), (11, 7, 0), (30, 12, -4), (31, 12, -4))
    snapshots = [dict(snapshot_tick=tick, players=[dict(id=7, x=x, z=z, hp=100)]) for tick, x, z in positions]
    return summary, snapshots


class NavigationEvidenceTest(unittest.TestCase):
    def test_real_observer_path_and_two_stopped_snapshots_are_required(self):
        summary, snapshots = evidence()
        result = verify_navigation(summary, snapshots)
        self.assertTrue(result["pass"])
        self.assertEqual(result["arrivals"]["minimap_arrival"]["nearby_snapshots"], 2)
        self.assertGreater(result["maximum_deviation_from_direct_line"], 3.7)
        snapshots = [snapshot for snapshot in snapshots if snapshot["snapshot_tick"] != 11]
        self.assertFalse(verify_navigation(summary, snapshots)["pass"])

    def test_client_claims_do_not_replace_authority_or_input_events(self):
        summary, snapshots = evidence()
        self.assertFalse(verify_navigation(summary, [])["pass"])
        wrong_id = copy.deepcopy(summary)
        wrong_id["player_id"] = 8
        self.assertFalse(verify_navigation(wrong_id, snapshots)["pass"])
        summary["events"] = [event for event in summary["events"] if event["event"] != "alt_minimap_blocked"]
        self.assertFalse(verify_navigation(summary, snapshots)["pass"])

    def test_structure_penetration_is_rejected(self):
        summary, snapshots = evidence()
        snapshots[3]["players"][0].update(x=0, z=0)
        self.assertFalse(verify_navigation(summary, snapshots)["pass"])

    def test_excessive_planned_detour_is_rejected_despite_correct_arrival(self):
        summary, snapshots = evidence()
        summary["events"][1]["detail"]["waypoints"].insert(0, [-70, 0, 30])
        result = verify_navigation(summary, snapshots)
        self.assertFalse(result["pass"])
        self.assertGreater(result["planned_route_length"], 25)
        self.assertTrue(all(arrival["nearby_snapshots"] >= 2 for arrival in result["arrivals"].values()))

    def test_excessive_authoritative_traversal_is_rejected_despite_correct_arrival(self):
        summary, snapshots = evidence()
        snapshots[2]["players"][0].update(x=-30, z=30)
        result = verify_navigation(summary, snapshots)
        self.assertFalse(result["pass"])
        self.assertLess(result["planned_route_length"], 25)
        self.assertGreater(result["authoritative_traversal_length"], 30)
        self.assertTrue(all(arrival["nearby_snapshots"] >= 2 for arrival in result["arrivals"].values()))

    def test_hello_only_observer_reassembles_real_udp_chunks(self):
        with tempfile.TemporaryDirectory() as directory, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server:
            server.bind(("127.0.0.1", 0))
            server.settimeout(1)
            path = Path(directory) / "snapshots.jsonl"
            observer = SnapshotObserver(server.getsockname(), path)
            try:
                observer.update(1)
                hello, address = server.recvfrom(65536)
                self.assertEqual(json.loads(hello), dict(type="hello", protocol_version=1))
                packet = dict(type="snapshot", server_epoch=9, round_id=1, snapshot_tick=42,
                              players=[dict(id=7, x=1, z=2, hp=100)], structures=[])
                data = json.dumps(packet).encode()
                halves = (data[:len(data) // 2], data[len(data) // 2:])
                for index in (1, 0):
                    server.sendto(FRAME_HEADER.pack(b"OMB1", 1, 9, 42, index, 2, len(data)) + halves[index], address)
                deadline = time.monotonic() + 1
                while not observer.samples and time.monotonic() < deadline:
                    select.select([observer.socket], [], [], max(0, deadline - time.monotonic()))
                    observer.update(1.1)
                self.assertEqual(observer.samples[0]["snapshot_tick"], 42)
                self.assertEqual(json.loads(path.read_text())["players"], packet["players"])
                server.setblocking(False)
                with self.assertRaises(BlockingIOError):
                    server.recvfrom(65536)  # No Join/Transform commands were sent.
            finally:
                observer.close()


if __name__ == "__main__":
    unittest.main()
