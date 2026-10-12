#!/usr/bin/env python3
"""Build or install Cockpit's native application and command-line client."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import secrets
import shutil
import stat
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
BUNDLE_NAME = "Cockpit.app"
CLI_BINARY_NAME = "cockpit"
LAUNCHER_NAME = "cockpit"
CLI_LAUNCHER_NAME = "cockpit-cli"
RECEIPT_NAME = "install.json"
PENDING_NAME = ".cockpit-install.pending.json"
DESKTOP_MARKER = "X-Cockpit-Installer=1"


class InstallError(RuntimeError):
    """Raised when an install would overwrite an unowned path or cannot continue."""


@dataclass(frozen=True)
class InstallPaths:
    data_home: Path
    bin_home: Path
    application: Path | None = None

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
    def gui_executable(self) -> Path:
        if self.application is not None:
            return self.application / "Contents" / "MacOS" / BINARY_NAME
        return self.binary

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

    @property
    def pending(self) -> Path:
        return self.data_home / PENDING_NAME


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build or install Cockpit's native application and command-line client without root privileges.",
        epilog="The default install uses XDG_DATA_HOME and XDG_BIN_HOME on Linux, or macOS Application Support plus ~/Applications/Cockpit.app and ~/.local/bin.",
    )
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--build", action="store_true", help="build the release binary before installing (the default)")
    action.add_argument("--reuse", action="store_true", help="install existing target/{release,debug}/cockpit-tauri and cockpit binaries")
    parser.add_argument("--debug", action="store_true", help="use a Tauri debug build for faster repeat installs")
    parser.add_argument("--prefix", type=Path, help="install below PREFIX/bin and PREFIX/share instead of XDG paths")
    parser.add_argument("--application-path", type=Path, help="macOS .app destination (use a disposable path for verification)")
    parser.add_argument("--uninstall", action="store_true", help="remove only files recorded by this installer's receipt")
    args = parser.parse_args(argv)
    if args.uninstall and (args.build or args.reuse or args.debug):
        parser.error("--uninstall cannot be combined with --build, --reuse, or --debug")
    return args


def _absolute_path(path: Path) -> Path:
    """Make a lexical absolute path without following a final destination symlink."""
    return Path(os.path.abspath(os.path.expanduser(os.fspath(path))))


def install_paths(prefix: Path | None, environment: dict[str, str] | None = None, application_path: Path | None = None) -> InstallPaths:
    environment = os.environ if environment is None else environment
    if prefix is not None:
        root = Path(os.path.expanduser(os.fspath(prefix))).resolve()
        application = None
        if sys.platform == "darwin":
            application = _absolute_path(application_path or (root / "Applications" / BUNDLE_NAME))
        return InstallPaths(data_home=root / "share", bin_home=root / "bin", application=application)
    home = environment.get("HOME")
    if not home:
        raise InstallError("HOME is required when --prefix is not set")
    if sys.platform == "darwin":
        default_data_home = Path(home) / "Library" / "Application Support"
    else:
        default_data_home = Path(home) / ".local" / "share"
    data_home = Path(environment.get("XDG_DATA_HOME", default_data_home)).expanduser().resolve()
    bin_home = Path(environment.get("XDG_BIN_HOME", Path(home) / ".local" / "bin")).expanduser().resolve()
    application = None
    if sys.platform == "darwin":
        application = _absolute_path(application_path or (Path(home) / "Applications" / BUNDLE_NAME))
    return InstallPaths(data_home=data_home, bin_home=bin_home, application=application)


def source_binary(project_root: Path, debug: bool) -> Path:
    profile = "debug" if debug else "release"
    return project_root / "target" / profile / BINARY_NAME


def source_cli_binary(project_root: Path, debug: bool) -> Path:
    profile = "debug" if debug else "release"
    return project_root / "target" / profile / CLI_BINARY_NAME


def source_bundle(project_root: Path, debug: bool) -> Path:
    profile = "debug" if debug else "release"
    return project_root / "target" / profile / "bundle" / "macos" / BUNDLE_NAME


def source_icon(project_root: Path) -> Path:
    return project_root / "src-tauri" / "icons" / "icon.png"


def run_build(project_root: Path, debug: bool) -> None:
    tauri_command = ["bunx", "tauri", "build"]
    if sys.platform != "darwin":
        tauri_command.append("--no-bundle")
    else:
        tauri_command.extend(["--bundles", "app"])
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


def _atomic_replace(destination: Path, writer, mode: int = 0o644) -> None:
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


def _is_present(path: Path) -> bool:
    return path.exists() or path.is_symlink()


def _safe_bundle_target(root: Path, entry: Path, target: str) -> Path:
    if os.path.isabs(target):
        raise InstallError(f"bundle symlink escapes its bundle: {entry}")
    resolved_root = root.resolve()
    resolved = (entry.parent / target).resolve(strict=False)
    try:
        resolved.relative_to(resolved_root)
    except ValueError as error:
        raise InstallError(f"bundle symlink escapes its bundle: {entry}") from error
    if not _is_present(resolved):
        raise InstallError(f"bundle symlink is dangling: {entry}")
    return resolved


def _bundle_entries(path: Path) -> list[dict[str, object]]:
    if path.is_symlink() or not path.is_dir():
        raise InstallError(f"macOS application bundle is not a directory: {path}")
    root = path.resolve()
    entries: list[dict[str, object]] = []

    def visit(current: Path, relative: Path) -> None:
        mode = stat.S_IMODE(os.lstat(current).st_mode)
        kind = stat.S_IFMT(os.lstat(current).st_mode)
        rel = "." if not relative.parts else relative.as_posix()
        if kind == stat.S_IFDIR:
            entries.append({"path": rel, "type": "directory", "mode": mode})
            children = sorted(current.iterdir(), key=lambda item: item.name)
            for child in children:
                visit(child, relative / child.name)
        elif kind == stat.S_IFREG:
            entries.append({"path": rel, "type": "file", "mode": mode, "sha256": sha256(current)})
        elif kind == stat.S_IFLNK:
            target = os.readlink(current)
            _safe_bundle_target(path, current, target)
            entries.append({"path": rel, "type": "symlink", "mode": mode, "target": target})
        else:
            raise InstallError(f"unsupported file type in macOS application bundle: {current}")

    visit(path, Path())
    return entries


def bundle_manifest(path: Path) -> list[dict[str, object]]:
    return _bundle_entries(path)


def bundle_manifest_hash(manifest: list[dict[str, object]]) -> str:
    encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def sha256_bundle(path: Path) -> str:
    return bundle_manifest_hash(bundle_manifest(path))


def bundle_identifier(path: Path) -> str | None:
    if path.is_symlink() or not path.is_dir():
        return None
    info = path / "Contents" / "Info.plist"
    try:
        if info.is_symlink() or not info.is_file():
            return None
        with info.open("rb") as stream:
            metadata = plistlib.load(stream)
    except (OSError, plistlib.InvalidFileException, TypeError, ValueError):
        return None
    if not isinstance(metadata, dict):
        return None
    identifier = metadata.get("CFBundleIdentifier")
    return identifier if isinstance(identifier, str) else None


def validate_bundle(path: Path, label: str = "macOS application bundle") -> list[dict[str, object]]:
    if bundle_identifier(path) != APPLICATION_ID:
        raise InstallError(f"{label} is missing or has the wrong bundle identifier: {path}")
    info = path / "Contents" / "Info.plist"
    try:
        with info.open("rb") as stream:
            metadata = plistlib.load(stream)
    except (OSError, plistlib.InvalidFileException, TypeError, ValueError) as error:
        raise InstallError(f"{label} has an unreadable Info.plist: {path}") from error
    executable_name = metadata.get("CFBundleExecutable") if isinstance(metadata, dict) else None
    if not isinstance(executable_name, str) or not executable_name or executable_name in {".", ".."} or "/" in executable_name or "\\" in executable_name:
        raise InstallError(f"{label} has an unsafe declared executable: {path}")
    if executable_name != BINARY_NAME:
        raise InstallError(f"{label} declares unexpected executable {executable_name!r}: {path}")
    executable = path / "Contents" / "MacOS" / executable_name
    if executable.is_symlink():
        _safe_bundle_target(path, executable, os.readlink(executable))
    if not executable.is_file() or not (os.stat(executable).st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)):
        raise InstallError(f"{label} executable is missing or not executable: {executable}")
    return _bundle_entries(path)
def receipt_paths(paths: InstallPaths, include_application: bool = True, include_binary: bool | None = None) -> dict[str, str]:
    if include_binary is None:
        include_binary = paths.application is None
    result: dict[str, str] = {}
    if include_binary:
        result["binary"] = str(paths.binary)
    result.update(
        {
            "launcher": str(paths.launcher),
            "cli_binary": str(paths.cli_binary),
            "cli_launcher": str(paths.cli_launcher),
            "desktop": str(paths.desktop),
            "icon": str(paths.icon),
        }
    )
    if include_application and paths.application is not None:
        result["application"] = str(paths.application)
    return result


def receipt_data(paths: InstallPaths, status: str, artifact_hashes: dict[str, str] | None = None, artifact_manifests: dict[str, list[dict[str, object]]] | None = None) -> dict[str, object]:
    return {
        "schema": 4 if paths.application is not None else 3,
        "application_id": APPLICATION_ID,
        "status": status,
        "paths": receipt_paths(paths),
        "artifact_sha256": artifact_hashes or {},
        "artifact_manifests": artifact_manifests or {},
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
    current_paths = receipt_paths(paths)
    historical_paths = receipt_paths(paths, include_binary=True)
    previous_paths = receipt_paths(paths, include_application=False, include_binary=True)
    legacy_paths = legacy_receipt_paths(paths)
    current = receipt.get("schema") == (4 if paths.application is not None else 3) and receipt.get("paths") == current_paths
    old_bundle = paths.application is not None and receipt.get("schema") == 3 and receipt.get("paths") == historical_paths
    previous = receipt.get("schema") == 2 and receipt.get("paths") == previous_paths
    legacy = receipt.get("schema") == 1 and receipt.get("paths") == legacy_paths
    if receipt.get("application_id") != APPLICATION_ID or not (current or old_bundle or previous or legacy):
        raise InstallError(f"installer receipt at {paths.receipt} does not belong to this destination")
    return receipt


def write_receipt(paths: InstallPaths, status: str, artifact_hashes: dict[str, str] | None = None, artifact_manifests: dict[str, list[dict[str, object]]] | None = None) -> None:
    content = json.dumps(receipt_data(paths, status, artifact_hashes, artifact_manifests), indent=2, sort_keys=True) + "\n"
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


def _path_descriptor(path: Path, application: bool = False) -> dict[str, object]:
    if not _is_present(path):
        return {"type": "absent"}
    info = os.lstat(path)
    mode = stat.S_IMODE(info.st_mode)
    if stat.S_ISLNK(info.st_mode):
        return {"type": "symlink", "mode": mode, "target": os.readlink(path)}
    if application:
        manifest = bundle_manifest(path)
        return {"type": "bundle", "mode": mode, "manifest": manifest, "sha256": bundle_manifest_hash(manifest)}
    if stat.S_ISREG(info.st_mode):
        return {"type": "file", "mode": mode, "sha256": sha256(path)}
    if stat.S_ISDIR(info.st_mode):
        return {"type": "directory", "mode": mode}
    raise InstallError(f"unsupported installed file type: {path}")


def _launcher_matches(path: Path, target: Path) -> bool:
    return path.is_symlink() and os.readlink(path) == str(target)


def _descriptor_matches(path: Path, descriptor: dict[str, object], application: bool = False) -> bool:
    try:
        current = _path_descriptor(path, application)
    except (OSError, InstallError):
        return False
    return current == descriptor




def _remove_known(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        for entry in path.rglob("*"):
            if entry.is_dir() and not entry.is_symlink():
                os.chmod(entry, stat.S_IMODE(os.lstat(entry).st_mode) | stat.S_IWUSR)
        os.chmod(path, stat.S_IMODE(os.lstat(path).st_mode) | stat.S_IWUSR)
        shutil.rmtree(path)


def _safe_transaction_path(path: Path, root: Path, token: str) -> bool:
    try:
        relative = path.relative_to(root)
    except ValueError:
        return False
    if not relative.parts or any(part in {".", ".."} for part in relative.parts):
        return False
    transaction_root = root / relative.parts[0]
    if not (transaction_root.name.endswith(token) or f"stage-{token}-" in transaction_root.name):
        return False
    current = root
    for index, part in enumerate(relative.parts):
        current = current / part
        if index < len(relative.parts) - 1 and current.is_symlink():
            return False
    return True


def _recover_pending(paths: InstallPaths) -> None:
    if not paths.pending.exists():
        return
    try:
        journal = json.loads(paths.pending.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise InstallError(f"cannot read pending installer transaction at {paths.pending}: {error}") from error
    if not isinstance(journal, dict) or journal.get("version") != 1 or journal.get("application_id") != APPLICATION_ID:
        raise InstallError(f"unrecognized pending installer transaction at {paths.pending}")
    token = journal.get("token")
    targets = journal.get("targets")
    if not isinstance(token, str) or not token or not isinstance(targets, dict):
        raise InstallError(f"unrecognized pending installer transaction at {paths.pending}")
    expected = receipt_paths(paths)
    allowed = set(expected) | {"receipt"}
    if paths.application is not None and "binary" in targets:
        allowed.add("binary")
    if "legacy_binary" in targets:
        allowed.add("legacy_binary")
    if set(targets) != allowed:
        raise InstallError(f"pending installer transaction does not belong to this destination: {paths.pending}")
    for name, record in targets.items():
        expected_target = str(paths.receipt) if name == "receipt" else str(paths.binary) if name in {"binary", "legacy_binary"} else expected.get(name)
        if not isinstance(record, dict) or record.get("target") != expected_target:
            raise InstallError(f"pending installer transaction has an unexpected target: {name}")
        target = Path(record["target"])
        staging = Path(record["staging"])
        backup = Path(record["backup"])
        if not isinstance(record.get("before"), dict) or not isinstance(record.get("after"), dict):
            raise InstallError(f"pending installer transaction has invalid descriptors: {name}")
        if not _safe_transaction_path(staging, target.parent, token):
            raise InstallError(f"pending installer transaction has an unsafe staging path: {staging}")
        if not _safe_transaction_path(backup, target.parent, token):
            raise InstallError(f"pending installer transaction has an unsafe backup path: {backup}")
        application = name == "application"
        if _is_present(staging) and not _descriptor_matches(staging, record["after"], application):
            raise InstallError(f"pending installer transaction has a modified staged artifact: {name}")
        if _is_present(backup) and not _descriptor_matches(backup, record["before"], application):
            raise InstallError(f"pending installer transaction has a modified backup artifact: {name}")
    status = journal.get("status")
    if status not in {"publishing", "rolled_back", "committed", "recovery_required"}:
        raise InstallError(f"unrecognized pending installer transaction status: {status}")
    if status == "recovery_required":
        raise InstallError(f"installer publication needs manual recovery; refusing to touch {paths.pending}")
    if status == "committed":
        for name, record in targets.items():
            if not _descriptor_matches(Path(record["target"]), record["after"], name == "application"):
                raise InstallError(f"completed installer transaction has a modified artifact: {name}")
    else:
        # A publication interruption is rolled back only when every target is
        # visibly either its recorded old generation, its staged generation,
        # or absent with its verified backup still present.
        for name, record in targets.items():
            target = Path(record["target"])
            before = record["before"]
            after = record["after"]
            current = _path_descriptor(target, name == "application")
            backup = Path(record["backup"])
            interrupted_backup = current.get("type") == "absent" and _is_present(backup)
            if not (_descriptor_matches(target, before, name == "application") or _descriptor_matches(target, after, name == "application") or interrupted_backup):
                raise InstallError(f"interrupted installer transaction has an unknown artifact state: {name}")
        if status == "publishing":
            for name, record in reversed(list(targets.items())):
                target = Path(record["target"])
                before = record["before"]
                after = record["after"]
                backup = Path(record["backup"])
                current = _path_descriptor(target, name == "application")
                if current == before:
                    pass
                elif current == after:
                    if _is_present(backup):
                        _remove_known(target)
                        os.replace(backup, target)
                    elif before.get("type") == "absent":
                        _remove_known(target)
                    else:
                        raise InstallError(f"interrupted transaction lacks backup for {name}")
                elif current.get("type") == "absent" and _is_present(backup):
                    os.replace(backup, target)
                else:
                    raise InstallError(f"interrupted transaction has an unrecognized state for {name}")
                _sync_directory(target.parent)
        for name, record in targets.items():
            if not _descriptor_matches(Path(record["target"]), record["before"], name == "application"):
                raise InstallError(f"installer rollback did not restore {name}")
    for record in targets.values():
        staging = Path(record["staging"])
        backup = Path(record["backup"])
        if _is_present(staging):
            _remove_known(staging)
        if _is_present(backup):
            _remove_known(backup)
    paths.pending.unlink(missing_ok=True)
    _sync_directory(paths.pending.parent)

def _canonical_destination(path: Path) -> Path:
    return path.parent.resolve() / path.name


def ensure_application_destination(paths: InstallPaths, receipt: dict[str, object] | None = None) -> None:
    if paths.application is None:
        return
    if paths.application.is_symlink():
        raise InstallError(f"refusing to replace unrecognized macOS application: {paths.application}")
    destination = _canonical_destination(paths.application)
    managed = (paths.binary, paths.cli_binary, paths.launcher, paths.cli_launcher, paths.desktop, paths.icon, paths.receipt, paths.pending)
    for managed_path in managed:
        other = _canonical_destination(managed_path)
        try:
            destination.relative_to(other)
            overlap = True
        except ValueError:
            try:
                other.relative_to(destination)
                overlap = True
            except ValueError:
                overlap = False
        if overlap:
            raise InstallError(f"macOS application destination overlaps managed path: {paths.application}")
    if not _is_present(paths.application):
        return
    owns_application = bool(
        receipt
        and (
            (receipt.get("schema") == 4 and receipt.get("paths") == receipt_paths(paths))
            or (receipt.get("schema") == 3 and receipt.get("paths") == receipt_paths(paths, include_binary=True))
        )
    )
    if not owns_application or bundle_identifier(paths.application) != APPLICATION_ID:
        raise InstallError(f"refusing to replace unrecognized macOS application: {paths.application}")


def ensure_installable(paths: InstallPaths) -> None:
    _recover_pending(paths)
    receipt = load_receipt(paths)
    ensure_application_destination(paths, receipt)
    if receipt is not None:
        status = receipt.get("status")
        if status != "installed":
            raise InstallError(f"installer receipt at {paths.receipt} has an interrupted or unrecognized status")
        hashes = receipt.get("artifact_sha256")
        if not isinstance(hashes, dict):
            raise InstallError(f"installer receipt at {paths.receipt} has no ownership manifest")
        legacy = receipt.get("schema") == 1
        receipt_paths_value = receipt.get("paths")
        raw_gui_owned = isinstance(receipt_paths_value, dict) and "binary" in receipt_paths_value
        expected: dict[str, Path] = {"icon": paths.icon, "desktop": paths.desktop}
        if raw_gui_owned:
            expected["binary"] = paths.binary
        if not legacy:
            expected["cli_binary"] = paths.cli_binary
        if paths.application is not None and receipt.get("schema") in {3, 4}:
            expected["application"] = paths.application
        changed: list[str] = []
        for name, path in expected.items():
            expected_hash = hashes.get(name)
            if name == "application":
                manifest = receipt.get("artifact_manifests", {}).get(name) if isinstance(receipt.get("artifact_manifests"), dict) else None
                matches = isinstance(expected_hash, str) and isinstance(manifest, list) and path.is_dir() and not path.is_symlink() and bundle_manifest(path) == manifest and bundle_manifest_hash(manifest) == expected_hash
            else:
                matches = isinstance(expected_hash, str) and path.is_file() and not path.is_symlink() and sha256(path) == expected_hash
            if not matches:
                changed.append(name)
        gui_target = paths.binary if raw_gui_owned else paths.gui_executable
        for name, launcher, target in (("launcher", paths.launcher, gui_target), ("cli_launcher", paths.cli_launcher, paths.cli_binary)):
            if legacy and name == "cli_launcher":
                continue
            if not _launcher_matches(launcher, target):
                changed.append(name)
        if paths.application is not None and not raw_gui_owned and _is_present(paths.binary):
            changed.append("legacy_binary")
        if legacy and (_is_present(paths.cli_binary) or _is_present(paths.cli_launcher)):
            changed.append("cli")
        if changed:
            raise InstallError("refusing to replace modified installed files: " + ", ".join(dict.fromkeys(changed)))
        return
    conflicts = [path for path in (paths.binary, paths.launcher, paths.cli_binary, paths.cli_launcher, paths.desktop, paths.icon) if _is_present(path)]
    if paths.application is not None and _is_present(paths.application):
        conflicts.append(paths.application)
    if conflicts:
        listed = ", ".join(str(path) for path in conflicts)
        raise InstallError(f"refusing to overwrite paths without an installer receipt: {listed}")


def _copy_bundle(source: Path, destination: Path) -> list[dict[str, object]]:
    manifest = validate_bundle(source)
    destination.mkdir(parents=True, exist_ok=False, mode=0o700)
    directories: list[tuple[Path, int]] = []
    for entry in manifest:
        relative = Path(str(entry["path"]))
        if relative == Path("."):
            directories.append((destination, int(entry["mode"])))
            continue
        source_entry = source / relative
        destination_entry = destination / relative
        kind = entry["type"]
        if kind == "directory":
            destination_entry.mkdir(mode=0o700)
            directories.append((destination_entry, int(entry["mode"])))
        elif kind == "file":
            destination_entry.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            shutil.copyfile(source_entry, destination_entry)
            os.chmod(destination_entry, int(entry["mode"]))
        elif kind == "symlink":
            destination_entry.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            os.symlink(str(entry["target"]), destination_entry)
        else:
            raise InstallError(f"unsupported bundle entry: {source_entry}")
    for directory, mode in sorted(directories, key=lambda item: len(item[0].parts), reverse=True):
        os.chmod(directory, mode)
    return bundle_manifest(destination)


def _stage_file(source: Path, destination: Path, mode: int) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    atomic_copy(source, destination, mode)


def _stage_symlink(target: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    os.symlink(target, destination)


def _write_journal(path: Path, journal: dict[str, object]) -> None:
    atomic_text(path, json.dumps(journal, indent=2, sort_keys=True) + "\n")


def install(paths: InstallPaths, binary: Path, cli_binary: Path, icon: Path, bundle: Path | None = None) -> None:
    for candidate, label in ((binary, "native application"), (cli_binary, "Cockpit CLI")):
        if not candidate.is_file() or not os.access(candidate, os.X_OK):
            raise InstallError(f"{label} binary is missing or not executable: {candidate}")
    if not icon.is_file():
        raise InstallError(f"Tauri icon is missing: {icon}")
    source_manifest: list[dict[str, object]] | None = None
    if paths.application is not None:
        if bundle is None:
            raise InstallError("macOS application bundle is required for this install")
        source_manifest = validate_bundle(bundle)
        executable = bundle / "Contents" / "MacOS" / BINARY_NAME
        if sha256(executable) != sha256(binary):
            raise InstallError("macOS application executable does not match the native binary")
    ensure_installable(paths)
    existing_receipt = load_receipt(paths)
    raw_gui_migration = bool(
        paths.application is not None
        and existing_receipt is not None
        and isinstance(existing_receipt.get("paths"), dict)
        and "binary" in existing_receipt["paths"]
    )
    paths.data_home.mkdir(parents=True, exist_ok=True)
    token = f"{os.getpid()}-{secrets.token_hex(8)}"
    bundle_stage_root: Path | None = None
    stages: dict[str, Path] = {}
    targets: dict[str, Path] = {}
    if paths.application is None:
        targets["binary"] = paths.binary
    elif raw_gui_migration:
        targets["legacy_binary"] = paths.binary
    targets.update({"cli_binary": paths.cli_binary, "icon": paths.icon, "desktop": paths.desktop})
    if paths.application is not None:
        targets["application"] = paths.application
    targets.update({"launcher": paths.launcher, "cli_launcher": paths.cli_launcher})
    targets["receipt"] = paths.receipt
    stage_roots: dict[Path, Path] = {}
    for name, target in targets.items():
        if name == "application":
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        stage_root = stage_roots.get(target.parent)
        if stage_root is None:
            stage_root = Path(tempfile.mkdtemp(prefix=f".cockpit-stage-{token}-", dir=target.parent))
            stage_roots[target.parent] = stage_root
        stages[name] = stage_root / name
    if paths.application is not None:
        assert bundle is not None
        paths.application.parent.mkdir(parents=True, exist_ok=True)
        bundle_stage_root = Path(tempfile.mkdtemp(prefix=f".{paths.application.name}.stage-{token}-", dir=paths.application.parent))
        stages["application"] = bundle_stage_root / BUNDLE_NAME
    try:
        if "binary" in stages:
            _stage_file(binary, stages["binary"], 0o755)
        _stage_file(cli_binary, stages["cli_binary"], 0o755)
        _stage_file(icon, stages["icon"], 0o644)
        atomic_text(stages["desktop"], desktop_contents(paths))
        _stage_symlink(paths.gui_executable, stages["launcher"])
        _stage_symlink(paths.cli_binary, stages["cli_launcher"])
        application_manifest: list[dict[str, object]] | None = None
        if paths.application is not None:
            assert source_manifest is not None and bundle is not None
            staged_manifest = _copy_bundle(bundle, stages["application"])
            if staged_manifest != source_manifest:
                raise InstallError("macOS application bundle changed while staging")
            validate_bundle(stages["application"])
            staged_executable = stages["application"] / "Contents" / "MacOS" / BINARY_NAME
            if sha256(staged_executable) != sha256(binary):
                raise InstallError("staged macOS application executable does not match staged native binary")
            application_manifest = staged_manifest
        artifact_hashes = {
            "cli_binary": sha256(stages["cli_binary"]),
            "icon": sha256(stages["icon"]),
            "desktop": sha256(stages["desktop"]),
        }
        if "binary" in stages:
            artifact_hashes["binary"] = sha256(stages["binary"])
        artifact_manifests: dict[str, list[dict[str, object]]] = {}
        if application_manifest is not None:
            artifact_hashes["application"] = bundle_manifest_hash(application_manifest)
            artifact_manifests["application"] = application_manifest
        receipt = receipt_data(paths, "installed", artifact_hashes, artifact_manifests)
        atomic_text(stages["receipt"], json.dumps(receipt, indent=2, sort_keys=True) + "\n")

        journal_targets: dict[str, dict[str, object]] = {}
        for name, target in targets.items():
            application = name == "application"
            before = _path_descriptor(target, application)
            after = _path_descriptor(stages[name], application)
            backup = target.parent / f".{target.name}.backup-{token}"
            journal_targets[name] = {"target": str(target), "staging": str(stages[name]), "backup": str(backup), "before": before, "after": after}
        journal: dict[str, object] = {"version": 1, "application_id": APPLICATION_ID, "token": token, "status": "publishing", "targets": journal_targets}
        _write_journal(paths.pending, journal)
        published: list[str] = []
        try:
            for name, record in journal_targets.items():
                target = Path(record["target"])
                backup = Path(record["backup"])
                target.parent.mkdir(parents=True, exist_ok=True)
                published.append(name)
                if _is_present(target):
                    os.replace(target, backup)
                if record["after"].get("type") != "absent":
                    os.replace(Path(record["staging"]), target)
                _sync_directory(target.parent)
                _write_journal(paths.pending, journal)
            journal["status"] = "committed"
            _write_journal(paths.pending, journal)
            try:
                for record in journal_targets.values():
                    backup = Path(record["backup"])
                    if _is_present(backup):
                        _remove_known(backup)
                for stage_root in stage_roots.values():
                    if _is_present(stage_root):
                        _remove_known(stage_root)
                if bundle_stage_root is not None and _is_present(bundle_stage_root):
                    _remove_known(bundle_stage_root)
                paths.pending.unlink(missing_ok=True)
                _sync_directory(paths.pending.parent)
            except OSError:
                # The committed receipt remains valid; recovery will verify
                # the generation before cleaning these private leftovers.
                pass
        except BaseException as error:
            rollback_ok = True
            try:
                for name, record in journal_targets.items():
                    application = name == "application"
                    staging = Path(record["staging"])
                    backup = Path(record["backup"])
                    if _is_present(staging) and not _descriptor_matches(staging, record["after"], application):
                        raise InstallError(f"modified staged artifact: {name}")
                    if _is_present(backup) and not _descriptor_matches(backup, record["before"], application):
                        raise InstallError(f"modified backup artifact: {name}")
            except (OSError, InstallError):
                rollback_ok = False
            for name in (reversed(published) if rollback_ok else ()):
                record = journal_targets[name]
                target = Path(record["target"])
                try:
                    backup = Path(record["backup"])
                    current = _path_descriptor(target, name == "application")
                    if current == record["before"] and not _is_present(backup):
                        continue
                    if current != record["after"] and current != record["before"] and not (current["type"] == "absent" and _is_present(backup)):
                        rollback_ok = False
                        break
                    if _is_present(backup):
                        if _is_present(target):
                            _remove_known(target)
                        os.replace(backup, target)
                    elif record["before"].get("type") == "absent":
                        if _is_present(target):
                            _remove_known(target)
                    else:
                        rollback_ok = False
                        break
                except (OSError, InstallError):
                    rollback_ok = False
                    break
                _sync_directory(target.parent)
            journal["status"] = "rolled_back" if rollback_ok else "recovery_required"
            _write_journal(paths.pending, journal)
            if rollback_ok:
                for record in journal_targets.values():
                    backup = Path(record["backup"])
                    if _is_present(backup):
                        _remove_known(backup)
                for stage_root in stage_roots.values():
                    if _is_present(stage_root):
                        _remove_known(stage_root)
                if bundle_stage_root is not None and _is_present(bundle_stage_root):
                    _remove_known(bundle_stage_root)
            raise InstallError(f"native publication failed; {'previous installation restored' if rollback_ok else 'recovery record retained at ' + str(paths.pending)}: {error}") from error
    except InstallError:
        raise
    except (OSError, shutil.Error) as error:
        raise InstallError(f"cannot stage native installation: {error}") from error
    finally:
        if not paths.pending.exists():
            for stage_root in stage_roots.values():
                if _is_present(stage_root):
                    _remove_known(stage_root)
            if bundle_stage_root is not None and _is_present(bundle_stage_root):
                _remove_known(bundle_stage_root)

def _owned_artifacts(paths: InstallPaths, receipt: dict[str, object]) -> list[tuple[str, Path, object]]:
    hashes = receipt.get("artifact_sha256")
    if not isinstance(hashes, dict):
        raise InstallError(f"installer receipt at {paths.receipt} has no ownership manifest")
    receipt_paths_value = receipt.get("paths")
    raw_gui_owned = paths.application is None or (isinstance(receipt_paths_value, dict) and "binary" in receipt_paths_value)
    owned: list[tuple[str, Path, object]] = [("launcher", paths.launcher, "launcher")]
    if raw_gui_owned:
        owned.append(("binary", paths.binary, hashes.get("binary")))
    owned.extend((("desktop", paths.desktop, hashes.get("desktop")), ("icon", paths.icon, hashes.get("icon"))))
    if isinstance(hashes.get("cli_binary"), str):
        owned.extend((("cli_launcher", paths.cli_launcher, "cli_launcher"), ("cli_binary", paths.cli_binary, hashes["cli_binary"])))
    if paths.application is not None and isinstance(hashes.get("application"), str):
        owned.append(("application", paths.application, hashes["application"]))
    return owned


def uninstall(paths: InstallPaths) -> bool:
    _recover_pending(paths)
    receipt = load_receipt(paths)
    if receipt is None:
        raise InstallError(f"no installer receipt exists at {paths.receipt}")
    ensure_application_destination(paths, receipt)
    receipt_paths_value = receipt.get("paths")
    raw_gui_owned = paths.application is None or (isinstance(receipt_paths_value, dict) and "binary" in receipt_paths_value)
    gui_target = paths.binary if raw_gui_owned else paths.gui_executable
    hashes = receipt.get("artifact_sha256")
    if not isinstance(hashes, dict):
        raise InstallError(f"installer receipt at {paths.receipt} has no ownership manifest")
    mismatches: list[str] = []
    for name, path, expected in _owned_artifacts(paths, receipt):
        if not _is_present(path):
            if name == "application":
                mismatches.append(name)
            continue
        if name in {"launcher", "cli_launcher"}:
            valid = _launcher_matches(path, gui_target if name == "launcher" else paths.cli_binary)
        elif name == "application":
            manifest = receipt.get("artifact_manifests", {}).get(name) if isinstance(receipt.get("artifact_manifests"), dict) else None
            valid = isinstance(manifest, list) and path.is_dir() and not path.is_symlink() and bundle_manifest(path) == manifest and bundle_manifest_hash(manifest) == expected
        else:
            valid = isinstance(expected, str) and path.is_file() and sha256(path) == expected
        if not valid:
            mismatches.append(name)
    if mismatches:
        for name in mismatches:
            print(f"kept unrecognized path: {name}", file=sys.stderr)
        return False
    for name, path, _ in _owned_artifacts(paths, receipt):
        if _is_present(path):
            _remove_known(path)
    paths.receipt.unlink(missing_ok=True)
    for directory in (paths.binary.parent, paths.app_root):
        try:
            directory.rmdir()
        except OSError:
            pass
    return True


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    if args.application_path is not None and sys.platform != "darwin":
        print("error: --application-path is only supported on macOS", file=sys.stderr)
        return 2
    try:
        paths = install_paths(args.prefix, application_path=args.application_path)
        if args.uninstall:
            complete = uninstall(paths)
            if complete:
                print(f"removed Cockpit files from {paths.data_home} and {paths.bin_home}")
            else:
                print(f"kept modified or foreign Cockpit files; receipt remains at {paths.receipt}", file=sys.stderr)
            return 0 if complete else 1
        if not args.reuse:
            run_build(PROJECT_ROOT, args.debug)
        bundle = source_bundle(PROJECT_ROOT, args.debug) if paths.application is not None else None
        install(paths, source_binary(PROJECT_ROOT, args.debug), source_cli_binary(PROJECT_ROOT, args.debug), source_icon(PROJECT_ROOT), bundle)
    except InstallError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(f"installed {APPLICATION_NAME} launcher: {paths.launcher}")
    print(f"installed {APPLICATION_NAME} CLI: {paths.cli_launcher}")
    if paths.application is not None:
        print(f"macOS application: {paths.application}")
    print(f"native binary: {paths.gui_executable}")
    print(f"CLI binary: {paths.cli_binary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
