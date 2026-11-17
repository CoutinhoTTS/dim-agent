"""Offline public user flows for executable layout training and acceptance."""

from contextlib import ExitStack
import fcntl
import json
import os
from pathlib import Path
import select
import signal
import socket
import subprocess
import time

from flows import LocalModel, checks, configure, conversation, environment, tui_pty


def stopped(process):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        pid, status = os.waitpid(process.pid, os.WNOHANG | os.WUNTRACED)
        if pid:
            if os.WIFSTOPPED(status) and os.WSTOPSIG(status) == signal.SIGSTOP:
                return
            process.returncode = os.waitstatus_to_exitcode(status)
            raise RuntimeError(f"training process exited early: {process.returncode}")
        time.sleep(.01)
    raise TimeoutError("training process did not stop")


def begin(process):
    stopped(process)
    Path(f"/proc/{process.pid}/clear_refs").write_text("1\n")
    process.send_signal(signal.SIGCONT)


def finish(process, profile=None):
    if profile:
        stopped(process)
        profile.write_text(Path(f"/proc/{process.pid}/smaps").read_text())
        process.send_signal(signal.SIGCONT)
    if process.wait(timeout=30) != 0:
        raise RuntimeError(f"training process failed: {process.returncode}")


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
        error = stack.enter_context((home / "server.stderr").open("w"))
        self.process = subprocess.Popen(
            [str(binary), "app-server", "--product-home", str(home), "--home", str(native),
             "--unix", str(self.socket_path), "--no-ws"], cwd=home, env=environment(home, {"LD_PRELOAD": str(probe)} if probe else None),
            stdout=subprocess.PIPE, stderr=error, start_new_session=True)
        stack.callback(self.process.stdout.close)
        stack.callback(terminate, self.process)
        if probe:
            begin(self.process)
        pending = bytearray()
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if select.select([self.process.stdout], [], [], .1)[0]:
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError(f"server exited before ready: {(home / 'server.stderr').read_text()[-4000:]}")
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
        finish(self.process, profile)


def tui(binary, home, stack, probe, profile):
    env = environment(home, {"LD_PRELOAD": str(probe)} if probe else None)
    if probe:
        env["DIM_LAYOUT_NO_EXIT_STOP"] = "1"

    def sample(client):
        client.send_signal(signal.SIGSTOP)
        stopped(client)
        profile.write_text(Path(f"/proc/{client.pid}/smaps").read_text())
        client.send_signal(signal.SIGCONT)

    tui_pty(binary, home, stack, env, terminate,
            begin=begin if probe else None, sample=sample if probe else None)


def train(binary, root, probe=None):
    root.mkdir()
    profiles = [root / f"{name}.smaps" for name in ("startup-shutdown", "tui", "conversation")]
    print(f"Running {'training' if probe else 'acceptance'} flows: {binary}", flush=True)
    with ExitStack() as stack:
        server = Server(binary, root / "startup", stack, probe)
        server.close(profiles[0] if probe else None)
    with ExitStack() as stack:
        model = LocalModel(stack)
        server = Server(binary, root / "interactive", stack)
        configure(server, model.http.server_port)
        tui(binary, server.home, stack, probe, profiles[1])
        server.close()
    with ExitStack() as stack:
        model = LocalModel(stack)
        server = Server(binary, root / "conversation", stack, probe)
        configure(server, model.http.server_port)
        conversation(binary, server.home, model)
        server.close(profiles[2] if probe else None)
    checks(binary, root)
    return profiles
