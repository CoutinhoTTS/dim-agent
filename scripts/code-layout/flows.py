"""Offline user flows shared by the layout trainers on every platform."""

import http.server
import json
import os
import socketserver
import subprocess
import threading

_TOOL_MARKER = "layout-probe-marker"
_UNSET = ("LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "DIM_LAYOUT_SAMPLE", "DIM_LAYOUT_EXIT_SAMPLE")


def environment(home, extra=None):
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("DIM_", "DIMCODE_"))
        and key.upper() not in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "LD_PRELOAD")
        and key not in _UNSET
    }
    env.update(
        DIMCODE_HOME=str(home),
        NO_PROXY="127.0.0.1,localhost",
        TERM="xterm-256color",
        COLORTERM="truecolor",
        LANG="C.UTF-8",
    )
    if extra:
        env.update(extra)
    return env


class LocalModel:
    """A loopback OpenAI-compatible fixture that runs one real read tool."""

    def __init__(self, stack):
        self.tool_seen = threading.Event()
        owner = self

        class LoopbackServer(http.server.ThreadingHTTPServer):
            def server_bind(self):
                socketserver.TCPServer.server_bind(self)
                self.server_name, self.server_port = self.server_address

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                self.connection.settimeout(30)
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                has_result = any(
                    m.get("role") == "tool" and _TOOL_MARKER in json.dumps(m.get("content"))
                    for m in request.get("messages", [])
                )
                if has_result:
                    owner.tool_seen.set()

                def frame(delta, finish_reason):
                    return "data: " + json.dumps(
                        {
                            "id": "layout",
                            "choices": [
                                {"index": 0, "delta": delta, "finish_reason": finish_reason}
                            ],
                        }
                    ) + "\n\n"

                tools = request.get("tools", [])
                if not has_result and any(
                    t.get("function", {}).get("name") == "read" for t in tools
                ):
                    payload = frame(
                        {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "layout-read",
                                    "type": "function",
                                    "function": {"name": "read", "arguments": '{"path":"marker.txt"}'},
                                }
                            ]
                        },
                        None,
                    )
                    payload += frame({}, "tool_calls")
                else:
                    payload = frame(
                        {"content": "layout-probe-complete" if has_result else "Offline fixture"},
                        "stop",
                    )
                data = (payload + "data: [DONE]\n\n").encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)

        self.http = LoopbackServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        thread.start()
        stack.callback(thread.join)
        stack.callback(self.http.server_close)
        stack.callback(self.http.shutdown)


def configure(server, port):
    provider = "018ec73f-0000-7000-8000-000000000007"
    server.rpc(
        "provider.create",
        {
            "providerId": provider,
            "displayName": "Layout fixture",
            "connection": {
                "driverKind": "openai-compatible",
                "baseUrl": f"http://127.0.0.1:{port}/v1",
                "auth": {"type": "anonymous"},
            },
            "enabled": True,
            "models": [
                {
                    "modelId": "fixture",
                    "displayName": "Fixture",
                    "status": "active",
                    "capabilities": {
                        "modalities": ["text"],
                        "toolCalling": True,
                        "streaming": True,
                        "structuredOutput": False,
                        "vision": False,
                        "audio": False,
                        "reasoning": False,
                        "contextWindow": 128000,
                        "maxOutputTokens": 4096,
                    },
                }
            ],
        },
    )
    server.rpc("provider.select", {"providerId": provider, "modelId": "fixture"})


def conversation(binary, home, model):
    """Run one real read tool through the CLI against the loopback fixture."""
    (home / "marker.txt").write_text(_TOOL_MARKER + "\n")
    result = subprocess.run(
        [
            str(binary),
            "--home",
            str(home),
            "--cwd",
            str(home),
            "exec",
            "--prompt",
            "Read marker.txt and report completion.",
        ],
        cwd=home,
        env=environment(home),
        capture_output=True,
        text=True,
        timeout=120,
    )
    if (
        result.returncode
        or "layout-probe-complete" not in result.stdout
        or not model.tool_seen.is_set()
    ):
        raise RuntimeError(f"tool conversation failed: {result.stdout}\n{result.stderr}")


def tui_pty(binary, home, stack, env, terminate, begin=None, sample=None):
    """Drive the full-screen TUI through three model-picker cycles on a Unix pty."""
    import fcntl
    import os
    import pty
    import re
    import select
    import struct
    import subprocess
    import termios
    import time

    master, slave = pty.openpty()
    stack.callback(os.close, master)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))

    def terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    try:
        client = subprocess.Popen(
            [str(binary), "tui", "--home", str(home), "--cwd", str(home), "--fullscreen"],
            cwd=home,
            env=env,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            preexec_fn=terminal,
        )
    finally:
        os.close(slave)
    stack.callback(terminate, client)
    if begin:
        begin(client)
    capture = bytearray()

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], .02)[0]:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                capture.extend(chunk)
                if b"\x1b[6n" in chunk:
                    os.write(master, b"\x1b[1;1R")
                if b"\x1b]11;?" in chunk:
                    os.write(master, b"\x1b]11;rgb:0000/0000/0000\x1b\\")

    drain(2)
    for _ in range(3):
        start = len(capture)
        os.write(master, b"/models\r")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            drain(.05)
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", bytes(capture[start:]))
            if b"Models" in plain and b"Enter select" in plain:
                break
        else:
            raise TimeoutError(f"TUI model picker did not render: {plain[-2000:]!r}")
        os.write(master, b"\x1b")
        drain(.2)
    if sample:
        sample(client)
    os.write(master, b"/exit\r")
    drain(.5)
    if client.wait(timeout=30) != 0:
        raise RuntimeError(f"TUI process failed: {client.returncode}")


def checks(binary, home):
    for arguments in (["--version"], ["--help"], ["app-server", "--help"]):
        subprocess.run(
            [str(binary), *arguments],
            env=environment(home),
            stdout=subprocess.DEVNULL,
            check=True,
            timeout=30,
        )
