"""Exercise the migration/rollback file boundary with an isolated home directory."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('manage', Path(__file__).parents[1] / 'scripts/manage.py')
manage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manage)


class MigrationTests(unittest.TestCase):
    def test_migration_preserves_context_and_other_profile_fields(self):
        template = {'action': {'encoder': {}}, 'states': [{'image': 'old', 'text': 'old'}],
                    'context': 'unchanged', 'settings': {'old': True}, 'children': [], 'current_state': 2}
        profile = {'keys': [template] * 8, 'sliders': [template] * 4, 'infobars': ['preserved']}
        migrated = manage.migrated(profile)
        self.assertEqual(migrated['infobars'], ['preserved'])
        for instance in migrated['keys'] + migrated['sliders']:
            self.assertEqual(instance['context'], 'unchanged')
            self.assertEqual(instance['settings'], {})
            self.assertEqual(instance['action']['plugin'], manage.PLUGIN)
        self.assertEqual(migrated['sliders'][0]['action']['encoder']['layout'], 'layouts/panel.json')
        self.assertEqual(profile['keys'][0]['states'][0]['image'], 'old')

    def test_complete_archive_restores_deleted_dependencies_and_permissions(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            for relative in manage.LEGACY:
                path = home / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                # A file is enough to exercise the archive path scope independently
                # of the actual workstation's folder contents.
                path.write_text(relative)
                path.chmod(0o751)
            with patch.object(manage, 'HOME', home):
                backup = manage.archive()
                for relative in manage.LEGACY:
                    (home / relative).unlink()
                with patch.object(manage, 'stopped'), patch.object(manage.subprocess, 'run'), \
                     patch.object(manage.subprocess, 'Popen'), patch.object(manage.time, 'sleep'):
                    manage.rollback(backup)
                for relative in manage.LEGACY:
                    self.assertEqual((home / relative).read_text(), relative)
                    self.assertEqual((home / relative).stat().st_mode & 0o777, 0o751)
                self.assertEqual(len(json.loads((backup / 'archive.json').read_text())), len(manage.LEGACY))


if __name__ == '__main__': unittest.main()
