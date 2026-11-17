import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const directory=fileURLToPath(new URL('./code-layout/',import.meta.url));
const python=process.platform==='win32'?'python':'python3';

test('interrupted native replay restores Cargo files after stopping its tools', {timeout:60_000},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:55_000,input:String.raw`
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

with tempfile.TemporaryDirectory(prefix='dim-layout-replay-') as temporary:
    root = Path(temporary)
    tool = root/'link.py'
    tool.write_text('''import os
from pathlib import Path
import subprocess
import sys
import time
root = Path.cwd()
(root/'cargo').write_bytes(b'PARTIAL REPLAY')
(root/'cargo.pdb').write_bytes(b'PARTIAL PDB')
if os.name != 'nt':
    subprocess.Popen([sys.executable, '-c', "import time; from pathlib import Path; time.sleep(1); Path('cargo').write_bytes(b'LATE CHILD WRITE')"])
(root/'ready').write_text(str(os.getpid()))
time.sleep(20)
(root/'cargo').write_bytes(b'LATE LINKER WRITE')
''')
    driver = root/'driver.py'
    driver.write_text('''import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
sys.path.insert(0, sys.argv[1])
import build_darwin
import build_windows
root = Path.cwd()
mode, number = sys.argv[2], int(sys.argv[3])
def cancel():
    deadline = time.monotonic() + 10
    while not (root/'ready').exists():
        if time.monotonic() >= deadline:
            os._exit(90)
        time.sleep(.01)
    if os.name == 'nt':
        signal.raise_signal(number)
    else:
        os.kill(os.getpid(), number)
threading.Thread(target=cancel, daemon=True).start()
record = {'cwd':str(root), 'path':os.environ['PATH'], 'output':str(root/'cargo'),
          'strip':'none', 'linkEnv':{}, 'pdb':str(root/'cargo.pdb')}
if mode == 'windows':
    record.update(compiler=[sys.executable, sys.argv[4]], arguments=['/OUT:'+str(root/'cargo')])
    replay = build_windows.replay_baseline
else:
    record.update(compiler=sys.executable, arguments=[sys.argv[4], '-o', str(root/'cargo')])
    if mode == 'strip':
        record.update(arguments=['-c', 'pass', '-o', str(root/'cargo')],
                      strip='debuginfo', objcopy=str(root/'strip'))
    replay = build_darwin.replay_baseline
try:
    replay(record, root/'work', root/'out')
except SystemExit:
    if os.name != 'nt':
        try:
            child, _ = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            pass
        else:
            raise AssertionError(('replay did not reap its tool', child))
    raise
raise AssertionError('interrupted replay returned a baseline')
''')
    cases = [('darwin', True), ('windows', True), ('windows', False)]
    signals = [signal.SIGTERM, signal.SIGINT]
    if os.name == 'nt':
        signals.append(signal.SIGBREAK)
    if os.name != 'nt':
        cases.append(('strip', True))
    for mode, had_pdb in cases:
        for number in signals:
            case = root/(mode+str(had_pdb)+str(number)); case.mkdir()
            (case/'work').mkdir(); (case/'out').mkdir()
            (case/'work/baseline.map').write_text('fixture map')
            (case/'cargo').write_bytes(b'ORIGINAL')
            if had_pdb:
                (case/'cargo.pdb').write_bytes(b'ORIGINAL PDB')
            if mode == 'strip':
                import shlex
                wrapper = case/'strip'
                wrapper.write_text('#!/bin/sh\nexec '+shlex.join([sys.executable, str(tool)])+' "$@"\n')
                wrapper.chmod(0o700)
            process = subprocess.run([sys.executable, str(driver), sys.argv[1], mode,
                                      str(int(number)), str(tool)], cwd=case,
                                     capture_output=True, text=True, timeout=15)
            assert process.returncode == 128+number, (mode, process.returncode, process.stderr)
            time.sleep(1.1)
            assert (case/'cargo').read_bytes() == b'ORIGINAL', mode
            if mode == 'windows':
                if had_pdb:
                    assert (case/'cargo.pdb').read_bytes() == b'ORIGINAL PDB'
                else:
                    assert not (case/'cargo.pdb').exists()
            assert not list((case/'out').glob('baseline*exe'))
            assert not (case/'out/baseline').exists()
            assert not list((case/'out').glob('dimcode*'))
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});

test('Windows shutdown sampling expires and releases the training process', {timeout:15_000},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:10_000,input:String.raw`
from contextlib import ExitStack
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch
sys.path.insert(0, sys.argv[1])
sys.modules['win32'] = SimpleNamespace()
import train_windows

with tempfile.TemporaryDirectory(prefix='dim-layout-shutdown-') as temporary:
    root = Path(temporary)
    profile = root/'shutdown.pages'
    for sample in ('success', 'unavailable'):
        clock = [0]
        requests, handles = [], []
        process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        server = train_windows.Server.__new__(train_windows.Server)
        server.process, server.binary = process, root/'fixture.exe'
        server.sequence = 0
        server.wire = SimpleNamespace(write=requests.append)

        def pages(*args):
            if not requests:
                return 4096, [0]
            if sample == 'unavailable':
                clock[0] = 60
                raise OSError('process sample unavailable')
            clock[0] += 31
            return 4096, [0]

        native = SimpleNamespace(open_process=lambda pid: pid, module=lambda handle: (0,),
            close=handles.append, working_set_pages=pages, page_size=lambda: 4096)
        with patch.object(train_windows, 'win32', native), \
             patch.object(train_windows.pe, 'text_range', return_value=(0,4096)), \
             patch.object(train_windows.time, 'monotonic', side_effect=lambda: clock[0]), \
             patch.object(train_windows.time, 'sleep', side_effect=lambda _: None):
            try:
                with ExitStack() as stack:
                    stack.callback(train_windows.terminate, process)
                    server.close(profile)
            except (TimeoutError, subprocess.TimeoutExpired):
                pass
            else:
                raise AssertionError('an alive training process passed shutdown acceptance')
        assert requests[-1]['method'] == 'daemon.shutdown'
        assert process.poll() is not None, 'the owned training process must be terminated and reaped'
        assert handles == [process.pid], 'the sampling handle must be closed'
        assert not profile.exists(), 'timed-out shutdown must not publish a sample'

    with ExitStack() as stack:
        process = subprocess.Popen([sys.executable, '-c', 'input()'], stdin=subprocess.PIPE)
        stack.callback(train_windows.terminate, process)
        stack.callback(process.stdin.close)
        def shutdown(value):
            process.stdin.write(b'\n'); process.stdin.flush()
            assert process.wait(timeout=5) == 0
        server = train_windows.Server.__new__(train_windows.Server)
        server.process, server.binary = process, root/'fixture.exe'
        server.sequence = 0
        server.wire = SimpleNamespace(write=shutdown)
        native = SimpleNamespace(open_process=lambda pid: pid, module=lambda handle: (0,),
            close=lambda handle: None, working_set_pages=lambda *args: (4096, [0,1]), page_size=lambda: 4096)
        with patch.object(train_windows, 'win32', native), \
             patch.object(train_windows.pe, 'text_range', return_value=(0,8192)):
            server.close(profile)
        assert profile.read_text() == 'pagesize 4096\n0\n4096\n', 'a fast successful shutdown keeps the pre-shutdown sample'
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});
