#!/usr/bin/env python3
"""Build/install, create a test profile, archive, migrate and restore Vincent Deck."""
import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
HOME = Path.home()
CONFIG = HOME / '.config/opendeck'
PLUGIN = 'dev.vincent.deck.sdPlugin'
DEVICE = 'sd-EL31L1A08599'
PROFILE = CONFIG / 'profiles' / DEVICE / 'Default.json'
NAMES = ['terminal', 'browser', 'screenshot', 'record', 'agent', 'clipboard',
         'night', 'lock', 'volume', 'mic', 'workspace', 'theme']
STATS_NAMES = ['cpu', 'memory', 'disk', 'network']
LEGACY = ['.local/bin/deck-icons', '.local/bin/deck-theme-cycle',
          '.config/opendeck/icons',
          '.config/opendeck/plugins/com.amansprojects.starterpack.sdPlugin/layouts',
          f'.config/opendeck/images/{DEVICE}/Default',
          f'.config/opendeck/profiles/{DEVICE}/Default.json']


def stopped():
    if subprocess.run(['pgrep', '-x', 'opendeck'], stdout=subprocess.DEVNULL).returncode == 0:
        raise RuntimeError('Stop OpenDeck first; profile edits require it to be stopped')


def manifest():
    actions = []
    for index, name in enumerate(NAMES + STATS_NAMES):
        panel = index >= 8
        title = {'cpu': 'CPU Stats', 'memory': 'Memory Stats', 'disk': 'Disk Stats',
                 'network': 'Network Stats'}.get(name, name.title())
        tooltip = ({'cpu': 'Live CPU graph; rotate to cycle power profiles',
                    'network': 'Live network graph; click for default profile, tap for speed test'}.get(name,
                    'Read-only live system graph') if name in STATS_NAMES else
                    'Toggle default agent console' if name == 'agent' else
                    'Rotate themes; click for performance profile; tap for wallpaper' if name == 'theme' else f'Vincent Deck {name}')
        action = dict(Name=title, UUID=f'dev.vincent.deck.{name}',
                      Icon=f'icons/{name}', States=[{'Image': f'icons/{name}'}],
                      Controllers=['Encoder' if panel else 'Keypad'],
                      SupportedInMultiActions=False, Tooltip=tooltip)
        if panel:
            rotate = {'volume': 'Adjust volume', 'mic': 'Adjust microphone',
                      'workspace': 'Switch workspace', 'theme': 'Cycle theme',
                      'cpu': 'Cycle power profile'}.get(name, '')
            push = {'volume': 'Next output device', 'mic': 'Next input device',
                     'workspace': 'Open menu', 'theme': 'Switch to performance profile',
                     'network': 'Switch to default profile'}.get(name, '')
            touch = {'volume': 'Toggle mute', 'mic': 'Toggle mute',
                     'theme': 'Next wallpaper', 'network': 'Run Omarchy speed test'}.get(name, push)
            action['Encoder'] = {'layout': 'layouts/panel.json',
                                  'TriggerDescription': {'Rotate': rotate, 'Push': push, 'Touch': touch}}
        actions.append(action)
    return dict(Name='Vincent Deck', Description='Theme-following CRT controls for Omarchy',
                Author='Vincent', Version='0.2.4', Category='Vincent Deck', Icon='icons/terminal',
                CodePathLin='x86_64-unknown-linux-gnu/bin/vincent-deck',
                CodePaths={'x86_64-unknown-linux-gnu': 'x86_64-unknown-linux-gnu/bin/vincent-deck'},
                OS=[{'Platform': 'linux'}], Actions=actions)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def install():
    subprocess.run(['cargo', 'build', '--release'], cwd=ROOT, check=True)
    bundle = ROOT / 'dist' / PLUGIN
    bundle.mkdir(parents=True, exist_ok=True)
    shutil.copytree(ROOT / 'assets', bundle, dirs_exist_ok=True)
    icons = bundle / 'icons'
    subprocess.run([ROOT / 'target/release/vincent-deck', '--render-samples', icons], check=True)
    binary = bundle / 'x86_64-unknown-linux-gnu/bin/vincent-deck'
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / 'target/release/vincent-deck', binary)
    write_json(bundle / 'manifest.json', manifest())
    target = CONFIG / 'plugins' / PLUGIN
    if target.exists():
        backup = HOME / '.local/state/vincent-deck' / f'plugin-{time.time_ns()}'
        shutil.copytree(target, backup)
    shutil.copytree(bundle, target, dirs_exist_ok=True, ignore=shutil.ignore_patterns('vincent-deck'))
    # Replace binary atomically even when an older plugin process is executing it.
    installed_binary = target / 'x86_64-unknown-linux-gnu/bin/vincent-deck'
    installed_binary.parent.mkdir(parents=True, exist_ok=True)
    temporary = installed_binary.with_suffix('.new')
    shutil.copy2(binary, temporary)
    temporary.replace(installed_binary)
    print(f'Installed {target}\nRestart OpenDeck to load the new plugin.')


def place_action(instance, definition, name, panel):
    action = copy.deepcopy(instance['action'])
    state = copy.deepcopy(instance['states'][0])
    state.update(image=f'plugins/{PLUGIN}/icons/{name}.png', show=False, text='')
    action.update(plugin=PLUGIN, uuid=definition['UUID'], name=definition['Name'],
                  tooltip=definition['Tooltip'], controllers=definition['Controllers'],
                  property_inspector='', supported_in_multi_actions=False,
                  icon=f'icons/{name}.png', states=[copy.deepcopy(state)],
                  disable_automatic_states=False, visible_in_action_list=True)
    encoder = action['encoder']
    encoder.update(background='', icon='', stack_color='', layout='layouts/panel.json' if panel else '$A0')
    encoder['trigger_description'] = dict(long_touch='', push='', rotate='', touch='')
    if panel:
        for key, value in definition['Encoder']['TriggerDescription'].items():
            encoder['trigger_description'][key.lower()] = value
    instance.update(action=action, settings={}, states=[state], current_state=0, children=None)


def migrated(profile):
    result = copy.deepcopy(profile)
    definitions = manifest()['Actions']
    for group, start in [('keys', 0), ('sliders', 8)]:
        if len(result[group]) != (8 if group == 'keys' else 4):
            raise RuntimeError(f'Unexpected {group} count')
        for offset, instance in enumerate(result[group]):
            definition = definitions[start + offset]
            name = NAMES[start + offset]
            place_action(instance, definition, name, bool(start))
    return result


def default_profile():
    if PROFILE.exists():
        return PROFILE
    lower = PROFILE.with_name('default.json')
    if lower.exists():
        return lower
    raise RuntimeError(f'Default profile not found in {PROFILE.parent}')


def stats_profile(profile):
    """Copy the profile and retain every key; assign stats only to copied dials."""
    result = copy.deepcopy(profile)
    if len(result['sliders']) != 4:
        raise RuntimeError('System Stats requires four dial panels')
    # Independent copies also protect keys if an in-memory template aliases slots.
    result['sliders'] = [copy.deepcopy(instance) for instance in result['sliders']]
    definitions = {action['UUID']: action for action in manifest()['Actions']}
    for instance, name in zip(result['sliders'], STATS_NAMES):
        place_action(instance, definitions[f'dev.vincent.deck.{name}'], name, True)
    return result


def create_stats_profile(name='System Stats'):
    stopped()
    if not name.strip() or name in ('.', '..') or any(char in name for char in '/\\'):
        raise RuntimeError('Profile name must be a nonempty filename without directory separators')
    source = default_profile()
    target = source.with_name(f'{name}.json')
    if any(path.stem.lower() == name.lower() for path in source.parent.glob('*.json')):
        raise RuntimeError(f'{name} profile already exists; refusing to overwrite it')
    data = stats_profile(json.loads(source.read_text()))
    # Retain copied keys' custom images as well as their action definitions.
    images = CONFIG / 'images' / DEVICE
    original_images = images / source.stem
    copied_images = images / target.stem
    if original_images.exists():
        if copied_images.exists():
            raise RuntimeError(f'{copied_images} already exists')
        shutil.copytree(original_images, copied_images)
    write_json(target, data)
    print(f'Created {target}; select {name} in OpenDeck. Default was not modified.')


def legacy_paths():
    paths = list(LEGACY)
    upper = HOME / f'.config/opendeck/profiles/{DEVICE}/Default.json'
    lower = upper.with_name('default.json')
    if not upper.exists() and lower.exists():
        paths = [path.replace(f'{DEVICE}/Default', f'{DEVICE}/default') for path in paths]
    return paths


def archive():
    destination = HOME / '.local/state/vincent-deck/backups' / str(time.time_ns())
    destination.mkdir(parents=True)
    records = []
    for relative in legacy_paths():
        source = HOME / relative
        records.append({'path': relative, 'present': source.exists() or source.is_symlink()})
        if source.exists() or source.is_symlink():
            target = destination / 'files' / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, target, symlinks=True)
            else:
                shutil.copy2(source, target, follow_symlinks=False)
    write_json(destination / 'archive.json', records)
    print(f'Rollback archive: {destination}')
    return destination


def rollback(destination):
    records = json.loads((destination / 'archive.json').read_text())
    for record in records:
        relative = Path(record['path'])
        allowed = set(LEGACY) | {f'.config/opendeck/profiles/{DEVICE}/default.json',
                                f'.config/opendeck/images/{DEVICE}/default'}
        if relative.is_absolute() or '..' in relative.parts or str(relative) not in allowed:
            raise RuntimeError('Invalid archive path')
        if record['present'] and not (destination / 'files' / relative).exists():
            raise RuntimeError(f'Incomplete archive: {relative}')
    subprocess.run(['pkill', '-x', 'opendeck'], check=False)
    for _ in range(50):
        if subprocess.run(['pgrep', '-x', 'opendeck'], stdout=subprocess.DEVNULL).returncode != 0:
            break
        time.sleep(.1)
    stopped()
    for record in records:
        target = HOME / record['path']
        if target.is_symlink() or target.is_file(): target.unlink()
        elif target.is_dir(): shutil.rmtree(target)
        if record['present']:
            source = destination / 'files' / record['path']
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir(): shutil.copytree(source, target, symlinks=True)
            else: shutil.copy2(source, target, follow_symlinks=False)
    subprocess.Popen(['opendeck'], start_new_session=True,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print('Legacy profile and dependencies restored; OpenDeck started.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['install', 'test-profile', 'stats-profile', 'migrate', 'archive', 'rollback'])
    parser.add_argument('--backup', type=Path)
    parser.add_argument('--name', default='System Stats', help='Name for the stats-profile command')
    args = parser.parse_args()
    if args.command == 'install': install()
    elif args.command == 'stats-profile': create_stats_profile(args.name)
    elif args.command == 'rollback':
        if not args.backup: parser.error('rollback requires --backup')
        rollback(args.backup)
    elif args.command == 'archive': archive()
    else:
        stopped()
        source = default_profile()
        data = migrated(json.loads(source.read_text()))
        if args.command == 'test-profile':
            target = source.with_name('Vincent Test.json')
            if target.exists(): raise RuntimeError(f'{target} already exists')
            write_json(target, data)
            print(f'Created {target}; select Vincent Test in OpenDeck for hardware review.')
        else:
            archive()
            write_json(source, data)
            print('Migrated Default. Start OpenDeck and run acceptance checks.')


if __name__ == '__main__': main()
