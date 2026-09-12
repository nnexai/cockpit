#!/usr/bin/env python3
"""Capture the current Cockpit browser surfaces in an owned disposable run.

The companion JavaScript drives the real browser UI. This wrapper owns the
Herdr server, gateway, fixture checkout, and Playwright profile and always
stops its own processes in the finally block.
"""

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path


ROOT = Path(os.environ.get("COCKPIT_ATLAS_ROOT", "/tmp/ca12-rerun"))
SESSION = os.environ.get("COCKPIT_ATLAS_SESSION", "ca12-rerun")
PORT = os.environ.get("COCKPIT_ATLAS_PORT", "4203")
REPO_SEED = Path(os.environ.get("COCKPIT_ATLAS_REPO_SEED", "/tmp/cfinal12/repos/cockpit"))
PROFILE = ROOT / "profile-atlas-app"
REPO = ROOT / "repos" / "cockpit"
SOCKET = ROOT / "config" / "herdr" / "sessions" / SESSION / "herdr.sock"
HOST = Path("/home/nnex/dev/prj/cockpit/target/debug/cockpit")
OUT = Path(__file__).resolve().parents[1]


def owned_env() -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "COCKPIT_"))}
    env.update(
        XDG_CONFIG_HOME=str(ROOT / "config"),
        XDG_STATE_HOME=str(ROOT / "state"),
        HERDR_CONFIG_PATH=str(ROOT / "config" / "herdr" / "config.toml"),
        HERDR_SOCKET_PATH=str(SOCKET),
    )
    return env


def run(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, cwd=REPO, env=owned_env(), check=check, text=True)


def stop_owned_browser() -> None:
    """Close the daemon and browser children tied to this run's profile."""
    owned: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="ignore")
        except OSError:
            continue
        if str(PROFILE) in command or "cliDaemon.js atlas-app" in command:
            owned.append(int(entry.name))
    for pid in owned:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    time.sleep(0.5)
    for pid in owned:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def bootstrap() -> tuple[subprocess.Popen[str], subprocess.Popen[str]]:
    if ROOT.exists():
        raise SystemExit(f"refusing to reuse existing capture root: {ROOT}")
    (ROOT / "repos").mkdir(parents=True)
    for part in ("config/herdr", "state", "worktrees", "companions", "cockpit-state", "evidence"):
        (ROOT / part).mkdir(parents=True)
    shutil.copytree(REPO_SEED, REPO)
    shutil.copy2("/tmp/cfinal12/config/herdr/plugins.json", ROOT / "config/herdr/plugins.json")
    shutil.copy2("/tmp/cfinal12/config/herdr/.plugins.lock", ROOT / "config/herdr/.plugins.lock")
    (ROOT / "cockpit.toml").write_text(
        f'''version = 1
repository_roots = ["{ROOT / "repos"}"]
worktree_root = "{ROOT / "worktrees"}"
companion_root = "{ROOT / "companions"}"
state_root = "{ROOT / "cockpit-state"}"

[[providers]]
id = "github"
base_url = "https://github.com"
executable = "/home/linuxbrew/.linuxbrew/bin/gh"

[browser]
playwright_cli = "/home/linuxbrew/.linuxbrew/bin/playwright-cli"
'''
    )
    herdr = subprocess.Popen(["herdr", "--session", SESSION, "server"], cwd=REPO, env=owned_env(), text=True)
    gateway = subprocess.Popen(
        [str(HOST), "serve", "--config", str(ROOT / "cockpit.toml"), "--herdr-session", SESSION,
         "--herdr-socket", str(SOCKET), "--bind", f"127.0.0.1:{PORT}", "--static-dir", "/home/nnex/dev/prj/cockpit/dist"],
        cwd=REPO,
        env=owned_env(),
        text=True,
    )
    run(["herdr", "--session", SESSION, "workspace", "create", "--cwd", str(REPO), "--label", "cockpit", "--focus"])
    return herdr, gateway


def main() -> int:
    herdr = gateway = None
    try:
        herdr, gateway = bootstrap()
        subprocess.run(["playwright-cli", "-s=atlas-app", "open", f"http://127.0.0.1:{PORT}/", "--browser=chrome", f"--profile={PROFILE}"], check=True)
        subprocess.run(["playwright-cli", "-s=atlas-app", "run-code", "--filename", str(Path(__file__).with_name("capture-app.js"))], check=True)
        return 0
    finally:
        subprocess.run(["playwright-cli", "-s=atlas-app", "close"], check=False)
        stop_owned_browser()
        run(["herdr", "session", "stop", SESSION, "--json"], check=False)
        for process in (gateway, herdr):
            if process and process.poll() is None:
                process.send_signal(signal.SIGINT)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.terminate()


if __name__ == "__main__":
    raise SystemExit(main())
