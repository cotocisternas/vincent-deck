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
    def test_stats_profile_preserves_original_keys_and_panels(self):
        template = {'action': {'encoder': {}}, 'states': [{'image': 'old', 'text': 'custom'}],
                    'context': 'unchanged', 'settings': {'old': True}, 'children': [], 'current_state': 2}
        profile = {'keys': [template] * 8, 'sliders': [template] * 4,
                   'infobars': ['preserved']}
        original = json.loads(json.dumps(profile))
        stats = manage.stats_profile(profile)
        self.assertEqual(profile, original)
        self.assertEqual(stats['keys'], original['keys'])
        self.assertEqual(stats['infobars'], original['infobars'])
        self.assertEqual([s['action']['uuid'] for s in stats['sliders']],
                         [f'dev.vincent.deck.{name}' for name in manage.STATS_NAMES])
        self.assertEqual([s['action']['encoder']['layout'] for s in stats['sliders']],
                         ['layouts/panel.json'] * 4)
        self.assertEqual(len(manage.manifest()['Actions']), 16)
        definitions = {a['UUID']: a for a in manage.manifest()['Actions']}
        self.assertEqual(definitions['dev.vincent.deck.cpu']['Encoder']['TriggerDescription']['Rotate'], 'Cycle power profile')
        self.assertEqual(definitions['dev.vincent.deck.network']['Encoder']['TriggerDescription']['Push'], 'Switch to default profile')
        self.assertEqual(definitions['dev.vincent.deck.network']['Encoder']['TriggerDescription']['Touch'], 'Run Omarchy speed test')
        self.assertEqual(definitions['dev.vincent.deck.theme']['Encoder']['TriggerDescription']['Push'], 'Switch to performance profile')
        self.assertEqual(definitions['dev.vincent.deck.theme']['Encoder']['TriggerDescription']['Touch'], 'Next wallpaper')
        for action, device in [('volume', 'output'), ('mic', 'input')]:
            triggers = definitions[f'dev.vincent.deck.{action}']['Encoder']['TriggerDescription']
            self.assertEqual(triggers['Push'], f'Next {device} device')
            self.assertEqual(triggers['Touch'], 'Toggle mute')
        self.assertEqual([a['UUID'] for a in manage.manifest()['Actions'][:12]],
                         [f'dev.vincent.deck.{name}' for name in manage.NAMES])

    def test_stats_profile_creation_keeps_lowercase_default_and_custom_key_images(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            config = home / '.config/opendeck'
            source = config / 'profiles' / manage.DEVICE / 'default.json'
            source.parent.mkdir(parents=True)
            template = {'action': {'encoder': {}}, 'states': [{'image': 'old'}], 'context': 'same'}
            data = {'keys': [template] * 8, 'sliders': [template] * 4}
            source.write_text(json.dumps(data))
            original = source.read_bytes()
            image = config / 'images' / manage.DEVICE / 'default' / 'Keypad.0.0' / '0.png'
            image.parent.mkdir(parents=True)
            image.write_bytes(b'custom key image')
            with patch.object(manage, 'CONFIG', config), \
                 patch.object(manage, 'PROFILE', source.with_name('Default.json')), \
                 patch.object(manage, 'stopped'):
                manage.create_stats_profile()
                with self.assertRaises(RuntimeError):
                    manage.create_stats_profile()
            self.assertEqual(source.read_bytes(), original)
            target = source.with_name('System Stats.json')
            self.assertEqual(json.loads(target.read_text())['keys'], data['keys'])
            self.assertEqual((config / 'images' / manage.DEVICE / 'System Stats' /
                              'Keypad.0.0' / '0.png').read_bytes(), image.read_bytes())

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
