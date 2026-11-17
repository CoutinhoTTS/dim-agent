"""Preserve Cargo artifacts while a native linker reuses their canonical paths."""

from contextlib import contextmanager
import os
import shutil
import signal
import subprocess


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
