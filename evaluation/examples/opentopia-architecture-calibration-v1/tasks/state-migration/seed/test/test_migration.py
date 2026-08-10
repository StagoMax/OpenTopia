import json
import tempfile
import unittest
from pathlib import Path

from migration import plan_migration


class MigrationTests(unittest.TestCase):
    def test_normalizes_accounts(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root)
            (source / "users.json").write_text(json.dumps([{"id": "A", "name": "Name", "email": " X@Example.Test "}]), encoding="utf-8")
            (source / "sessions.json").write_text("[]", encoding="utf-8")
            plan = plan_migration(source)
            self.assertEqual(plan["accounts"], [{"account_id": "a", "display_name": "Name", "normalized_email": "x@example.test"}])


if __name__ == "__main__":
    unittest.main()
