#!/usr/bin/env python3
"""Restart the installed Cockpit GUI on Linux without touching Herdr or agents.

Running GUIs are matched by executable (the installed binary, including a
replaced "(deleted)" image) and an argument-free command line, so `cockpit
serve` and CLI processes are left alone. Each window is closed through niri,
which runs Cockpit's own graceful shutdown; without niri the process gets
SIGTERM. Exit is proven with a pidfd before the replacement is started in its
own transient systemd user unit, so it outlives the invoking terminal.
"""

from __future__ import annotations

import glob
import json
import os
import select
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

EXIT_TIMEOUT_SECONDS = 20


def data_home() -> Path:
    return Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")


def bin_home() -> Path:
    return Path(os.environ.get("XDG_BIN_HOME") or Path.home() / ".local/bin")


def gui_processes(binary: Path) -> list[int]:
    # The installer renames a replaced binary to .cockpit.backup-* before
    # removing it, so running GUIs may report that name with " (deleted)".
    backup_prefix = f".{binary.name}.backup-"
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            exe = os.readlink(entry / "exe")
            argv = (entry / "cmdline").read_bytes().split(b"\0")[:-1]
        except OSError:
            continue
        path = Path(exe.removesuffix(" (deleted)"))
        if (
            path.parent == binary.parent
            and (path.name == binary.name or path.name.startswith(backup_prefix))
            and len(argv) == 1
        ):
            found.append(int(entry.name))
    return found


def niri_environment() -> dict[str, str] | None:
    if not shutil.which("niri"):
        return None
    environment = dict(os.environ)
    if not environment.get("NIRI_SOCKET"):
        runtime = os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
        sockets = glob.glob(f"{runtime}/niri.*.sock")
        if len(sockets) != 1:
            return None
        environment["NIRI_SOCKET"] = sockets[0]
    return environment


def niri_windows(environment: dict[str, str], pid: int) -> list[int]:
    result = subprocess.run(
        ["niri", "msg", "--json", "windows"], env=environment, capture_output=True, text=True, check=True
    )
    return [window["id"] for window in json.loads(result.stdout) if window.get("pid") == pid]


def stop(pid: int, niri: dict[str, str] | None) -> None:
    descriptor = os.pidfd_open(pid)
    try:
        windows = niri_windows(niri, pid) if niri else []
        if windows:
            for window in windows:
                subprocess.run(
                    ["niri", "msg", "action", "close-window", "--id", str(window)], env=niri, check=True
                )
            print(f"closed Cockpit window(s) {windows} of PID {pid}")
        else:
            signal.pidfd_send_signal(descriptor, signal.SIGTERM)
            print(f"no niri window for PID {pid}; sent SIGTERM (skips graceful shutdown)")
        ready, _, _ = select.select([descriptor], [], [], EXIT_TIMEOUT_SECONDS)
        if not ready:
            sys.exit(f"Cockpit PID {pid} did not exit within {EXIT_TIMEOUT_SECONDS}s; not starting a second instance")
    finally:
        os.close(descriptor)


def start(launcher: Path) -> None:
    if shutil.which("systemd-run"):
        unit = f"cockpit-restart-{int(time.time())}"
        subprocess.run(
            ["systemd-run", "--user", "--collect", "--quiet", f"--unit={unit}", str(launcher)], check=True
        )
        print(f"started Cockpit in user unit {unit}")
    else:
        process = subprocess.Popen(
            [str(launcher)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        print(f"started Cockpit as PID {process.pid}")


def main() -> None:
    if not sys.platform.startswith("linux"):
        sys.exit("restart-native.py supports Linux only")
    binary = data_home() / "cockpit/bin/cockpit"
    launcher = bin_home() / "cockpit"
    if not binary.is_file() or not launcher.exists():
        sys.exit(f"Cockpit is not installed at {binary}; run scripts/install-native.py first")
    pids = gui_processes(binary)
    niri = niri_environment() if pids else None
    for pid in pids:
        stop(pid, niri)
    if not pids:
        print("no running Cockpit GUI")
    start(launcher)


if __name__ == "__main__":
    main()
