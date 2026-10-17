#!/usr/bin/env python3
"""Create an isolated Herdr/Cockpit fixture for the UI polish scenarios."""

import argparse
import http.client
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time

try:
    from . import fixture_resources
except ImportError:
    # Preserve importlib-based frozen-driver wrappers without changing sys.path.
    import importlib.util
    spec = importlib.util.spec_from_file_location("cockpit_fixture_resources", Path(__file__).with_name("fixture_resources.py"))
    fixture_resources = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture_resources)

REPO = Path(__file__).resolve().parents[2]
FIXTURE_PORT_SECONDS = 5
HERDR_SOCKET_SECONDS = 10
GATEWAY_READY_SECONDS = 30


def environment(root):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "COCKPIT_"))}
    for key in ("ENV", "BASH_ENV", "ZDOTDIR", "PI_CONFIG_FILES",
                "OMP_AUTH_BROKER_URL", "OMP_AUTH_BROKER_TOKEN"):
        env.pop(key, None)
    env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"),
               XDG_STATE_HOME=str(root / "state"), XDG_CACHE_HOME=str(root / "cache"),
               XDG_DATA_HOME=str(root / "data"),
               XDG_RUNTIME_DIR=str(root / "runtime"), TMPDIR=str(root / "tmp"),
               PI_CODING_AGENT_DIR=str(root / "omp-agent"),
               PI_CODING_AGENT_SESSION_DIR=str(root / "omp-sessions"),
               PI_NATIVES_DIR=str(root / "native-cache"),
               OMP_AUTH_BROKER_SNAPSHOT_TTL_MS="0",
               HERDR_CONFIG_PATH=str(root / "config/herdr/config.toml"),
               COCKPIT_CONFIG=str(root / "cockpit.toml"))
    manifest_path = root / fixture_resources.MANIFEST
    if manifest_path.exists():
        env["COCKPIT_FIXTURE_CREATION_ID"] = json.loads(manifest_path.read_text())["creation_id"]
    ledger_path = root / "runtime.json"
    if ledger_path.exists():
        ledger = json.loads(ledger_path.read_text())
        env.update(COCKPIT_HERDR_SESSION=ledger["session"], COCKPIT_HERDR_SOCKET=ledger["socket"])
    return env


def rpc(root, method, params):
    ledger = json.loads((root / "runtime.json").read_text())
    assert root.name.startswith("cpol-") and ledger["session"].startswith("polish-")
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(8)
        client.connect(ledger["socket"])
        client.sendall((json.dumps({"id": "polish-proof", "method": method, "params": params}) + "\n").encode())
        response = json.loads(client.makefile().readline())
        if "error" in response:
            raise RuntimeError(response)
        return response


def launch(root, name, argv, env):
    with (root / (name + ".log")).open("w") as output:
        child = subprocess.Popen(argv, cwd=REPO, env=env, stdin=subprocess.DEVNULL,
                                 stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    (root / (name + ".pid")).write_text(str(child.pid))
    fixture_resources.record_process(root, name, child.pid)
    return child


def log_tail(log, lines=20):
    try:
        return "\n".join(log.read_text(errors="replace").splitlines()[-lines:])
    except FileNotFoundError:
        return ""


def wait_ready(child, log, description, seconds, probe):
    deadline = time.monotonic() + seconds
    last_error = None
    while True:
        if child.poll() is not None:
            raise RuntimeError(
                f"{description}: {log.stem} exited with {child.returncode} before readiness\n{log_tail(log)}")
        try:
            ready = probe()
        except Exception as error:
            last_error = error
            ready = None
        if child.poll() is not None:
            raise RuntimeError(
                f"{description}: {log.stem} exited with {child.returncode} before readiness\n{log_tail(log)}")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            detail = f" (last error: {last_error})" if last_error else ""
            raise RuntimeError(
                f"{description}: not ready after {seconds}s{detail}\n{log_tail(log)}")
        if ready:
            return ready
        time.sleep(min(.1, remaining))


def gateway_origin(text):
    prefix = "listening http://127.0.0.1:"
    for line in text.splitlines():
        if line.startswith(prefix) and line[len(prefix):].isdigit():
            return line.removeprefix("listening ")
    return None


def http_ok(origin):
    connection = http.client.HTTPConnection(origin.removeprefix("http://"), timeout=1)
    try:
        connection.request("GET", "/")
        return connection.getresponse().status == 200
    finally:
        connection.close()


VIEWER_PLUGINS = ("herdr-file-viewer", "persiyanov.reviewr")
ACTION_LAUNCHER_PLUGIN = "nnex.action-launcher"


def copy_plugins(root, plugin_ids):
    """Copy selected installed plugins into the fixture's own registry.

    Writes only under the fixture root and never runs `herdr plugin`, which may reach
    the owner's registry. The owner's registry is only read.
    """
    owner_config = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "herdr"
    registry = json.loads((owner_config / "plugins.json").read_text())
    copied = []
    for entry in registry:
        if entry["plugin_id"] not in plugin_ids:
            continue
        source = Path(entry["plugin_root"])
        target = root / "plugins" / source.name
        shutil.copytree(source, target, symlinks=True)
        entry = json.loads(json.dumps(entry).replace(str(source), str(target)))
        entry["source"] = {"kind": "local"}
        copied.append(entry)
    missing = set(plugin_ids) - {entry["plugin_id"] for entry in copied}
    if missing:
        raise SystemExit(f"owner registry lacks plugins: {sorted(missing)}")
    (root / "config/herdr/plugins.json").write_text(json.dumps(copied, indent=2))


def configure_action_launcher(root):
    """Install harmless custom command acceptance data only in this fixture."""
    config_dir = root / "config/herdr/plugins/config" / ACTION_LAUNCHER_PLUGIN
    config_dir.mkdir(parents=True)
    actions = {"actions": [{
        "id": "fixture-proof",
        "title": "Fixture proof writes disposable marker",
        "command": ["sh", "-c", f"printf selected > {root}/evidence/launcher-selected"],
        "mode": "run",
        "cwd": "workspace",
    }, {
        "id": "fixture-service",
        "title": "Fixture service tab",
        "command": ["sh", "-c", "printf service-up; exec sleep 3600"],
        "mode": "tab",
        "cwd": "workspace",
    }]}
    (config_dir / "actions.json").write_text(json.dumps(actions, indent=2))
    (root / "config/herdr/config.toml").write_text(
        '[[keys.command]]\nkey = "prefix+alt+a"\ntype = "plugin_action"\n'
        f'command = "{ACTION_LAUNCHER_PLUGIN}.open"\n'
        'description = "Fixture launcher"\n\n'
        '[[keys.command]]\nkey = "alt+z"\ntype = "shell"\n'
        f'command = "printf direct > {root}/evidence/direct-selected"\n'
        'description = "Fixture direct binding"\n')


def start(with_plugins=False, with_action_launcher=False):
    root = Path(tempfile.mkdtemp(prefix="cpol-", dir="/tmp"))
    session = "polish-" + root.name[5:]
    for folder in ["config/herdr", "state", "cache", "data", "runtime", "tmp", "omp-agent", "omp-sessions", "native-cache", "repositories/sample", "plain/nested", "evidence", "www"]:
        (root / folder).mkdir(mode=0o700, parents=True)
    (root / "config/herdr/config.toml").write_text('')
    plugin_ids = list(VIEWER_PLUGINS) if with_plugins else []
    if with_action_launcher:
        plugin_ids.append(ACTION_LAUNCHER_PLUGIN)
    if plugin_ids:
        copy_plugins(root, plugin_ids)
    if with_action_launcher:
        configure_action_launcher(root)
    (root / "cockpit.toml").write_text(
        f'version = 1\nrepository_roots = ["{root}/repositories"]\n'
        f'worktree_root = "{root}/worktrees"\n'
        f'state_root = "{root}/cockpit-state"\n')
    fixture = root / "repositories/sample"
    (fixture / "README.md").write_text('---\ntitle: Polish fixture\n---\n# Sample project\n\n## Summary\n\nOriginal source coordinates survive rendered Markdown.\n\n## Detail\n\nSelect this text to leave a comment.\n')
    (fixture / "sample.txt").write_text(''.join(f'Original line {i}\n' for i in range(1, 81)))
    for args in [["init", "-b", "main"], ["add", "."], ["-c", "user.name=Polish fixture", "-c", "user.email=polish@example.invalid", "commit", "-m", "Initial fixture"]]:
        subprocess.run(["git", "-C", str(fixture), *args], check=True, stdout=subprocess.DEVNULL)
    (fixture / "sample.txt").write_text((fixture / "sample.txt").read_text().replace('Original line 5\n', 'Changed line five\nAdded review line\n'))
    (root / "plain/nested/notes.md").write_text('# Borrowed notes\n\nKeep this directory and file.\n')
    (root / "www/index.html").write_text('''<!doctype html><title>Polish annotation fixture</title>
<body style="margin:0;padding:60px;font:20px system-ui;height:2400px;background:#eef3f8">
<h1>Annotation fixture</h1><button id="counter" onclick="this.textContent=Number(this.textContent)+1" style="padding:20px">0</button>
<p id="subject" style="margin-top:60px;padding:35px;background:white;width:600px">Keep this note attached to the original paragraph.</p>
<h2 style="margin-top:900px" id="below">A second anchor below the fold</h2>''')
    ledger = {"root": str(root), "session": session, "socket": str(root / f"config/herdr/sessions/{session}/herdr.sock"),
              "protected": ["default", "pre-existing sessions and gateways"], "fixture": str(fixture)}
    (root / "runtime.json").write_text(json.dumps(ledger, indent=2))
    fixture_resources.create(root, session)
    try:
        env = environment(root)
        fixture_server = launch(root, "fixture", [sys.executable, "-u", "-c",
            "from http.server import ThreadingHTTPServer,SimpleHTTPRequestHandler; from functools import partial; import sys; "
            "server=ThreadingHTTPServer(('127.0.0.1',0),partial(SimpleHTTPRequestHandler,directory=sys.argv[1])); "
            "print(server.server_port,flush=True); server.serve_forever()", str(root / "www")], env)
        def fixture_port():
            lines = (root / "fixture.log").read_text().splitlines()
            return int(lines[0]) if lines else None
        port = wait_ready(fixture_server, root / "fixture.log", "annotation fixture port",
                          FIXTURE_PORT_SECONDS, fixture_port)
        ledger["fixture_url"] = f"http://127.0.0.1:{port}/"
        fixture_resources.atomic_json(root / "runtime.json", ledger)
        herdr = launch(root, "herdr", [shutil.which("herdr"), "--session", session, "server"], env)
        wait_ready(herdr, root / "herdr.log", "Herdr socket", HERDR_SOCKET_SECONDS,
                   lambda: Path(ledger["socket"]).exists())
        result = rpc(root, "workspace.create", {"cwd": str(fixture), "label": "Polish sample", "focus": True})
        (root / "workspace.json").write_text(json.dumps(result, indent=2))
        gateway = launch(root, "gateway", [str(REPO / "target/debug/cockpit"), "serve", "--herdr-session", session,
            "--herdr-socket", ledger["socket"], "--config", str(root / "cockpit.toml"), "--static-dir", str(REPO / "dist")], env)
        def gateway_ready():
            origin = gateway_origin((root / "gateway.log").read_text(errors="replace"))
            return origin if origin and http_ok(origin) else None
        ledger["gateway_url"] = wait_ready(
            gateway, root / "gateway.log", "Cockpit gateway", GATEWAY_READY_SECONDS, gateway_ready)
        fixture_resources.atomic_json(root / "runtime.json", ledger)
    except BaseException:
        print(f"fixture start failed; evidence retained at {root}; stop recorded processes with: "
              f"python3 {Path(__file__).resolve()} stop {root}", file=sys.stderr)
        raise
    print(root)


def stop(root):
    try:
        result = fixture_resources.shutdown(root)
    except (OSError, ValueError, KeyError) as error:
        result = {"state": "unknown", "evidence": f"ownership unproven: {error}", "effects": []}
    print(json.dumps(result, indent=2))
    print(f"Normal fixture shutdown observed {result['state']}; evidence retained at {root}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "stop", "rpc", "resource-manifest", "inspect", "scratch-inspect", "scratch-cleanup"])
    parser.add_argument("root", nargs="?", type=Path)
    parser.add_argument("method", nargs="?")
    parser.add_argument("params", nargs="?", default="{}")
    parser.add_argument("--with-plugins", action="store_true",
                        help="copy the owner's Files and Reviewr plugins into the fixture")
    parser.add_argument("--with-action-launcher", action="store_true",
                        help="copy the owner's action launcher and configure harmless marker and persistent tab actions")
    parser.add_argument("--creation-id")
    parser.add_argument("--scratch-path")
    args = parser.parse_args()
    if args.action == "start":
        start(args.with_plugins, args.with_action_launcher)
    elif args.action == "stop":
        stop(args.root)
    elif args.action == "rpc":
        print(json.dumps(rpc(args.root, args.method, json.loads(args.params)), indent=2))
    elif args.action == "resource-manifest":
        print(json.dumps(fixture_resources.registration(args.root), indent=2))
    elif args.action == "inspect":
        print(json.dumps(fixture_resources.shutdown(args.root, args.creation_id, inspect_only=True), indent=2))
    else:
        print(json.dumps(fixture_resources.cleanup_scratch(
            args.root, args.creation_id, args.scratch_path,
            inspect_only=args.action == "scratch-inspect"), indent=2))
