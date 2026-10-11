"""Creation-only receipts for the ui_polish_runtime fixture's normal shutdown.

Linux /proc and pidfds are observational guards, not a same-UID sandbox. Old
fixtures are deliberately not adopted. Only recorded generated scratch roots
can be removed; the fixture evidence, repositories and borrowed notes remain.
"""
import contextlib
import ctypes
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import uuid

ADAPTER = "ui_polish_runtime"
MANIFEST = "creation-manifest.json"
JOURNAL = "cleanup-effects.json"
SCRATCH = ("cache", "tmp", "native-cache")


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".new")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as output:
        json.dump(value, output, indent=2)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def filesystem_identity(path):
    info = path.lstat()
    if path.is_symlink() or not path.is_dir():
        raise ValueError("owned root must be a real directory")
    return {"path": str(path), "device": info.st_dev, "inode": info.st_ino}


def boot_id():
    return Path("/proc/sys/kernel/random/boot_id").read_text().strip()


def process_identity(pid):
    directory = Path(f"/proc/{pid}")
    stat = (directory / "stat").read_text().rsplit(")", 1)[1].split()
    executable = (directory / "exe").stat()
    return {"pid": pid, "boot_id": boot_id(), "start_ticks": int(stat[19]),
            "session_id": int(stat[3]), "process_group": int(stat[2]),
            "exe_device": executable.st_dev, "exe_inode": executable.st_ino}


def process_environment(pid):
    return dict(part.split("=", 1) for part in Path(f"/proc/{pid}/environ").read_text().split("\0") if "=" in part)


def create(root, session):
    """Called only immediately after mkdtemp and creation of generated roots."""
    manifest = {"adapter": ADAPTER, "creation_id": str(uuid.uuid4()),
                "root": filesystem_identity(root), "session": session,
                "boot_id": boot_id(), "processes": [],
                "owner_run_id": os.environ.get("COCKPIT_RUN_ID"),
                "owner_root_id": os.environ.get("COCKPIT_ROOT_ID"),
                "machine_id": Path("/etc/machine-id").read_text().strip(),
                "scratch": [filesystem_identity(root / name) for name in SCRATCH]}
    atomic_json(root / MANIFEST, manifest)
    return manifest


def record_process(root, name, pid):
    with locked(root):
        manifest = load(root)
        identity = process_identity(pid)
        environment = process_environment(pid)
        if identity["session_id"] != pid or environment.get("COCKPIT_FIXTURE_CREATION_ID") != manifest["creation_id"]:
            raise ValueError("launch lacks exact fixture creation membership")
        manifest["processes"].append({"name": name, "identity": identity})
        atomic_json(root / MANIFEST, manifest)


@contextlib.contextmanager
def locked(root):
    if root.resolve() != root or root.is_symlink() or not root.name.startswith("cpol-"):
        raise ValueError("fixture root is not an exact created directory")
    if not (root / MANIFEST).is_file() or (root / MANIFEST).is_symlink():
        raise ValueError("fixture has no original creation manifest; kept")
    fd = os.open(root / ".cleanup.lock", os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def load(root, creation_id=None):
    if root.resolve() != root or (root / MANIFEST).is_symlink():
        raise ValueError("fixture root/manifest alias is not a creation identity")
    manifest = json.loads((root / MANIFEST).read_text())
    ledger = json.loads((root / "runtime.json").read_text())
    if (manifest.get("adapter") != ADAPTER or not root.name.startswith("cpol-")
            or manifest["root"] != filesystem_identity(root)
            or ledger["root"] != str(root) or ledger["session"] != manifest["session"]
            or not manifest["session"].startswith("polish-")
            or (creation_id is not None and creation_id != manifest["creation_id"])):
        raise ValueError("fixture creation receipt does not match current root/session")
    return manifest


def read_journal(root):
    path = root / JOURNAL
    return json.loads(path.read_text()) if path.exists() else {"effects": {}, "members": {}}


def receipt(root, journal, effect_id, state, evidence):
    identity = journal["members"].get(effect_id, journal.get("identities", {}).get(effect_id))
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    namespace = uuid.UUID(json.loads((root / MANIFEST).read_text())["creation_id"])
    result = {"effect_id": str(uuid.uuid5(namespace, effect_id)),
              "kind": "stop_process" if effect_id.startswith("process:") else "remove_scratch",
              "identity_digest": digest, "state": state, "evidence": evidence,
              "updated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")}
    journal["effects"][effect_id] = result
    atomic_json(root / JOURNAL, journal)
    return result


def inspect_process(identity, manifest):
    if boot_id() != identity["boot_id"]:
        return "refused", "boot_changed"
    try:
        current = process_identity(identity["pid"])
        environment = process_environment(identity["pid"])
    except FileNotFoundError:
        # A dead original identity is absent even if its numeric PID was reused.
        return "absent", "process_absent"
    except (OSError, ValueError):
        return "unknown", "process_unobservable"
    if current != identity:
        return "refused", "process_identity_changed"
    if environment.get("COCKPIT_FIXTURE_CREATION_ID") != manifest["creation_id"]:
        return "refused", "foreign_membership"
    return "live", "exact_creation_membership"


def members(manifest, problems=None):
    """Inspect created sessions; escaped/foreign membership remains protected."""
    sessions = {item["identity"]["session_id"] for item in manifest["processes"]}
    result = {}
    for directory in Path("/proc").iterdir():
        if not directory.name.isdecimal() or int(directory.name) == os.getpid():
            continue
        try:
            stat = (directory / "stat").read_text().rsplit(")", 1)[1].split()
            environment = process_environment(int(directory.name)) if directory.stat().st_uid == os.getuid() else {}
            if int(stat[3]) not in sessions:
                if environment.get("COCKPIT_FIXTURE_CREATION_ID") == manifest["creation_id"]:
                    raise ValueError("escaped fixture consumer has no creation-session proof")
                continue
            if stat[0] == "Z":
                continue  # exited; no live filesystem consumer
            identity = process_identity(int(directory.name))
            if environment.get("COCKPIT_FIXTURE_CREATION_ID") != manifest["creation_id"]:
                raise ValueError("foreign process in fixture session")
            result[f"process:{identity['pid']}:{identity['start_ticks']}"] = identity
        except FileNotFoundError:
            continue
        except (OSError, ValueError) as error:
            if problems is None:
                raise ValueError("process membership cannot be exhaustively observed") from error
            problems.append(f"{directory.name}: {error}")
    return result


def stop_process(root, manifest, journal, effect_id, identity):
    state, evidence = inspect_process(identity, manifest)
    previous = journal["effects"].get(effect_id, {}).get("state")
    if state == "live" and previous in ("intent", "unknown"):
        return receipt(root, journal, effect_id, "unknown", "interrupted_intent_live_requires_inspection")
    if state != "live":
        return receipt(root, journal, effect_id, state, evidence)
    if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
        return receipt(root, journal, effect_id, "refused", "pidfd_unavailable")
    descriptor = None
    try:
        descriptor = os.pidfd_open(identity["pid"])
        # Recheck after pinning; never signal a newly reused numeric PID.
        state, evidence = inspect_process(identity, manifest)
        if state != "live":
            return receipt(root, journal, effect_id, state, evidence)
        receipt(root, journal, effect_id, "intent", "exact_identity_before_sigterm")
        if load(root, manifest["creation_id"]) != manifest:
            return receipt(root, journal, effect_id, "refused", "fixture_creation_changed_before_sigterm")
        state, evidence = inspect_process(identity, manifest)
        if state != "live":
            return receipt(root, journal, effect_id, state, evidence)
        signal.pidfd_send_signal(descriptor, signal.SIGTERM)
        import select
        ready, _, _ = select.select([descriptor], [], [], 5)
        if ready:
            return receipt(root, journal, effect_id, "absent", "pidfd_joined_exit")
        return receipt(root, journal, effect_id, "unknown", "sigterm_sent_survivor_protected")
    except ProcessLookupError:
        return receipt(root, journal, effect_id, "absent", "process_absent")
    except (OSError, ValueError) as error:
        return receipt(root, journal, effect_id, "unknown", str(error))
    finally:
        if descriptor is not None:
            os.close(descriptor)


def shutdown(root, creation_id=None, inspect_only=False, request=None):
    with locked(root):
        manifest = load(root, creation_id)
        if request is not None:
            validate_request(root, manifest, request)
        journal = read_journal(root)
        if manifest["boot_id"] != boot_id():
            return {"state": "refused", "evidence": "boot_changed", "effects": list(journal["effects"].values())}
        # Persist each discovered constituent before dispatch, not just parent PIDs.
        problems = []
        observed = members(manifest, problems)
        pinned = journal["members"]
        pinned.update(observed)
        for item in manifest["processes"]:
            identity = item["identity"]
            pinned.setdefault(f"process:{identity['pid']}:{identity['start_ticks']}", identity)
        atomic_json(root / JOURNAL, journal)
        for effect_id, identity in pinned.items():
            if inspect_only:
                state, evidence = inspect_process(identity, manifest)
                if state == "live":
                    state = "unknown" if journal["effects"].get(effect_id, {}).get("state") in ("intent", "unknown") else "ready"
                if state != "ready":
                    receipt(root, journal, effect_id, state, evidence)
            else:
                stop_process(root, manifest, journal, effect_id, identity)
        survivors = members(manifest, problems)
        states = [journal["effects"].get(key, {}).get("state", "ready") for key in pinned]
        complete = not problems and not survivors and all(state == "absent" for state in states)
        # A protected constituent does not veto independently verified effects.
        ready = inspect_only and "ready" in states
        return {"state": "absent" if complete else "ready" if ready else "unknown",
                "evidence": "all_recorded_constituents_joined" if complete else "; ".join(problems) or "constituent_inspection",
                "effects": list(journal["effects"].values())}


def references_root(root):
    """Fail closed on unobservable same-user processes, mount trees or foreign FDs."""
    root_text = str(root) + "/"
    for directory in Path("/proc").iterdir():
        if not directory.name.isdecimal() or int(directory.name) == os.getpid():
            continue
        try:
            links = [directory / "cwd", directory / "root"]
            links.extend((directory / "fd").iterdir())
            for link in links:
                try:
                    target = os.readlink(link).removesuffix(" (deleted)")
                except FileNotFoundError:
                    continue
                if target == str(root) or target.startswith(root_text):
                    return True
            if any(root_text in line for line in (directory / "maps").read_text().splitlines()):
                return True
            # A bind mount can expose a constituent under another pathname.
            if str(root) in (directory / "mountinfo").read_text():
                return True
            if directory.stat().st_uid == os.getuid():
                if any(value == str(root) or value.startswith(root_text) for value in process_environment(int(directory.name)).values()):
                    return True
        except FileNotFoundError:
            continue
        except OSError:
            # Cross-UID unreadable /proc also prevents an exhaustive consumer proof.
            return True
    return False


def rename_no_replace(source_fd, source, target_fd, target):
    """Linux's atomic no-clobber rename, used only for typed scratch reservation."""
    function = ctypes.CDLL(None, use_errno=True).renameat2
    function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    function.restype = ctypes.c_int
    if function(source_fd, os.fsencode(source), target_fd, os.fsencode(target), 1):
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), source)


def remove_reserved_scratch(root, expected, journal, effect_id):
    """Reserve the directory, then verify what moved; never delete its public name."""
    parent_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    holding_fd = None
    holding_name = ".scratch-reservation-" + uuid.uuid4().hex
    basename = Path(expected["path"]).name
    try:
        root_info = os.fstat(parent_fd)
        creation_root = load(root)["root"]
        if (root_info.st_dev, root_info.st_ino) != (creation_root["device"], creation_root["inode"]):
            return receipt(root, journal, effect_id, "unknown", "fixture_parent_changed")
        if filesystem_identity(Path(expected["path"])) != expected or references_root(root):
            return receipt(root, journal, effect_id, "unknown", "identity_or_consumers_changed_before_reservation")
        os.mkdir(holding_name, mode=0o700, dir_fd=parent_fd)
        holding_fd = os.open(holding_name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)
        holding_info = os.fstat(holding_fd)
        reservation = {"path": str(root / holding_name / "resource"), "state": "reserved",
                       "container_device": holding_info.st_dev, "container_inode": holding_info.st_ino}
        journal.setdefault("reservations", {})[effect_id] = reservation
        atomic_json(root / JOURNAL, journal)
        rename_no_replace(parent_fd, basename, holding_fd, "resource")
        moved = os.stat("resource", dir_fd=holding_fd, follow_symlinks=False)
        if (not stat.S_ISDIR(moved.st_mode)
                or (moved.st_dev, moved.st_ino) != (expected["device"], expected["inode"])):
            # The source was replaced during reservation. Restore without
            # overwriting another concurrent public entry; never remove it.
            try:
                rename_no_replace(holding_fd, "resource", parent_fd, basename)
            except OSError:
                reservation["state"] = "mismatch_held"
                return receipt(root, journal, effect_id, "unknown", "replacement_preserved_at: " + reservation["path"])
            reservation["state"] = "restored_mismatch"
            current_holding = os.stat(holding_name, dir_fd=parent_fd, follow_symlinks=False)
            if (current_holding.st_dev, current_holding.st_ino) != (holding_info.st_dev, holding_info.st_ino):
                return receipt(root, journal, effect_id, "unknown", "restored_but_reservation_container_changed")
            os.rmdir(holding_name, dir_fd=parent_fd)
            return receipt(root, journal, effect_id, "refused", "replacement_restored_without_deletion")
        if references_root(root):
            return receipt(root, journal, effect_id, "unknown", "consumer_changed_after_reservation")
        # Only the identity-verified reserved entry is removed. A concurrently
        # installed public basename is outside this fd-anchored operation.
        fresh = os.stat("resource", dir_fd=holding_fd, follow_symlinks=False)
        if (fresh.st_dev, fresh.st_ino) != (expected["device"], expected["inode"]):
            return receipt(root, journal, effect_id, "unknown", "reserved_identity_changed")
        shutil.rmtree("resource", dir_fd=holding_fd)
        reservation["state"] = "removed"
        current_holding = os.stat(holding_name, dir_fd=parent_fd, follow_symlinks=False)
        if (current_holding.st_dev, current_holding.st_ino) != (holding_info.st_dev, holding_info.st_ino):
            return receipt(root, journal, effect_id, "unknown", "removed_but_reservation_container_changed")
        os.rmdir(holding_name, dir_fd=parent_fd)
        return receipt(root, journal, effect_id, "absent", "registered_scratch_removed_from_reservation")
    except (OSError, AttributeError) as error:
        return receipt(root, journal, effect_id, "unknown", str(error))
    finally:
        if holding_fd is not None:
            os.close(holding_fd)
        os.close(parent_fd)


def cleanup_scratch(root, creation_id, scratch_path, inspect_only=False, request=None):
    with locked(root):
        manifest = load(root, creation_id)
        if request is not None:
            validate_request(root, manifest, request)
        expected = next((item for item in manifest["scratch"] if item["path"] == scratch_path), None)
        if expected is None or Path(scratch_path).parent != root:
            raise ValueError("scratch was not registered at creation")
        path = Path(scratch_path)
        effect_id = "scratch:" + path.name
        journal = read_journal(root)
        journal.setdefault("identities", {})[effect_id] = expected
        reservation = journal.get("reservations", {}).get(effect_id)
        if reservation and reservation["state"] in ("reserved", "mismatch_held"):
            return receipt(root, journal, effect_id, "unknown",
                           "reserved_scratch_requires_inspection: " + reservation["path"])
        if not path.exists() and not path.is_symlink():
            return receipt(root, journal, effect_id, "absent", "registered_scratch_absent")
        if manifest["boot_id"] != boot_id() or filesystem_identity(path) != expected:
            return receipt(root, journal, effect_id, "refused", "scratch_identity_changed")
        if (not journal["members"] or members(manifest)
                or any(journal["effects"].get(key, {}).get("state") != "absent" for key in journal["members"])
                or references_root(root)):
            return receipt(root, journal, effect_id, "refused", "consumer_quiescence_unproven")
        if journal["effects"].get(effect_id, {}).get("state") in ("intent", "unknown"):
            return receipt(root, journal, effect_id, "unknown", "interrupted_scratch_requires_inspection")
        # Do not descend through symlinks, foreign mounts or replaced roots.
        for parent, directories, files in os.walk(path, followlinks=False):
            for name in directories + files:
                child = Path(parent) / name
                info = child.lstat()
                if (child.is_symlink() or info.st_dev != expected["device"] or os.path.ismount(child)
                        or info.st_uid != os.getuid() or info.st_nlink > 1 and not stat.S_ISDIR(info.st_mode)
                        or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode))):
                    return receipt(root, journal, effect_id, "refused", "foreign_or_unclassified_scratch_entry")
        if inspect_only:
            return {"effect_id": effect_id, "state": "ready", "evidence": "registered_scratch_quiescent"}
        if not shutil.rmtree.avoids_symlink_attacks:
            return receipt(root, journal, effect_id, "refused", "safe_directory_removal_unavailable")
        receipt(root, journal, effect_id, "intent", "exact_created_root_and_observed_quiescence")
        return remove_reserved_scratch(root, expected, journal, effect_id)


def registration(root, request=None):
    with locked(root):
        manifest = load(root)
        if request is not None:
            validate_request(root, manifest, request)
        if manifest["boot_id"] != boot_id():
            raise ValueError("fixture creation is from another boot")
        for item in manifest["processes"]:
            state, _ = inspect_process(item["identity"], manifest)
            if state not in ("live", "absent"):
                raise ValueError("fixture process creation identity is unproven")
        for scratch in manifest["scratch"]:
            if filesystem_identity(Path(scratch["path"])) != scratch:
                raise ValueError("scratch creation identity changed")
        return typed_registration(root, manifest)


def typed_registration(root, manifest):
    info = (root / MANIFEST).stat()

    def filesystem(item):
        return {"creation_operation_id": manifest["creation_id"],
                "machine_id": manifest["machine_id"], "root": item["path"],
                "device": str(item["device"]), "inode": str(item["inode"]),
                "manifest_path": str(root / MANIFEST),
                "manifest_device": str(info.st_dev), "manifest_inode": str(info.st_ino)}

    fixture = {"kind": "fixture_set", "identity": {
        "adapter": ADAPTER, "creation_operation_id": manifest["creation_id"],
        "filesystem": filesystem(manifest["root"]),
        "processes": [{"pid": item["identity"]["pid"],
                       "start_ticks": item["identity"]["start_ticks"],
                       "kernel_boot_id": item["identity"]["boot_id"]}
                      for item in manifest["processes"]]}}
    return {"owner_run_id": manifest["owner_run_id"], "owner_root_id": manifest["owner_root_id"],
            "resources": [fixture] + [{"kind": "scratch", "identity": filesystem(item)}
                                     for item in manifest["scratch"]]}


def validate_request(root, manifest, request):
    actual = typed_registration(root, manifest)
    if (actual["owner_run_id"] != request["run_id"] or actual["owner_root_id"] != request["root_id"]
            or not actual["owner_run_id"] or request["resource"] not in actual["resources"]
            or manifest["machine_id"] != Path("/etc/machine-id").read_text().strip()):
        raise ValueError("typed resource is not this run's creation receipt")


def helper_operation(request):
    """Closed product helper operation; never executes caller-provided commands."""
    resource = request["resource"]
    identity = resource["identity"]
    filesystem = identity["filesystem"] if resource["kind"] == "fixture_set" else identity
    root = Path(filesystem["manifest_path"]).parent
    action = request["action"]
    if action == "register":
        registration(root, request=request)
        return {"state": "ready", "evidence": "matching_creation_receipt_and_owner"}
    if action == "receipts":
        with locked(root):
            validate_request(root, load(root, filesystem["creation_operation_id"]), request)
            return {"effects": list(read_journal(root)["effects"].values())}
    if action not in ("inspect", "apply"):
        raise ValueError("unsupported fixture operation")
    if resource["kind"] == "fixture_set":
        return shutdown(root, filesystem["creation_operation_id"], inspect_only=action == "inspect", request=request)
    if resource["kind"] == "scratch":
        return cleanup_scratch(root, filesystem["creation_operation_id"], filesystem["root"], inspect_only=action == "inspect", request=request)
    raise ValueError("unsupported fixture resource kind")
