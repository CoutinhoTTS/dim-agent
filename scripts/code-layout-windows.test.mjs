import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const directory=fileURLToPath(new URL('./code-layout/',import.meta.url));
const python=process.platform==='win32'?'python':'python3';

test('COFF layout reads map columns, bounds chunks and accepts reordered aliases',()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',input:String.raw`
from pathlib import Path
import sys
import tempfile
sys.path.insert(0, sys.argv[1])
import build_windows
import pe
import samples

with tempfile.TemporaryDirectory(prefix='dim-layout-coff-') as temporary:
    root = Path(temporary)
    def write_map(path, chunks):
        lines = ['Address  Size     Align Out     In      Symbol', '00001000 00000040  4096 .text']
        for address, names in chunks:
            lines.append(f'{address:08x} 00000010    16         library.lib(member.obj):(.text)')
            lines.extend(f'{address:08x} 00000000     0                 {name}' for name in names)
        path.write_text('\n'.join(lines)+'\n', encoding='utf-8', errors='surrogateescape')
    base, candidate, order = root/'base.map', root/'candidate.map', root/'symbols.order'
    write_map(base, [(0x1000, ['rust::first']), (0x1020, ['_Rrust_second','_Rrust_alias'])])
    write_map(candidate, [(0x1000, ['_Rrust_second','_Rrust_alias']), (0x1020, ['rust::first'])])
    functions, sizes = pe.symbols(base)
    assert functions[0] == (0,16,'rust::first'), 'padding before the next chunk is not function code'
    assert sorted(s for s in sizes if s is not None) == [16]
    order.write_text('_Rrust_alias\n_Rrust_second\nrust::first\n')
    assert build_windows.validate(base,candidate,order)['movedFunctionSymbols'] == 3
    order.write_text('rust::first\n_Rrust_second\n_Rrust_alias\n')
    try:
        build_windows.validate(base,candidate,order)
    except RuntimeError as error:
        assert 'do not follow' in str(error)
    else:
        raise AssertionError('different addresses still have to follow the order')
    raw = b'_Rraw_\xc0'.decode('utf-8', errors='surrogateescape')
    write_map(base, [(0x1000, [raw]), (0x1020, ['_Rsecond'])])
    write_map(candidate, [(0x1010, [raw]), (0x1030, ['_Rsecond'])])
    profile = root/'sample.pages'
    samples.write(profile, 4096, [0])
    build_windows.order(base, [profile], order)
    assert order.read_bytes() == b'_Rraw_\xc0\n_Rsecond\n'
    assert build_windows.validate(base,candidate,order)['movedFunctionSymbols'] == 2
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});

test('Windows layout orders lld COFF functions and samples native working sets', {
  skip:process.platform!=='win32',
  timeout:120_000,
},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:115_000,input:String.raw`
import os
from pathlib import Path
import shutil
import platform
import subprocess
import sys
import tempfile
import time
from contextlib import ExitStack
from unittest.mock import patch
sys.path.insert(0, sys.argv[1])
import build_windows
import pe
import win32
import train_windows
from samples import write as write_sample

artifacts = os.environ.get('DIM_LAYOUT_ARTIFACTS')
class Keep(tempfile.TemporaryDirectory):
    def cleanup(self):
        if artifacts:
            target = Path(artifacts)
            target.mkdir(parents=True, exist_ok=True)
            for pattern in ('*.map', '*.order', 'captured*.exe', 'captured*.pdb', 'capture/link.json',
                            'canonical*.exe', 'canonical*.pdb', 'rust-capture/*.map'):
                for path in Path(self.name).glob(pattern):
                    shutil.copy2(path, target / path.name)
        super().cleanup()

with Keep(prefix='dim-layout-win-') as temporary:
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
    subprocess.run(['cl','/nologo','/c','/Gy','/Od','/Z7',str(root/'fixture.c'),
                    '/Fo:'+str(root/'fixture.obj')],check=True)
    host = build_windows.TARGETS[platform.machine().upper()]
    linker = build_windows.lld_linker(host)
    assert linker, 'lld-link or rust-lld must be available'
    print(subprocess.check_output([*linker, '--version'], text=True).strip(), flush=True)
    record = {'compiler':linker,
              'arguments':['/entry:entry','/subsystem:console','/nodefaultlib',
                           str(root/'fixture.obj'),'kernel32.lib',
                           '/OUT:'+str(root/'base.exe')],
              'cwd':str(root),'path':os.environ['PATH'],'pdb':None,'linkEnv':{},
              'inputs':[], 'output':str(root/'base.exe')}
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
    natvis = root/'temporary.natvis'
    natvis.write_text('<AutoVisualizer xmlns="http://schemas.microsoft.com/vstudio/debugger/natvis/2010"><Type Name="LayoutValue"><DisplayString>layout</DisplayString></Type></AutoVisualizer>')
    response = root/'link.rsp'
    response.write_text(' '.join(build_windows.quote(argument) for argument in
        ['/entry:entry','/subsystem:console','/nodefaultlib','/DEBUG',
         '/PDBALTPATH:%_PDB%', '/PDB:'+str(root/'captured.pdb'),
         '/NATVIS:'+str(natvis),
         str(root/'fixture.obj'),'kernel32.lib','/OUT:'+str(root/'captured.exe')]))
    os.environ['DIM_LAYOUT_CAPTURE'] = str(capture)
    os.environ['DIM_LAYOUT_LINKER'] = __import__('json').dumps(linker)
    build_windows.capture(['@'+str(response)])
    recorded = __import__('json').loads((capture/'link.json').read_text())
    assert recorded['inputs']
    assert recorded['output'].endswith('captured.exe')
    captured = Path(recorded['output'])
    original = captured.read_bytes()
    (root/'captured-original.exe').write_bytes(original)
    shutil.copyfile(recorded['pdb'], root/'captured-original.pdb')
    assert (root/'captured.pdb').is_file()
    assert (capture/'baseline.map').is_file()
    (root/'fixture.obj').unlink()
    natvis.unlink()
    original_pdb = Path(recorded['pdb']).read_bytes()
    replay = root/'replay'; replay.mkdir()
    replayed, _ = build_windows.replay_baseline(recorded, capture, replay)
    assert captured.read_bytes() == original
    assert Path(recorded['pdb']).read_bytes() == original_pdb
    assert replayed.read_bytes() == original, ('captured inputs and map arguments must preserve the PE/PDB identity',
        recorded['output'], str(root/'captured.exe'), build_windows.digest(root/'captured-original.exe'), build_windows.digest(captured))
    assert not (root/'fixture.obj').exists() and not natvis.exists(), 'replay removes only the inputs it restored'
    assert subprocess.run([str(captured)]).returncode == 42

    canonical = root/'canonical.rs'
    canonical.write_text('''#[inline(never)] fn layout_first(x: u32) -> u32 { std::hint::black_box(x).wrapping_add(3) }
#[inline(never)] fn layout_second(x: u32) -> u32 { std::hint::black_box(x).wrapping_mul(5) }
fn main() { assert_eq!(layout_first(layout_second(2)), 13); let p = std::env::args().nth(1).unwrap(); println!("{}", std::fs::canonicalize(p).unwrap().display()); }
''')
    cl = shutil.which('cl')
    assert cl, 'cl must be on PATH'
    msvc = str(Path(cl).with_name('link.exe'))
    wrapper = root/'linker.exe'
    rustc = build_windows.rustc_command()
    subprocess.run([rustc, '-Clinker='+msvc, str(build_windows.HERE/'link_capture.rs'),
                    '-o', str(wrapper)], check=True)
    rust_capture = root/'rust-capture'; rust_capture.mkdir()
    environment = dict(os.environ, DIM_LAYOUT_CAPTURE=str(rust_capture),
        DIM_LAYOUT_PYTHON=sys.executable, DIM_LAYOUT_SCRIPT=str(build_windows.HERE/'build_windows.py'))
    subprocess.run([rustc, '-Clinker='+str(wrapper), '-Clto=fat', '-Ccodegen-units=1',
                    '-Cstrip=debuginfo', str(canonical), '-o', str(root/'canonical.exe')],
                   env=environment, check=True)
    recorded = __import__('json').loads((rust_capture/'link.json').read_text())
    executable = Path(recorded['output'])
    original = executable.read_bytes()
    (root/'canonical-original.exe').write_bytes(original)
    build_windows.relink(recorded, executable, preserve_pdb=True)
    assert executable.read_bytes() == original, 'replay must retain Rust temporary NatVis inputs'
    expected = subprocess.check_output([str(root/'canonical.exe'), str(root)], text=True).strip()
    assert win32.canonical_path(root) == expected, (win32.canonical_path(root), expected)
    rust_symbols, _ = pe.symbols(rust_capture/'baseline.map')
    selected = [name for _, _, name in rust_symbols if 'layout_first' in name or 'layout_second' in name]
    assert len(selected) == 2 and all('::' not in name for name in selected), selected
    rust_order = root/'rust.order'; rust_order.write_text('\n'.join(reversed(selected))+'\n')
    rust_candidate = root/'rust-candidate.exe'; rust_map = root/'rust.map'
    assert build_windows.ordered_link(recorded, rust_candidate, rust_order, rust_map) == []
    assert build_windows.validate(rust_capture/'baseline.map', rust_map, rust_order)['movedFunctionSymbols'] > 0
    assert subprocess.check_output([str(rust_candidate), str(root)], text=True).strip() == expected

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

    (root / 'echoer.c').write_text('''#include <windows.h>
#include <stdio.h>
int main(void) {
    DWORD mode;
    if (!GetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), &mode)) return 10;
    if (!GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mode)) return 11;
    if (!GetConsoleMode(GetStdHandle(STD_ERROR_HANDLE), &mode)) return 12;
    printf("layout-conpty-out\\n");
    fprintf(stderr, "layout-conpty-err\\n");
    fflush(stdout);
    fflush(stderr);
    return getchar() == '\\n' ? 0 : 13;
}
''')
    subprocess.run(['cl','/nologo',str(root/'echoer.c'),'/Fe:'+str(root/'echoer.exe')],check=True)
    console = win32.Console([str(root/'echoer.exe')], root, dict(os.environ))
    try:
        capture = bytearray()
        deadline = time.monotonic() + 20
        markers = (b'layout-conpty-out', b'layout-conpty-err')
        while time.monotonic() < deadline and not all(marker in capture for marker in markers):
            capture.extend(console.take())
            time.sleep(.02)
        assert all(marker in capture for marker in markers), (bytes(capture[-2000:]), console.error, console.poll())
        assert console.poll() is None, 'the child must wait for input through ConPTY'
        console.write(b"\r")
        assert console.wait(timeout=20) == 0
    finally:
        console.terminate()
        console.close()

    original_console = win32.Console
    handles = []
    clock = [0]
    def clock_now():
        clock[0] += .1
        return clock[0]
    def fixture_console(*args):
        child = original_console([str(root/'echoer.exe')], root, dict(os.environ))
        handles.append(win32.open_process(child.pid))
        return child
    try:
        with patch.object(win32, 'Console', side_effect=fixture_console), \
             patch.object(train_windows.time, 'monotonic', side_effect=clock_now), \
             patch.object(train_windows.time, 'sleep', side_effect=lambda _: None):
            try:
                with ExitStack() as stack:
                    train_windows.tui(root/'unused.exe', root, stack, False, None)
            except TimeoutError as error:
                assert 'model picker' in str(error)
            else:
                raise AssertionError('the fixture never renders a model picker')
        assert len(handles) == 1
        code = win32.wintypes.DWORD()
        assert win32.kernel32.GetExitCodeProcess(handles[0], win32.ctypes.byref(code))
        assert code.value != 259, 'a failed TUI flow must not leave its child running'
    finally:
        for handle in handles:
            win32.close(handle)
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});
