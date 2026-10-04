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
    for index, name in enumerate(NAMES):
        panel = index >= 8
        action = dict(Name=name.title(), UUID=f'dev.vincent.deck.{name}',
                      Icon=f'icons/{name}', States=[{'Image': f'icons/{name}'}],
                      Controllers=['Encoder' if panel else 'Keypad'],
                      SupportedInMultiActions=False, Tooltip='Toggle default agent console' if name == 'agent' else f'Vincent Deck {name}')
        if panel:
            rotate = {'volume': 'Adjust volume', 'mic': 'Adjust microphone',
                      'workspace': 'Switch workspace', 'theme': 'Cycle theme'}[name]
            push = {'volume': 'Toggle mute', 'mic': 'Toggle mute',
                    'workspace': 'Open menu', 'theme': 'Next wallpaper'}[name]
            action['Encoder'] = {'layout': 'layouts/panel.json',
                                 'TriggerDescription': {'Rotate': rotate, 'Push': push, 'Touch': push}}
        actions.append(action)
    return dict(Name='Vincent Deck', Description='Theme-following CRT controls for Omarchy',
                Author='Vincent', Version='0.1.1', Category='Vincent Deck', Icon='icons/terminal',
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


def migrated(profile):
    result = copy.deepcopy(profile)
    definitions = manifest()['Actions']
    for group, start in [('keys', 0), ('sliders', 8)]:
        if len(result[group]) != (8 if group == 'keys' else 4):
            raise RuntimeError(f'Unexpected {group} count')
        for offset, instance in enumerate(result[group]):
            definition = definitions[start + offset]
            action = copy.deepcopy(instance['action'])
            name = NAMES[start + offset]
            state = copy.deepcopy(instance['states'][0])
            state.update(image=f'plugins/{PLUGIN}/icons/{name}.png', show=False, text='')
            action.update(plugin=PLUGIN, uuid=definition['UUID'], name=definition['Name'],
                          tooltip=definition['Tooltip'], controllers=definition['Controllers'],
                          property_inspector='', supported_in_multi_actions=False,
                          icon=f'icons/{name}.png', states=[copy.deepcopy(state)],
                          disable_automatic_states=False, visible_in_action_list=True)
            encoder = action['encoder']
            encoder.update(background='', icon='', stack_color='', layout='layouts/panel.json' if start else '$A0')
            encoder['trigger_description'] = dict(long_touch='', push='', rotate='', touch='')
            if start:
                for key, value in definition['Encoder']['TriggerDescription'].items():
                    encoder['trigger_description'][key.lower()] = value
            instance.update(action=action, settings={}, states=[state], current_state=0, children=None)
    return result


def archive():
    destination = HOME / '.local/state/vincent-deck/backups' / str(time.time_ns())
    destination.mkdir(parents=True)
    records = []
    for relative in LEGACY:
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
        if relative.is_absolute() or '..' in relative.parts or str(relative) not in LEGACY:
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
    parser.add_argument('command', choices=['install', 'test-profile', 'migrate', 'archive', 'rollback'])
    parser.add_argument('--backup', type=Path)
    args = parser.parse_args()
    if args.command == 'install': install()
    elif args.command == 'rollback':
        if not args.backup: parser.error('rollback requires --backup')
        rollback(args.backup)
    elif args.command == 'archive': archive()
    else:
        stopped()
        data = migrated(json.loads(PROFILE.read_text()))
        if args.command == 'test-profile':
            target = PROFILE.with_name('Vincent Test.json')
            if target.exists(): raise RuntimeError(f'{target} already exists')
            write_json(target, data)
            print(f'Created {target}; select Vincent Test in OpenDeck for hardware review.')
        else:
            archive()
            write_json(PROFILE, data)
            print('Migrated Default. Start OpenDeck and run acceptance checks.')


if __name__ == '__main__': main()
