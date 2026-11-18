import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const directory=fileURLToPath(new URL('./code-layout/',import.meta.url));
const python=process.platform==='win32'?'python':'python3';

test('native capture preserves the first link independently of saved inputs', {timeout:30_000},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:25_000,input:String.raw`
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
from unittest.mock import patch
sys.path.insert(0, sys.argv[1])
import build_darwin
import build_windows
import replay

with tempfile.TemporaryDirectory(prefix='dim-layout-capture-') as temporary:
    root = Path(temporary)
    tool = root/'link.py'
    tool.write_text('''from pathlib import Path
import sys
args = sys.argv[1:]
if args[0].startswith('@'):
    args = [line.removeprefix('"').removesuffix('"') for line in Path(args[0][1:]).read_text().splitlines()]
source = Path(next(arg for arg in args if arg.endswith('.obj')))
payload = str(source).encode() + b'\\n' + str(source.stat().st_mtime_ns).encode() + b'\\n' + source.read_bytes()
if '-o' in args:
    output = Path(args[args.index('-o')+1])
    map_path = Path(next(arg for arg in args if arg.startswith('-Wl,-map,')).split(',',2)[2])
else:
    output = Path(next(arg[5:] for arg in args if arg.startswith('/OUT:')))
    map_path = Path(next(arg[8:] for arg in args if arg.startswith('/lldmap:')))
    Path(next(arg[5:] for arg in args if arg.startswith('/PDB:'))).write_bytes(payload)
map_path.write_text('fixture map')
output.write_bytes(payload)
''')
    for module in (build_darwin, build_windows):
        for corrupt in (False, True):
            case = root/(module.__name__+str(corrupt)); case.mkdir()
            work = case/'work'; work.mkdir()
            output = case/'out'; output.mkdir()
            source = case/'rust-temp/input.obj'; source.parent.mkdir()
            source.write_bytes(b'ORIGINAL INPUT')
            os.utime(source, ns=(1000000000000000000, 1000000000000000000))
            cargo = case/'cargo'; pdb = case/'cargo.pdb'
            environment = dict(os.environ, DIM_LAYOUT_CAPTURE=str(work),
                DIM_LAYOUT_CC=sys.executable, DIM_LAYOUT_LINKER=json.dumps([sys.executable,str(tool)]),
                DIM_LAYOUT_STRIP='none', DIM_LAYOUT_DYLD_LIBRARY_PATH='')
            source_argument = str(source) if os.name == 'nt' else os.path.relpath(source)
            arguments = ([str(tool),source_argument,'-o',str(cargo)] if module == build_darwin
                         else [source_argument,'/OUT:'+str(cargo),'/PDB:'+str(pdb)])
            copy2 = shutil.copy2
            def save_input(src, dst, **kwargs):
                result = copy2(src, dst, **kwargs)
                if corrupt and Path(src).samefile(source):
                    Path(dst).write_bytes(b'WRONG SAVED INPUT')
                    shutil.copystat(src, dst)
                return result
            with patch.dict(os.environ, environment), patch.object(shutil, 'copy2', side_effect=save_input):
                module.capture(arguments)
            original = cargo.read_bytes()
            assert original.endswith(b'ORIGINAL INPUT'), 'the first link must use original inputs, even if capture is wrong'
            original_pdb = pdb.read_bytes() if pdb.exists() else None
            record = json.loads((work/'link.json').read_text())
            captured = next(item for item in record['inputs'] if Path(item['source']).samefile(source))
            assert Path(captured['file']).read_bytes() == (b'WRONG SAVED INPUT' if corrupt else b'ORIGINAL INPUT')
            shutil.rmtree(source.parent)
            try:
                baseline, _ = module.replay_baseline(record, work, output)
            except RuntimeError as error:
                assert corrupt and 'did not reproduce' in str(error), error
                assert (output/('baseline-replay' if module == build_darwin else 'baseline-replay.exe')).read_bytes().endswith(b'WRONG SAVED INPUT')
            else:
                assert not corrupt, 'deterministically wrong capture must fail baseline validation'
                assert baseline.read_bytes() == original
                replay.save_baseline(output, baseline, record, 'fixture', Path(sys.argv[1]))
                resumed = replay.load_baseline(output, baseline, 'fixture')
                candidate = output/'sorted'
                module.relink(resumed, candidate, map_path=output/'sorted.map')
                assert candidate.read_bytes() == original
                (output/'manifest.json').write_text(json.dumps(dict(target='fixture',
                    baselineSha256=replay.file_hash(baseline), candidateSha256=replay.file_hash(candidate))))
                replay.measure_artifacts(output, 'fixture', baseline, candidate, lambda: [42])
                assert json.loads((output/'manifest.json').read_text())['measurements'] == [42]
                candidate.write_bytes(b'changed')
                try:
                    replay.measure_artifacts(output, 'fixture', baseline, candidate, lambda: [])
                except ValueError as error:
                    assert 'validated layout manifest' in str(error), error
                else:
                    raise AssertionError('measurement used a different executable')
            assert cargo.read_bytes() == original, 'baseline validation must restore the original executable'
            if original_pdb is not None:
                assert pdb.read_bytes() == original_pdb
            assert not source.parent.exists(), 'replay must remove recreated Rust temporary inputs and directories'
            if not corrupt:
                source.parent.mkdir(); source.write_bytes(b'NEW BUILD INPUT')
                try:
                    module.replay_baseline(record, work, output)
                except RuntimeError as error:
                    assert 'input changed' in str(error), error
                else:
                    raise AssertionError('replay accepted an input from another build')
                assert source.read_bytes() == b'NEW BUILD INPUT', 'replay must not overwrite existing input files'
                assert cargo.read_bytes() == original
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});

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
          'strip':'none', 'linkEnv':{}, 'pdb':str(root/'cargo.pdb'),
          'inputs':[{'source':str(root/'rust-temp/input.obj'), 'file':str(root/'work/input.obj'),
                     'sha256':__import__('hashlib').sha256(b'INPUT').hexdigest()}]}
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
            (case/'work/input.obj').write_bytes(b'INPUT')
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
            assert not (case/'rust-temp').exists(), 'interrupted replay must remove the temporary inputs it restored'
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

test('native measurement uses separate disposable homes without reverse DNS', {timeout:15_000},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:10_000,input:String.raw`
from contextlib import ExitStack, closing
import faulthandler
from http.client import HTTPConnection
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch
from urllib.parse import urlsplit
sys.path.insert(0, sys.argv[1])
sys.modules['win32'] = SimpleNamespace()
import train_windows
import train_darwin
from samples import write
faulthandler.dump_traceback_later(8)

with tempfile.TemporaryDirectory(prefix='dim-layout-measure-') as temporary:
    root = Path(temporary)
    for trainer in (train_windows, train_darwin):
        processes = []
        class Process:
            def __init__(self, args, **options):
                self.home = Path(options['cwd'])
                self.pid = len(processes) + 1
                self.returncode = None
                self.stdout = tempfile.TemporaryFile()
                self.stdout.write(b'{"pipeName":"fixture","unixPath":"fixture"}\n')
                self.stdout.seek(0)
                processes.append(self)
            def poll(self): return self.returncode
            def send_signal(self, number): self.returncode = 0
            def kill(self): self.returncode = -9
            def wait(self, **options):
                assert self.returncode is not None, 'measurement must request shutdown'
                return self.returncode

        def rpc(process, method, params):
            if method == 'provider.create':
                (process.home/'model.url').write_text(params['connection']['baseUrl'])
            elif method == 'daemon.shutdown':
                process.returncode = 0
            return {}

        class Wire:
            def __init__(self, path, stack):
                self.process = next(p for p in processes if p.home == Path(path).parent.parent)
            def write(self, request):
                self.reply = {'id':request['id'], 'result':rpc(self.process, request['method'], request['params'])}
            def readline(self): return json.dumps(self.reply).encode() + b'\n'

        def client(args, **options):
            home = Path(options['cwd'])
            marker = (home/'marker.txt').read_text()
            data = json.dumps({'messages':[{'role':'tool','content':marker}]}).encode()
            url = urlsplit((home/'model.url').read_text())
            with closing(HTTPConnection(url.hostname,url.port,timeout=3)) as connection:
                connection.request('POST',url.path+'/chat/completions',data,{'Content-Type':'application/json'})
                assert b'layout-probe-complete' in connection.getresponse().read()
            return subprocess.CompletedProcess(args,0,'layout-probe-complete','')

        native = SimpleNamespace(CREATE_SUSPENDED=0, default_pipe=lambda home: str(home/'pipe'),
            open_process=lambda pid: pid, empty_working_set=lambda handle: None,
            suspend=lambda handle: None, resume=lambda handle: None, close=lambda handle: None,
            module=lambda handle: (0,), working_set_pages=lambda *args: (4096,[0,1]),
            memory=lambda handle: {'workingSet':12288,'private':4096,'pagefile':4096},
            cpu=lambda handle: {'user':.01,'kernel':.02})
        with ExitStack() as stack:
            stack.enter_context(patch('socket.gethostbyaddr',side_effect=AssertionError('loopback measurement must not require reverse DNS')))
            stack.enter_context(patch.object(trainer,'subprocess',SimpleNamespace(
                Popen=Process,run=client,PIPE=subprocess.PIPE)))
            if trainer is train_windows:
                stack.enter_context(patch.object(trainer,'win32',native))
                stack.enter_context(patch.object(trainer,'Wire',Wire))
                stack.enter_context(patch.object(trainer.pe,'text_range',return_value=(0,8192)))
                measured = trainer.measure(root/'baseline',root/'candidate',root/'windows',pairs=2)
            else:
                stack.enter_context(patch.object(trainer,'select',SimpleNamespace(select=lambda reads,*args: (reads,[],[]))))
                stack.enter_context(patch.object(trainer.Server,'rpc',lambda self,method,params: rpc(self.process,method,params)))
                stack.enter_context(patch.object(trainer,'sample',side_effect=lambda process,path: write(path,4096,[0,1])))
                stack.enter_context(patch.object(trainer,'_ps',return_value={'resident':12288,'cpu':.03}))
                stack.enter_context(patch.object(trainer,'_footprint',return_value=8192))
                measured = trainer.measure(root/'baseline',root/'candidate',root/'darwin',root/'probe',pairs=2)
        measured = json.loads(json.dumps(measured))
        assert set(measured) == {'baseline','candidate'}
        for observations in measured.values():
            assert len(observations) == 2
            for sample in observations:
                assert sample['pageSize'] == 4096 and sample['codePages'] == 2
                assert sample['codeResident'] == 8192 and sample['cpu'] == .03
                assert sample['wall'] >= 0 and sample['conversationWall'] >= 0
        assert len(processes) == len({p.home for p in processes}) == 8
        assert all((p.home/'rust').is_dir() for p in processes)
        assert all(p.returncode == 0 and p.stdout.closed for p in processes)
faulthandler.cancel_dump_traceback_later()
`});
  assert.equal(result.error,undefined,result.stdout+result.stderr);
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
