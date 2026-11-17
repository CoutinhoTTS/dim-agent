import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const directory=fileURLToPath(new URL('./code-layout/',import.meta.url));
const python=process.platform==='win32'?'python':'python3';

test('Windows layout orders lld COFF functions and samples native working sets', {
  skip:process.platform!=='win32',
  timeout:120_000,
},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:115_000,input:String.raw`
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
sys.path.insert(0, sys.argv[1])
import build_windows
import pe
import win32
from samples import write as write_sample

with tempfile.TemporaryDirectory(prefix='dim-layout-win-') as temporary:
    root = Path(temporary)
    (root / 'fixture.c').write_text('''#include <windows.h>
__declspec(dllexport) int public_func(int x) { return x + 3; }
static __declspec(noinline) int internal_func(int x) { return x * 5; }
static __declspec(noinline) int internal_two(int x) { return x - 7; }
typedef int (*fn)(int);
static fn volatile f1 = internal_func;
static fn volatile f2 = internal_two;
void entry(void) {
    volatile int value = public_func(f1(2)) + f2(3);
    ExitProcess(value ? 42 : 1);
}
''')
    subprocess.run(['cl','/nologo','/c','/Gy','/Od',str(root/'fixture.c'),
                    '/Fo:'+str(root/'fixture.obj')],check=True)
    linker = shutil.which('lld-link')
    assert linker, 'lld-link must be on PATH'
    record = {'compiler':linker,
              'arguments':['/entry:entry','/subsystem:console','/nodefaultlib',
                           str(root/'fixture.obj'),'kernel32.lib',
                           '/OUT:'+str(root/'base.exe')],
              'cwd':str(root),'path':os.environ['PATH'],'pdb':None}
    base = root/'base.exe'; base_map = root/'base.map'
    build_windows.relink(record, base, map_path=base_map)
    assert subprocess.run([str(base)]).returncode == 42
    order = root/'order'
    order.write_text('internal_two\ninternal_func\nentry\npublic_func\n')
    candidate = root/'candidate.exe'; candidate_map = root/'candidate.map'
    assert build_windows.ordered_link(record, candidate, order, candidate_map) == []
    assert subprocess.run([str(candidate)]).returncode == 42
    measurement = build_windows.validate(base_map, candidate_map, order)
    assert measurement['movedFunctionSymbols'] > 0
    missing = root/'missing.order'
    missing.write_text('absent_symbol\ninternal_two\ninternal_func\nentry\npublic_func\n')
    assert build_windows.ordered_link(record, root/'missing.exe', missing, root/'missing.map') == ['absent_symbol']
    assert 'absent_symbol' not in missing.read_text()
    samples = root/'samples.pages'
    write_sample(samples, 4096, [0])
    training = build_windows.order(base_map, [samples], root/'trained.order')
    assert training['orderedFunctions'] > 0
    assert 'entry' in (root/'trained.order').read_text()

    capture = root/'capture'; capture.mkdir()
    response = root/'link.rsp'
    response.write_text(' '.join(['/entry:entry','/subsystem:console','/nodefaultlib',
                                  str(root/'fixture.obj'),'kernel32.lib',
                                  '/OUT:'+str(root/'captured.exe')]))
    os.environ['DIM_LAYOUT_CAPTURE'] = str(capture)
    os.environ['DIM_LAYOUT_LINKER'] = linker
    build_windows.capture(['@'+str(response)])
    recorded = __import__('json').loads((capture/'link.json').read_text())
    assert recorded['inputs']
    assert recorded['output'].endswith('captured.exe')

    canonical = root/'canonical.rs'
    canonical.write_text('fn main() { let p = std::env::args().nth(1).unwrap(); println!("{}", std::fs::canonicalize(p).unwrap().display()); }')
    cl = shutil.which('cl')
    assert cl, 'cl must be on PATH'
    msvc = str(Path(cl).with_name('link.exe'))
    subprocess.run(['rustc', '-Clinker='+msvc, str(canonical), '-o', str(root/'canonical.exe')], check=True)
    expected = subprocess.check_output([str(root/'canonical.exe'), str(root)], text=True).strip()
    assert win32.canonical_path(root) == expected, (win32.canonical_path(root), expected)

    process = subprocess.Popen(['cmd.exe','/c','ping','-n','5','127.0.0.1'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                               creationflags=win32.CREATE_SUSPENDED)
    handle = win32.open_process(process.pid)
    try:
        win32.empty_working_set(handle)
        win32.resume(handle)
        time.sleep(.2)
        base_address = win32.module(handle)[0]
        _, _, size = win32.module(handle)
        rva, text_size = pe.text_range(Path(os.environ['SystemRoot'])/'System32'/'cmd.exe')
        page_size, pages = win32.working_set_pages(handle, base_address, rva, text_size)
        assert pages
    finally:
        win32.close(handle)
        process.kill(); process.wait()

    console = win32.Console(['cmd.exe','/c','echo','layout-conpty'], root, dict(os.environ),
                            suspended=False, inherit_environment=True)
    try:
        capture = bytearray()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and b'layout-conpty' not in capture:
            capture.extend(console.take())
            time.sleep(.02)
        assert b'layout-conpty' in capture, (bytes(capture[-2000:]), console.error, console.poll())
        assert console.wait(timeout=20) == 0
    finally:
        console.close()
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});
