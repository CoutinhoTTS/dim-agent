"""Native PE/Windows training and acceptance flows for the layout build."""

from contextlib import ExitStack
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import threading
import time

from flows import LocalModel, checks, configure, conversation, environment
import pe
from samples import write as write_sample
import win32


def terminate(process):
    if process.poll() is None:
        process.kill()
        process.wait()


class Wire:
    def __init__(self, path, stack):
        self.fd = win32.open_pipe(path)
        stack.callback(os.close, self.fd)
        self.pending = bytearray()

    def write(self, value):
        os.write(self.fd, json.dumps(value).encode() + b"\n")

    def readline(self):
        while b"\n" not in self.pending:
            chunk = os.read(self.fd, 65536)
            if not chunk:
                raise RuntimeError("server connection closed")
            self.pending.extend(chunk)
        line, _, rest = self.pending.partition(b"\n")
        self.pending = rest
        return line + b"\n"


class Server:
    def __init__(self, binary, home, stack, sampling=False):
        self.home, self.binary = home, binary
        home.mkdir(parents=True, mode=0o700)
        native = home / "rust"
        native.mkdir(mode=0o700)
        self.pipe = win32.default_pipe(native)
        error = stack.enter_context((home / "server.stderr").open("w"))
        flags = win32.CREATE_SUSPENDED if sampling else 0
        self.process = subprocess.Popen(
            [str(binary), "app-server", "--product-home", str(home), "--home", str(native),
             "--pipe", self.pipe, "--no-ws"],
            cwd=home, env=environment(home), stdout=subprocess.PIPE, stderr=error,
            creationflags=flags)
        stack.callback(self.process.stdout.close)
        stack.callback(terminate, self.process)
        if sampling:
            handle = win32.open_process(self.process.pid)
            try:
                win32.empty_working_set(handle)
                win32.resume(handle)
            finally:
                win32.close(handle)
        ready = queue.Queue()

        def read():
            while True:
                line = self.process.stdout.readline()
                if not line:
                    ready.put(None)
                    return
                ready.put(line)

        threading.Thread(target=read, daemon=True).start()
        deadline = time.monotonic() + 120
        pending = bytearray()
        while time.monotonic() < deadline:
            try:
                line = ready.get(timeout=1)
            except queue.Empty:
                continue
            if line is None:
                raise RuntimeError(
                    f"server exited before ready: "
                    f"{(home / 'server.stderr').read_text()[-4000:]}"
                )
            pending.extend(line)
            if b'"pipeName"' in pending:
                break
        else:
            raise TimeoutError(f"server not ready: {(home / 'server.stderr').read_text()[-4000:]}")
        self.stack = stack
        self.wire = None
        self.sequence = 0

    def connect(self):
        if self.wire is not None:
            return
        self.wire = Wire(self.pipe, self.stack)
        self.sequence += 1
        self.wire.write({"jsonrpc": "2.0", "id": self.sequence,
                         "method": "hello",
                         "params": {"protocolVersion": 4, "clientKind": "other"}})
        while True:
            reply = json.loads(self.wire.readline())
            if reply.get("id") == self.sequence:
                if "error" in reply:
                    raise RuntimeError(f"hello: {reply['error']}")
                return

    def rpc(self, method, params):
        self.connect()
        self.sequence += 1
        self.wire.write({"jsonrpc": "2.0", "id": self.sequence,
                         "method": method, "params": params})
        while True:
            reply = json.loads(self.wire.readline())
            if reply.get("id") == self.sequence:
                if "error" in reply:
                    raise RuntimeError(f"{method}: {reply['error']}")
                return reply["result"]

    def close(self, profile=None):
        self.connect()
        handle = None
        if profile:
            handle = win32.open_process(self.process.pid)
            try:
                base = win32.module(handle)[0]
                rva, size = pe.text_range(self.binary)
            except BaseException:
                win32.close(handle)
                raise
        self.sequence += 1
        self.wire.write({"jsonrpc": "2.0", "id": self.sequence,
                         "method": "daemon.shutdown", "params": {"force": True}})
        if profile:
            last = None
            try:
                while self.process.poll() is None:
                    try:
                        _, pages = win32.working_set_pages(handle, base, rva, size)
                    except OSError:
                        break
                    if pages:
                        last = pages
                    time.sleep(.005)
            finally:
                win32.close(handle)
            if self.process.wait(timeout=60) != 0:
                raise RuntimeError(f"training process failed: {self.process.returncode}")
            if not last:
                raise RuntimeError("the shutdown working-set sample was empty")
            write_sample(profile, win32.page_size(), last)
        else:
            if self.process.wait(timeout=60) != 0:
                raise RuntimeError(f"training process failed: {self.process.returncode}")


def tui(binary, home, stack, sampling, profile):
    console = win32.Console(
        [binary, "tui", "--home", home, "--cwd", home, "--fullscreen"],
        home, environment(home))
    stack.callback(console.close)
    if sampling:
        console.trim_running()
    capture = bytearray()

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            chunk = console.take()
            if chunk:
                capture.extend(chunk)
                if b"\x1b[6n" in chunk:
                    console.write(b"\x1b[1;1R")
                if b"\x1b]11;?" in chunk:
                    console.write(b"\x1b]11;rgb:0000/0000/0000\x1b\\")
            time.sleep(.02)

    drain(2)
    for _ in range(3):
        start = len(capture)
        console.write(b"/models\r")
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            drain(.05)
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", bytes(capture[start:]))
            if b"Models" in plain and b"Enter select" in plain:
                break
        else:
            raise TimeoutError(f"TUI model picker did not render: {plain[-2000:]!r}")
        console.write(b"\x1b")
        drain(.2)
    if sampling:
        handle = win32.open_process(console.pid)
        try:
            win32.suspend(handle)
            try:
                base, _, _ = win32.module(handle)
                rva, size = pe.text_range(binary)
                page_size, pages = win32.working_set_pages(handle, base, rva, size)
                if not pages:
                    raise RuntimeError("the TUI's .text pages are not resident")
            finally:
                win32.resume(handle)
        finally:
            win32.close(handle)
        write_sample(profile, page_size, pages)
    console.write(b"/exit\r")
    code = console.wait(timeout=60)
    if code != 0:
        raise RuntimeError(f"TUI process failed: {code}")


def train(binary, root, sampling=False):
    root.mkdir()
    profiles = [root / f"{name}.pages" for name in ("startup-shutdown", "tui", "conversation")]
    print(f"Running {'training' if sampling else 'acceptance'} flows: {binary}", flush=True)
    with ExitStack() as stack:
        server = Server(binary, root / "startup", stack, sampling)
        server.close(profiles[0] if sampling else None)
    with ExitStack() as stack:
        model = LocalModel(stack)
        server = Server(binary, root / "interactive", stack)
        configure(server, model.http.server_port)
        tui(binary, server.home, stack, sampling, profiles[1])
        server.close()
    with ExitStack() as stack:
        model = LocalModel(stack)
        server = Server(binary, root / "conversation", stack, sampling)
        configure(server, model.http.server_port)
        conversation(binary, server.home, model)
        server.close(profiles[2] if sampling else None)
    checks(binary, root)
    return profiles


def _one(binary, root):
    root.mkdir(parents=True)
    started = time.monotonic()
    with ExitStack() as stack:
        server = Server(binary, root, stack, sampling=True)
        ready = time.monotonic() - started
        time.sleep(.1)
        handle = win32.open_process(server.process.pid)
        try:
            win32.suspend(handle)
            try:
                base = win32.module(handle)[0]
                rva, size = pe.text_range(binary)
                page_size, pages = win32.working_set_pages(handle, base, rva, size)
                memory = win32.memory(handle)
                cpu = win32.cpu(handle)
            finally:
                win32.resume(handle)
        finally:
            win32.close(handle)
        server.close()
    if not pages:
        raise RuntimeError("the startup working set has no .text pages")
    return {"wall": ready, "cpu": cpu["user"] + cpu["kernel"], "workingSet": memory["workingSet"],
            "private": memory["private"], "pagefile": memory["pagefile"],
            "pageSize": page_size, "codePages": len(pages),
            "codeResident": len(pages) * page_size}


def _latency(binary, root, model):
    root.mkdir(parents=True)
    with ExitStack() as stack:
        server = Server(binary, root, stack)
        configure(server, model.http.server_port)
        (server.home / "marker.txt").write_text("layout-probe-marker\n")
        started = time.monotonic()
        result = subprocess.run(
            [str(binary), "--home", str(server.home), "--cwd", str(server.home),
             "exec", "--prompt", "Read marker.txt and report completion."],
            cwd=server.home, env=environment(server.home), capture_output=True,
            text=True, timeout=120)
        elapsed = time.monotonic() - started
        if result.returncode or not model.tool_seen.is_set():
            raise RuntimeError(f"measurement conversation failed: {result.stdout}\n{result.stderr}")
        server.close()
    return elapsed


def measure(baseline, candidate, root, pairs=3):
    root.mkdir()
    results = {"baseline": [], "candidate": []}
    with ExitStack() as stack:
        model = LocalModel(stack)
        for index in range(pairs):
            order = [("baseline", baseline), ("candidate", candidate)]
            if index % 2:
                order.reverse()
            for name, binary in order:
                sample = _one(binary, root / f"{index}-{name}")
                sample["conversationWall"] = _latency(
                    binary, root / f"{index}-{name}-conversation", model)
                results[name].append(sample)
    return results
