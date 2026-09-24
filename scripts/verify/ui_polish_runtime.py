#!/usr/bin/env python3
"""Create an isolated Herdr/Cockpit fixture for the UI polish scenarios."""

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]


def environment(root):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "COCKPIT_"))}
    env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"),
               XDG_STATE_HOME=str(root / "state"), XDG_CACHE_HOME=str(root / "cache"),
               XDG_DATA_HOME=str(root / "data"),
               HERDR_CONFIG_PATH=str(root / "config/herdr/config.toml"),
               COCKPIT_CONFIG=str(root / "cockpit.toml"))
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
    return child.pid


def start():
    root = Path(tempfile.mkdtemp(prefix="cpol-", dir="/tmp"))
    session = "polish-" + root.name[5:]
    for folder in ["config/herdr", "state", "cache", "data", "repositories/sample", "plain/nested", "evidence", "www"]:
        (root / folder).mkdir(parents=True)
    (root / "config/herdr/config.toml").write_text('')
    (root / "cockpit.toml").write_text(
        f'version = 1\nrepository_roots = ["{root}/repositories"]\n'
        f'worktree_root = "{root}/worktrees"\ncompanion_root = "{root}/companions"\n'
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
    env = environment(root)
    launch(root, "fixture", [sys.executable, "-u", "-c",
        "from http.server import ThreadingHTTPServer,SimpleHTTPRequestHandler; from functools import partial; import sys; "
        "server=ThreadingHTTPServer(('127.0.0.1',0),partial(SimpleHTTPRequestHandler,directory=sys.argv[1])); "
        "print(server.server_port,flush=True); server.serve_forever()", str(root / "www")], env)
    for _ in range(50):
        lines = (root / "fixture.log").read_text().splitlines()
        if lines:
            ledger["fixture_url"] = f"http://127.0.0.1:{int(lines[0])}/"
            (root / "runtime.json").write_text(json.dumps(ledger, indent=2))
            break
        time.sleep(.1)
    launch(root, "herdr", [shutil.which("herdr"), "--session", session, "server"], env)
    for _ in range(100):
        if Path(ledger["socket"]).exists():
            break
        time.sleep(.1)
    result = rpc(root, "workspace.create", {"cwd": str(fixture), "label": "Polish sample", "focus": True})
    (root / "workspace.json").write_text(json.dumps(result, indent=2))
    launch(root, "gateway", [str(REPO / "target/debug/cockpit"), "serve", "--herdr-session", session,
        "--herdr-socket", ledger["socket"], "--config", str(root / "cockpit.toml"), "--static-dir", str(REPO / "dist")], env)
    print(root)


def stop(root):
    ledger = json.loads((root / "runtime.json").read_text())
    assert str(root) == ledger["root"] and root.name.startswith("cpol-")
    for name in ["native", "gateway", "tui", "fixture", "herdr"]:
        pidfile = root / (name + ".pid")
        if pidfile.exists():
            pid = int(pidfile.read_text())
            cmdline = Path(f"/proc/{pid}/cmdline")
            environ = Path(f"/proc/{pid}/environ")
            identity = cmdline.read_text() + environ.read_text() if cmdline.exists() and environ.exists() else ""
            if ledger["session"] in identity or str(root) in identity:
                os.killpg(pid, signal.SIGTERM)
    print(f"Stopped matching fixture processes; evidence retained at {root}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "stop", "rpc"])
    parser.add_argument("root", nargs="?", type=Path)
    parser.add_argument("method", nargs="?")
    parser.add_argument("params", nargs="?", default="{}")
    args = parser.parse_args()
    if args.action == "start":
        start()
    elif args.action == "stop":
        stop(args.root)
    else:
        print(json.dumps(rpc(args.root, args.method, json.loads(args.params)), indent=2))
