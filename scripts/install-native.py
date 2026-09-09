#!/usr/bin/env python3
"""Build or install Cockpit's native application and command-line client."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


PROJECT_ROOT = Path(__file__).resolve().parents[1]
APPLICATION_ID = "dev.cockpit.app"
APPLICATION_NAME = "Cockpit"
BINARY_NAME = "cockpit-tauri"
CLI_BINARY_NAME = "cockpit"
LAUNCHER_NAME = "cockpit"
CLI_LAUNCHER_NAME = "cockpit-cli"
RECEIPT_NAME = "install.json"
DESKTOP_MARKER = "X-Cockpit-Installer=1"


class InstallError(RuntimeError):
    """Raised when an install would overwrite an unowned path or cannot continue."""


@dataclass(frozen=True)
class InstallPaths:
    data_home: Path
    bin_home: Path

    @property
    def app_root(self) -> Path:
        return self.data_home / "cockpit"

    @property
    def binary(self) -> Path:
        return self.app_root / "bin" / LAUNCHER_NAME

    @property
    def cli_binary(self) -> Path:
        return self.app_root / "bin" / CLI_LAUNCHER_NAME

    @property
    def launcher(self) -> Path:
        return self.bin_home / LAUNCHER_NAME

    @property
    def cli_launcher(self) -> Path:
        return self.bin_home / CLI_LAUNCHER_NAME

    @property
    def desktop(self) -> Path:
        return self.data_home / "applications" / f"{APPLICATION_ID}.desktop"

    @property
    def icon(self) -> Path:
        return self.data_home / "icons" / "hicolor" / "256x256" / "apps" / f"{APPLICATION_ID}.png"

    @property
    def receipt(self) -> Path:
        return self.app_root / RECEIPT_NAME


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build or install Cockpit's native application and command-line client without root privileges.",
        epilog="The default install uses XDG_DATA_HOME and XDG_BIN_HOME on Linux, or macOS Application Support plus ~/.local/bin.",
    )
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--build", action="store_true", help="build the release binary before installing (the default)")
    action.add_argument("--reuse", action="store_true", help="install existing target/{release,debug}/cockpit-tauri and cockpit binaries")
    parser.add_argument("--debug", action="store_true", help="use a Tauri debug build for faster repeat installs")
    parser.add_argument("--prefix", type=Path, help="install below PREFIX/bin and PREFIX/share instead of XDG paths")
    parser.add_argument("--uninstall", action="store_true", help="remove only files recorded by this installer's receipt")
    args = parser.parse_args(argv)
    if args.uninstall and (args.build or args.reuse or args.debug):
        parser.error("--uninstall cannot be combined with --build, --reuse, or --debug")
    return args


def install_paths(prefix: Path | None, environment: dict[str, str] | None = None) -> InstallPaths:
    environment = os.environ if environment is None else environment
    if prefix is not None:
        root = prefix.expanduser().resolve()
        return InstallPaths(data_home=root / "share", bin_home=root / "bin")
    home = environment.get("HOME")
    if not home:
        raise InstallError("HOME is required when --prefix is not set")
    if sys.platform == "darwin":
        default_data_home = Path(home) / "Library" / "Application Support"
    else:
        default_data_home = Path(home) / ".local" / "share"
    data_home = Path(environment.get("XDG_DATA_HOME", default_data_home)).expanduser().resolve()
    bin_home = Path(environment.get("XDG_BIN_HOME", Path(home) / ".local" / "bin")).expanduser().resolve()
    return InstallPaths(data_home=data_home, bin_home=bin_home)


def source_binary(project_root: Path, debug: bool) -> Path:
    profile = "debug" if debug else "release"
    return project_root / "target" / profile / BINARY_NAME


def source_cli_binary(project_root: Path, debug: bool) -> Path:
    profile = "debug" if debug else "release"
    return project_root / "target" / profile / CLI_BINARY_NAME


def source_icon(project_root: Path) -> Path:
    return project_root / "src-tauri" / "icons" / "icon.png"


def run_build(project_root: Path, debug: bool) -> None:
    tauri_command = ["bunx", "tauri", "build", "--no-bundle"]
    if debug:
        tauri_command.append("--debug")
    cli_command = ["cargo", "build", "-p", "cockpit-host", "--bin", CLI_BINARY_NAME]
    if not debug:
        cli_command.append("--release")
    try:
        subprocess.run(tauri_command, cwd=project_root, check=True)
        subprocess.run(cli_command, cwd=project_root, check=True)
    except FileNotFoundError as error:
        raise InstallError(f"{error.filename} was not found; install the repository's toolchain first") from error
    except subprocess.CalledProcessError as error:
        raise InstallError(f"native build failed with exit status {error.returncode}") from error


def _atomic_replace(destination: Path, writer, mode: int) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            writer(stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, destination)
        _sync_directory(destination.parent)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def _sync_directory(directory: Path) -> None:
    try:
        descriptor = os.open(directory, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    except OSError:
        return
    try:
        try:
            os.fsync(descriptor)
        except OSError:
            pass
    finally:
        os.close(descriptor)

def atomic_copy(source: Path, destination: Path, mode: int) -> None:
    def write(stream) -> None:
        with source.open("rb") as input_stream:
            shutil.copyfileobj(input_stream, stream, length=1024 * 1024)

    _atomic_replace(destination, write, mode)


def atomic_text(destination: Path, text: str, mode: int = 0o644) -> None:
    _atomic_replace(destination, lambda stream: stream.write(text.encode("utf-8")), mode)


def atomic_symlink(target: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.parent / f".{destination.name}.{os.getpid()}.tmp"
    temporary.unlink(missing_ok=True)
    try:
        os.symlink(target, temporary)
        os.replace(temporary, destination)
        _sync_directory(destination.parent)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def receipt_data(paths: InstallPaths, status: str, artifact_hashes: dict[str, str] | None = None) -> dict[str, object]:
    return {
        "schema": 2,
        "application_id": APPLICATION_ID,
        "status": status,
        "paths": {
            "binary": str(paths.binary),
            "launcher": str(paths.launcher),
            "cli_binary": str(paths.cli_binary),
            "cli_launcher": str(paths.cli_launcher),
            "desktop": str(paths.desktop),
            "icon": str(paths.icon),
        },
        "artifact_sha256": artifact_hashes or {},
    }


def legacy_receipt_paths(paths: InstallPaths) -> dict[str, str]:
    return {
        "binary": str(paths.binary),
        "launcher": str(paths.launcher),
        "desktop": str(paths.desktop),
        "icon": str(paths.icon),
    }

def load_receipt(paths: InstallPaths) -> dict[str, object] | None:
    if not paths.receipt.exists():
        return None
    try:
        receipt = json.loads(paths.receipt.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise InstallError(f"cannot read installer receipt at {paths.receipt}: {error}") from error
    if not isinstance(receipt, dict):
        raise InstallError(f"installer receipt at {paths.receipt} does not belong to this destination")
    expected = receipt_data(paths, status=str(receipt.get("status", "")))["paths"]
    legacy = legacy_receipt_paths(paths)
    current = receipt.get("schema") == 2 and receipt.get("paths") == expected
    previous = receipt.get("schema") == 1 and receipt.get("paths") == legacy
    if receipt.get("application_id") != APPLICATION_ID or not (current or previous):
        raise InstallError(f"installer receipt at {paths.receipt} does not belong to this destination")
    return receipt


def write_receipt(paths: InstallPaths, status: str, artifact_hashes: dict[str, str] | None = None) -> None:
    content = json.dumps(receipt_data(paths, status, artifact_hashes), indent=2, sort_keys=True) + "\n"
    atomic_text(paths.receipt, content)


def desktop_exec(path: Path) -> str:
    escaped = str(path).replace("%", "%%").replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$")
    return f'"{escaped}"'


def desktop_contents(paths: InstallPaths) -> str:
    return "\n".join(
        (
            "[Desktop Entry]",
            "Version=1.0",
            "Type=Application",
            f"Name={APPLICATION_NAME}",
            "Comment=Graphical client for local coding-agent sessions",
            f"Exec={desktop_exec(paths.launcher)}",
            f"TryExec={str(paths.launcher).replace(chr(92), chr(92) * 2)}",
            f"Icon={APPLICATION_ID}",
            "Terminal=false",
            "Categories=Development;IDE;",
            DESKTOP_MARKER,
            "",
        )
    )


def ensure_installable(paths: InstallPaths) -> None:
    receipt = load_receipt(paths)
    if receipt is not None:
        status = receipt.get("status")
        if status == "installing":
            if receipt.get("schema") == 1 and any(
                path.exists() or path.is_symlink() for path in (paths.cli_binary, paths.cli_launcher)
            ):
                raise InstallError("refusing to replace unrecognized legacy CLI files")
            return
        hashes = receipt.get("artifact_sha256")
        if status != "installed" or not isinstance(hashes, dict):
            raise InstallError(f"installer receipt at {paths.receipt} has an unrecognized status")
        legacy = receipt.get("schema") == 1
        expected = {
            "binary": paths.binary,
            "icon": paths.icon,
            "desktop": paths.desktop,
        }
        if not legacy:
            expected.update({"cli_binary": paths.cli_binary})
        changed = [name for name, path in expected.items() if not isinstance(hashes.get(name), str) or not path.is_file() or sha256(path) != hashes[name]]
        for name, launcher, target in (
            ("launcher", paths.launcher, paths.binary),
            ("cli_launcher", paths.cli_launcher, paths.cli_binary),
        ):
            if legacy and name == "cli_launcher":
                continue
            if not launcher.is_symlink() or launcher.resolve() != target:
                changed.append(name)
        if legacy and any(
            path.exists() or path.is_symlink() for path in (paths.cli_binary, paths.cli_launcher)
        ):
            changed.append("cli")
        if changed:
            raise InstallError("refusing to replace modified installed files: " + ", ".join(changed))
        return
    conflicts = [path for path in (paths.binary, paths.launcher, paths.cli_binary, paths.cli_launcher, paths.desktop, paths.icon) if path.exists() or path.is_symlink()]
    if conflicts:
        listed = ", ".join(str(path) for path in conflicts)
        raise InstallError(f"refusing to overwrite paths without an installer receipt: {listed}")
def install(paths: InstallPaths, binary: Path, cli_binary: Path, icon: Path) -> None:
    for candidate, label in ((binary, "native application"), (cli_binary, "Cockpit CLI")):
        if not candidate.is_file() or not os.access(candidate, os.X_OK):
            raise InstallError(f"{label} binary is missing or not executable: {candidate}")
    if not icon.is_file():
        raise InstallError(f"Tauri icon is missing: {icon}")
    ensure_installable(paths)
    write_receipt(paths, "installing")
    atomic_copy(binary, paths.binary, 0o755)
    atomic_copy(cli_binary, paths.cli_binary, 0o755)
    atomic_copy(icon, paths.icon, 0o644)
    atomic_text(paths.desktop, desktop_contents(paths))
    atomic_symlink(paths.binary, paths.launcher)
    atomic_symlink(paths.cli_binary, paths.cli_launcher)
    write_receipt(
        paths,
        "installed",
        {
            "binary": sha256(paths.binary),
            "cli_binary": sha256(paths.cli_binary),
            "icon": sha256(paths.icon),
            "desktop": sha256(paths.desktop),
        },
    )


def _remove_if_owned(path: Path, predicate) -> bool:
    if not (path.exists() or path.is_symlink()):
        return True
    if not predicate():
        print(f"kept unrecognized path: {path}", file=sys.stderr)
        return False
    path.unlink()
    return True


def uninstall(paths: InstallPaths) -> bool:
    receipt = load_receipt(paths)
    if receipt is None:
        raise InstallError(f"no installer receipt exists at {paths.receipt}")
    hashes = receipt.get("artifact_sha256")
    if receipt.get("status") != "installed" or not isinstance(hashes, dict):
        raise InstallError(f"installer receipt at {paths.receipt} does not describe a completed install")

    def hash_matches(name: str, path: Path) -> bool:
        expected = hashes.get(name)
        return isinstance(expected, str) and path.is_file() and sha256(path) == expected

    removed = True
    removed &= _remove_if_owned(paths.launcher, lambda: paths.launcher.is_symlink() and paths.launcher.resolve() == paths.binary)
    if isinstance(hashes.get("cli_binary"), str):
        removed &= _remove_if_owned(paths.cli_launcher, lambda: paths.cli_launcher.is_symlink() and paths.cli_launcher.resolve() == paths.cli_binary)
        removed &= _remove_if_owned(paths.cli_binary, lambda: hash_matches("cli_binary", paths.cli_binary))
    removed &= _remove_if_owned(paths.desktop, lambda: hash_matches("desktop", paths.desktop))
    removed &= _remove_if_owned(paths.icon, lambda: hash_matches("icon", paths.icon))
    removed &= _remove_if_owned(paths.binary, lambda: hash_matches("binary", paths.binary))
    if removed:
        paths.receipt.unlink(missing_ok=True)
    for directory in (paths.binary.parent, paths.app_root):
        try:
            directory.rmdir()
        except OSError:
            pass
    return bool(removed)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        paths = install_paths(args.prefix)
        if args.uninstall:
            complete = uninstall(paths)
            if complete:
                print(f"removed Cockpit files from {paths.data_home} and {paths.bin_home}")
            else:
                print(f"removed recognized Cockpit files; kept receipt at {paths.receipt}", file=sys.stderr)
            return 0 if complete else 1
        if not args.reuse:
            run_build(PROJECT_ROOT, args.debug)
        install(
            paths,
            source_binary(PROJECT_ROOT, args.debug),
            source_cli_binary(PROJECT_ROOT, args.debug),
            source_icon(PROJECT_ROOT),
        )
    except InstallError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(f"installed {APPLICATION_NAME} launcher: {paths.launcher}")
    print(f"installed {APPLICATION_NAME} CLI: {paths.cli_launcher}")
    print(f"desktop entry: {paths.desktop}")
    print(f"native binary: {paths.binary}")
    print(f"CLI binary: {paths.cli_binary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
