#!/usr/bin/env python3
"""The PostgreSQL test runner derives the restricted-role URL from the owner URL."""
import unittest

import postgres_tests


class RoleUrlTests(unittest.TestCase):
    def test_credentials_are_replaced_and_host_port_database_kept(self):
        self.assertEqual(
            postgres_tests.with_credentials(
                "postgres://postgres:secret@localhost:5432/omoba_test?sslmode=disable",
                "omoba_portal_test",
                "p@ss",
            ),
            "postgres://omoba_portal_test:p%40ss@localhost:5432/omoba_test?sslmode=disable",
        )

    def test_url_without_credentials_or_port_gains_them(self):
        self.assertEqual(
            postgres_tests.with_credentials("postgresql://db/omoba", "u", "p"),
            "postgresql://u:p@db/omoba",
        )

    def test_non_postgres_urls_are_rejected(self):
        with self.assertRaises(ValueError):
            postgres_tests.with_credentials("mysql://db/omoba", "u", "p")
        with self.assertRaises(ValueError):
            postgres_tests.with_credentials("postgres:///omoba", "u", "p")


if __name__ == "__main__":
    unittest.main()
