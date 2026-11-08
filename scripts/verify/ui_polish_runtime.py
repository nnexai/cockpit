#!/usr/bin/env python3
"""Create an isolated Herdr/Cockpit fixture for the UI polish scenarios."""

import argparse
import hashlib
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
import uuid

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


def inherited_omp_agent_dir():
    """Resolve the default-profile local store without opening or copying credentials."""
    if any(os.environ.get(key) not in (None, "", "default") for key in ("OMP_PROFILE", "PI_PROFILE")):
        raise ValueError("OMP auth inheritance requires the default profile")
    if os.environ.get("PI_CONFIG_DIR") not in (None, "", ".omp"):
        raise ValueError("OMP auth inheritance does not support PI_CONFIG_DIR overrides")
    if os.environ.get("OMP_AUTH_BROKER_URL"):
        raise ValueError("OMP auth inheritance requires a local credential store, not a broker")
    home = Path(os.environ["HOME"])
    default_dir = home / ".omp" / "agent"
    agent_dir = Path(os.environ.get("PI_CODING_AGENT_DIR") or default_dir).resolve()
    data_home = os.environ.get("XDG_DATA_HOME")
    if agent_dir == default_dir.resolve() and data_home and (Path(data_home) / "omp").exists():
        raise ValueError("OMP auth inheritance does not support an XDG-migrated agent database")
    if not agent_dir.is_dir() or not (agent_dir / "agent.db").is_file():
        raise ValueError("OMP auth inheritance requires an existing agent directory and agent.db")
    if agent_dir.stat().st_uid != os.getuid() or (agent_dir / "agent.db").stat().st_uid != os.getuid():
        raise ValueError("OMP agent directory and database must be owned by the current user")
    return agent_dir


def environment(root, inherited_agent_dir=None):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "COCKPIT_"))}
    for key in ("ENV", "BASH_ENV", "ZDOTDIR", "PI_CONFIG_FILES",
                "OMP_AUTH_BROKER_URL", "OMP_AUTH_BROKER_TOKEN"):
        env.pop(key, None)
    if inherited_agent_dir is not None:
        env.pop("OMP_PROFILE", None)
        env.pop("PI_PROFILE", None)
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
    if inherited_agent_dir is not None:
        env["PI_CODING_AGENT_DIR"] = str(inherited_agent_dir)
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


def service_command(root, endpoint, *args):
    command = [str(REPO / "target/debug/cockpit"), "service",
               "--config", str(root / "cockpit.toml"),
               "--herdr", endpoint["executable"],
               "--herdr-session", endpoint["session"],
               "--herdr-socket", endpoint["socket"], *args]
    return subprocess.run(command, cwd=REPO, env=environment(root), capture_output=True,
                          text=True, check=True, timeout=40)


def backend_status(root, endpoint):
    return json.loads(service_command(root, endpoint, "status", "--json").stdout)


def running_backend(status, endpoint):
    if status.get("state") != "running":
        raise ValueError(f"backend status is {status.get('state', 'unknown')}")
    instance = status["instance"]
    instance_id = instance["instance_id"]
    if not isinstance(instance_id, str) or str(uuid.UUID(instance_id)) != instance_id:
        raise ValueError("backend instance is not a canonical UUID")
    if instance["herdr"] != endpoint:
        raise ValueError("backend Herdr endpoint differs from the fixture")
    return instance_id


def record_backend(root, ledger, herdr_executable):
    endpoint = {"executable": str(Path(herdr_executable).resolve()),
                "session": ledger["session"], "socket": ledger["socket"]}
    with fixture_resources.locked(root):
        manifest = fixture_resources.load(root)
        for name in ("gateway", "herdr"):
            entries = [item for item in manifest["processes"] if item["name"] == name]
            if len(entries) != 1 or fixture_resources.inspect_process(entries[0]["identity"], manifest)[0] != "live":
                raise ValueError(f"fixture {name} ownership is unproven")
        instance_id = running_backend(backend_status(root, endpoint), endpoint)
        ledger["backend"] = {
            "creation_id": manifest["creation_id"], "root": manifest["root"],
            "instance_id": instance_id, "herdr": endpoint,
            "config_sha256": hashlib.sha256((root / "cockpit.toml").read_bytes()).hexdigest(),
        }
        fixture_resources.atomic_json(root / "runtime.json", ledger)


def stop_backend(root):
    with fixture_resources.locked(root):
        manifest = fixture_resources.load(root)
        ledger = json.loads((root / "runtime.json").read_text())
        backend = ledger.get("backend")
        if not isinstance(backend, dict):
            raise ValueError("no backend creation receipt (pre-feature or incomplete fixture)")
        endpoint = backend["herdr"]
        expected_socket = str(root / f"config/herdr/sessions/{manifest['session']}/herdr.sock")
        if (manifest["boot_id"] != fixture_resources.boot_id()
                or backend["creation_id"] != manifest["creation_id"]
                or backend["root"] != manifest["root"]
                or endpoint["session"] != manifest["session"]
                or endpoint["socket"] != expected_socket
                or ledger["socket"] != expected_socket
                or not isinstance(endpoint["executable"], str)
                or not Path(endpoint["executable"]).is_absolute()
                or backend["config_sha256"] != hashlib.sha256((root / "cockpit.toml").read_bytes()).hexdigest()):
            raise ValueError("backend creation receipt does not match fixture identity")
        instance_id = running_backend(backend_status(root, endpoint), endpoint)
        if instance_id != backend["instance_id"]:
            raise ValueError("backend instance changed")
        service_command(root, endpoint, "stop", "--operator", "--expected-instance", instance_id)


def start(with_plugins=False, with_action_launcher=False, inherit_omp_auth=False, quiet_period_seconds=None):
    inherited_agent_dir = inherited_omp_agent_dir() if inherit_omp_auth else None
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
    if inherited_agent_dir is not None:
        overlay = root / "omp-smoke.yml"
        # The private fixture HOME would otherwise stop a fresh OMP at its setup wizard
        # before the launch prompt runs.
        overlay.write_text("autoResume: false\nstartup:\n  setupWizard: false\n")
    if inherited_agent_dir is not None or quiet_period_seconds is not None:
        with (root / "cockpit.toml").open("a") as config:
            config.write("\n[orchestration]\n")
            if quiet_period_seconds is not None:
                config.write(f"quiet_period_seconds = {quiet_period_seconds}\n")
            if inherited_agent_dir is not None:
                config.write('extra_args = ["--config", '
                             + json.dumps(str(overlay))
                             + ', "--no-extensions", "--no-skills", "--no-rules", "--no-context-files"]\n')
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
        env = environment(root, inherited_agent_dir)
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
        herdr_executable = shutil.which("herdr")
        if not herdr_executable:
            raise ValueError("Herdr executable not found")
        herdr = launch(root, "herdr", [herdr_executable, "--session", session, "server"], env)
        wait_ready(herdr, root / "herdr.log", "Herdr socket", HERDR_SOCKET_SECONDS,
                   lambda: Path(ledger["socket"]).exists())
        result = rpc(root, "workspace.create", {"cwd": str(fixture), "label": "Polish sample", "focus": True})
        (root / "workspace.json").write_text(json.dumps(result, indent=2))
        gateway = launch(root, "gateway", [str(REPO / "target/debug/cockpit"), "serve",
            "--herdr", herdr_executable, "--herdr-session", session, "--herdr-socket", ledger["socket"],
            "--config", str(root / "cockpit.toml"), "--static-dir", str(REPO / "dist")], env)
        def gateway_ready():
            origin = gateway_origin((root / "gateway.log").read_text(errors="replace"))
            return origin if origin and http_ok(origin) else None
        ledger["gateway_url"] = wait_ready(
            gateway, root / "gateway.log", "Cockpit gateway", GATEWAY_READY_SECONDS, gateway_ready)
        record_backend(root, ledger, herdr_executable)
    except BaseException:
        print(f"fixture start failed; evidence retained at {root}; stop recorded processes with: "
              f"python3 {Path(__file__).resolve()} stop {root}", file=sys.stderr)
        raise
    print(root)


def stop(root):
    try:
        stop_backend(root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        result = {"state": "unknown", "evidence": f"backend ownership or stop unproven: {error}",
                  "effects": []}
    else:
        try:
            result = fixture_resources.shutdown(root)
        except (OSError, ValueError, KeyError, TypeError) as error:
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
    parser.add_argument("--inherit-omp-auth", action="store_true",
                        help="start with the existing default-profile OMP agent directory (writable auth and cache state)")
    parser.add_argument("--quiet-period-seconds", type=int,
                        help="override the orchestration quiet period for this fixture")
    parser.add_argument("--creation-id")
    parser.add_argument("--scratch-path")
    args = parser.parse_args()
    if args.action == "start":
        start(args.with_plugins, args.with_action_launcher, args.inherit_omp_auth, args.quiet_period_seconds)
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
