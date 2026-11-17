"""Native Mach-O training and acceptance flows for the layout build."""

from contextlib import ExitStack
import json
import os
from pathlib import Path
import re
import select
import signal
import socket
import subprocess
import time

from flows import LocalModel, checks, configure, conversation, environment, tui_pty
from samples import read as read_sample


def sample(client, path, timeout=30):
    if path.exists():
        path.unlink()
    client.send_signal(signal.SIGUSR1)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists() and path.stat().st_size:
            return
        time.sleep(.02)
    raise TimeoutError(f"the layout probe did not write {path}")


def terminate(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


class Server:
    def __init__(self, binary, home, stack, probe=None):
        self.home, self.binary = home, binary
        home.mkdir(parents=True, mode=0o700)
        native = home / "rust"
        native.mkdir(mode=0o700)
        self.socket_path = native / "daemon.sock"
        self.exit_sample = home / "exit.pages"
        self.signal_sample = home / "signal.pages"
        extra = {"DYLD_INSERT_LIBRARIES": str(probe)} if probe else None
        if probe:
            extra["DIM_LAYOUT_SAMPLE"] = str(self.signal_sample)
            extra["DIM_LAYOUT_EXIT_SAMPLE"] = str(self.exit_sample)
        error = stack.enter_context((home / "server.stderr").open("w"))
        self.process = subprocess.Popen(
            [str(binary), "app-server", "--product-home", str(home), "--home", str(native),
             "--unix", str(self.socket_path), "--no-ws"],
            cwd=home, env=environment(home, extra),
            stdout=subprocess.PIPE, stderr=error, start_new_session=True)
        stack.callback(self.process.stdout.close)
        stack.callback(terminate, self.process)
        pending = bytearray()
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            if select.select([self.process.stdout], [], [], .1)[0]:
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError(
                        f"server exited before ready: "
                        f"{(home / 'server.stderr').read_text()[-4000:]}"
                    )
                pending.extend(chunk)
                if b'"unixPath"' in pending and b"\n" in pending:
                    break
        else:
            raise TimeoutError(f"server not ready: {(home / 'server.stderr').read_text()[-4000:]}")
        self.stack = stack
        self.wire = None
        self.sequence = 0

    def rpc(self, method, params):
        if self.wire is None:
            connection = self.stack.enter_context(socket.socket(socket.AF_UNIX))
            connection.settimeout(30)
            connection.connect(str(self.socket_path))
            self.wire = self.stack.enter_context(connection.makefile("rwb"))
            self.rpc("hello", {"protocolVersion": 4, "clientKind": "other"})
        self.sequence += 1
        self.wire.write(json.dumps({"jsonrpc": "2.0", "id": self.sequence,
                                    "method": method, "params": params}).encode() + b"\n")
        self.wire.flush()
        while True:
            line = self.wire.readline()
            if not line:
                raise RuntimeError(f"connection closed during {method}")
            reply = json.loads(line)
            if reply.get("id") == self.sequence:
                if "error" in reply:
                    raise RuntimeError(f"{method}: {reply['error']}")
                return reply["result"]

    def close(self, profile=None):
        self.process.send_signal(signal.SIGTERM)
        if self.process.wait(timeout=60) != 0:
            raise RuntimeError(f"training process failed: {self.process.returncode}")
        if profile:
            if not self.exit_sample.exists():
                raise RuntimeError("the layout probe did not write the shutdown sample")
            profile.write_text(self.exit_sample.read_text())


def train(binary, root, probe=None):
    root.mkdir()
    profiles = [root / f"{name}.pages" for name in ("startup-shutdown", "tui", "conversation")]
    print(f"Running {'training' if probe else 'acceptance'} flows: {binary}", flush=True)
    with ExitStack() as stack:
        server = Server(binary, root / "startup", stack, probe)
        server.close(profiles[0] if probe else None)
    with ExitStack() as stack:
        model = LocalModel(stack)
        server = Server(binary, root / "interactive", stack)
        configure(server, model.http.server_port)
        env = environment(server.home, {"DYLD_INSERT_LIBRARIES": str(probe)} if probe else None)
        signal_sample = server.home / "tui.pages"
        if probe:
            env["DIM_LAYOUT_SAMPLE"] = str(signal_sample)
            env["DIM_LAYOUT_EXIT_SAMPLE"] = str(server.home / "exit.pages")

        def take(client):
            sample(client, signal_sample)
            profiles[1].write_text(signal_sample.read_text())

        tui_pty(binary, server.home, stack, env, terminate,
                sample=take if probe else None)
        server.close()
    with ExitStack() as stack:
        model = LocalModel(stack)
        server = Server(binary, root / "conversation", stack, probe)
        configure(server, model.http.server_port)
        conversation(binary, server.home, model)
        server.close(profiles[2] if probe else None)
    checks(binary, root)
    return profiles


def _ps(pid):
    result = subprocess.run(["ps", "-o", "rss=,time=", "-p", str(pid)],
                            capture_output=True, text=True, check=True)
    fields = result.stdout.split()
    rss = int(fields[0]) * 1024
    parts = fields[1].split(":")
    if len(parts) == 2:
        hours, minutes = 0, int(parts[0])
    elif len(parts) == 3:
        hours, minutes = int(parts[0]), int(parts[1])
    else:
        raise ValueError(f"unexpected ps CPU time: {fields[1]!r}")
    return {"resident": rss, "cpu": hours * 3600 + minutes * 60 + float(parts[-1])}


def _footprint(pid):
    result = subprocess.run(["vmmap", "-summary", str(pid)],
                            capture_output=True, text=True)
    if result.returncode:
        return None
    for line in result.stdout.splitlines():
        if line.strip().startswith("Physical footprint:"):
            value = line.split(":", 1)[1].strip()
            match = re.fullmatch(r"([0-9.]+)([KMG])", value)
            if match:
                scale = {"K": 1024, "M": 1024 ** 2, "G": 1024 ** 3}[match[2]]
                return int(float(match[1]) * scale)
    return None


def _one(binary, root, probe):
    started = time.monotonic()
    with ExitStack() as stack:
        server = Server(binary, root, stack, probe)
        ready = time.monotonic() - started
        time.sleep(.1)
        sample(server.process, server.signal_sample)
        page_size, pages = read_sample(server.signal_sample)
        ps = _ps(server.process.pid)
        footprint = _footprint(server.process.pid)
        server.close()
    return {"wall": ready, "cpu": ps["cpu"], "resident": ps["resident"],
            "physicalFootprint": footprint, "pageSize": page_size,
            "codePages": len(pages), "codeResident": len(pages) * page_size}


def _latency(binary, root, model):
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


def measure(baseline, candidate, root, probe, pairs=3):
    """Alternate instrumented startup samples and one local tool conversation."""
    root.mkdir()
    results = {"baseline": [], "candidate": []}
    with ExitStack() as stack:
        model = LocalModel(stack)
        for index in range(pairs):
            order = [("baseline", baseline), ("candidate", candidate)]
            if index % 2:
                order.reverse()
            for name, binary in order:
                sample = _one(binary, root / f"{index}-{name}", probe)
                sample["conversationWall"] = _latency(
                    binary, root / f"{index}-{name}-conversation", model)
                results[name].append(sample)
    return results
