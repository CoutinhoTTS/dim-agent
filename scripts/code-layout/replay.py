"""Restore captured input paths and preserve Cargo artifacts during native replay."""

from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile


@contextmanager
def original_inputs(inputs):
    with ExitStack() as cleanup:
        for item in inputs:
            source = Path(item["source"])
            if source.exists():
                if file_hash(source) != item["sha256"]:
                    raise RuntimeError(f"captured linker input changed: {source}")
                continue
            missing = []
            parent = source.parent
            while not parent.exists():
                missing.append(parent)
                parent = parent.parent
            for parent in reversed(missing):
                parent.mkdir()
                cleanup.callback(parent.rmdir)
            if file_hash(Path(item["file"])) != item["sha256"]:
                raise RuntimeError(f"archived linker input changed: {item['file']}")
            with source.open("xb") as destination:
                cleanup.callback(source.unlink)
                with Path(item["file"]).open("rb") as saved:
                    shutil.copyfileobj(saved, destination)
            stamp = Path(item["file"]).stat()
            os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
        yield


@contextmanager
def preserve_cargo(files):
    saved = []
    for path, backup in files:
        existed = path.exists()
        if existed:
            shutil.copy2(path, backup)
        saved.append((path, backup, existed))
    signals = [signal.SIGINT, signal.SIGTERM]
    if os.name == "nt":
        signals.append(signal.SIGBREAK)
    previous = {number: signal.getsignal(number) for number in signals}
    interrupted = None

    def cancel(number, frame):
        nonlocal interrupted
        if interrupted is None:
            interrupted = number
            raise SystemExit(128 + number)

    def defer(number, frame):
        nonlocal interrupted
        interrupted = number

    try:
        for number in signals:
            signal.signal(number, cancel)
        yield
    finally:
        for number in signals:
            signal.signal(number, defer)
        try:
            for path, backup, existed in saved:
                if existed:
                    shutil.copy2(backup, path)
                else:
                    path.unlink(missing_ok=True)
        finally:
            for number, handler in previous.items():
                signal.signal(number, handler)
        if interrupted is not None:
            raise SystemExit(128 + interrupted)


def run_link(command, *, cwd, env):
    with subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, encoding="utf-8", errors="surrogateescape",
                          start_new_session=os.name != "nt") as process:
        try:
            while True:
                try:
                    stdout, stderr = process.communicate(timeout=.2)
                    break
                except subprocess.TimeoutExpired:
                    pass
        except BaseException:
            if os.name == "nt":
                process.kill()
            else:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            process.wait()
            raise
        return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def save_baseline(output, baseline, record, target, repo):
    record.update(target=target, baselineSha256=file_hash(baseline),
                  mapSha256=file_hash(output / "baseline.map"),
                  source=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
                  dirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=repo)),
                  lto=os.environ.get("CARGO_PROFILE_RELEASE_LTO", "profile"))
    (output / "capture.json").write_text(json.dumps(record))


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_baseline(output, baseline, target):
    record = json.loads((output / "capture.json").read_text())
    if (record["target"] != target or file_hash(baseline) != record["baselineSha256"]
            or file_hash(output / "baseline.map") != record["mapSha256"]):
        raise ValueError("target, executable or map differs from the captured baseline")
    return record


def measure_artifacts(output, target, baseline, executable, sample):
    manifest = json.loads((output / "manifest.json").read_text())
    if (manifest["target"] != target or file_hash(baseline) != manifest["baselineSha256"]
            or file_hash(executable) != manifest["candidateSha256"]):
        raise ValueError("target or executable differs from the validated layout manifest")
    manifest["measurements"] = sample()
    with tempfile.NamedTemporaryFile(dir=output, mode="w", delete=False) as temp:
        json.dump(manifest, temp, indent=2)
        temp.write("\n")
    Path(temp.name).replace(output / "manifest.json")
