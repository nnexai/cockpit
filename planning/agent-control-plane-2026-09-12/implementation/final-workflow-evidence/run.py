import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path('/tmp/cfinal12')
REPO = ROOT / 'repos/cockpit'
SESSION = 'cf12'
SOCKET = ROOT / 'config/herdr/sessions' / SESSION / 'herdr.sock'
HOST = Path('/home/nnex/dev/prj/cockpit/target/debug/cockpit')
env = {k: v for k, v in os.environ.items() if not k.startswith(('HERDR_', 'COCKPIT_'))}
env['GH_CONFIG_DIR'] = os.environ.get('GH_CONFIG_DIR', str(Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config'))) / 'gh'))
env.update(XDG_CONFIG_HOME=str(ROOT / 'config'), XDG_STATE_HOME=str(ROOT / 'state'), HERDR_CONFIG_PATH=str(ROOT / 'config/herdr/config.toml'), HERDR_SOCKET_PATH=str(SOCKET))

commands = {
    'server': ['herdr', '--session', SESSION, 'server'],
    'create-parent': ['herdr', '--session', SESSION, 'workspace', 'create', '--cwd', str(REPO), '--label', 'cockpit', '--focus'],
    'inspect': ['herdr', '--session', SESSION, 'workspace', 'list'],
    'schema': ['herdr', '--session', SESSION, 'api', 'schema'],
    'snapshot': ['herdr', '--session', SESSION, 'api', 'snapshot'],
    'reload': ['herdr', '--session', SESSION, 'server', 'reload-config'],
    'plugins': ['herdr', '--session', SESSION, 'plugin', 'list', '--json'],
    'read-context-shell': ['herdr', '--session', SESSION, 'pane', 'read', 'w2:p2', '--source', 'recent-unwrapped', '--lines', '16'],
    'read-review': ['herdr', '--session', SESSION, 'pane', 'read', 'w2:p5', '--source', 'visible', '--lines', '45'],
    'tui': ['herdr', '--session', SESSION],
    'stop': ['herdr', 'session', 'stop', SESSION, '--json'],
    'gateway': [str(HOST), 'serve', '--config', str(ROOT / 'cockpit.toml'), '--herdr-session', SESSION, '--herdr-socket', str(SOCKET), '--bind', '127.0.0.1:4197', '--static-dir', '/home/nnex/dev/prj/cockpit/dist'],
}
if len(sys.argv) != 2 or sys.argv[1] not in commands:
    raise SystemExit('Choose one: ' + ', '.join(commands))
raise SystemExit(subprocess.call(commands[sys.argv[1]], env=env, cwd=REPO))
