import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const directory=fileURLToPath(new URL('./code-layout/',import.meta.url));
const python=process.platform==='win32'?'python':'python3';

test('macOS layout orders Mach-O functions and samples __text residency', {
  skip:process.platform!=='darwin',
  timeout:120_000,
},()=>{
  const result=spawnSync(python,['-',directory],{encoding:'utf8',timeout:115_000,input:String.raw`
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
sys.path.insert(0, sys.argv[1])
import build_darwin
import macho
import samples

with tempfile.TemporaryDirectory(prefix='dim-layout-mac-') as temporary:
    root = Path(temporary)
    (root / 'fixture.c').write_text('''__attribute__((noinline)) int first(void) { return 17; }
__attribute__((noinline)) int second(void) { return 25; }
int main(void) { return first() + second(); }
''')
    obj = root / 'fixture.o'
    subprocess.run(['cc','-O0','-c',str(root/'fixture.c'),'-o',str(obj)],check=True)
    record = {'compiler':'cc','arguments':['-o',str(root/'base'),str(obj)],
              'cwd':str(root),'path':os.environ['PATH']}
    base = root/'base'; base_map = root/'base.map'
    build_darwin.relink(record, base, map_path=base_map)
    assert subprocess.run([str(base)]).returncode == 42
    entries = macho.map_symbols(base_map, *macho.text_range(base)[:2])
    assert {name for _,_,name in entries} >= {'_first','_second','_main'}
    order = root/'order'
    order.write_text('_second\n_first\n_main\n')
    candidate = root/'candidate'; candidate_map = root/'candidate.map'
    assert build_darwin.ordered_link(record, candidate, order, candidate_map) == []
    assert subprocess.run([str(candidate)]).returncode == 42
    measurement = build_darwin.validate(base, candidate, base_map, candidate_map, order)
    assert measurement['movedFunctionSymbols'] > 0
    missing = root/'missing.order'
    missing.write_text('_absent\n_second\n_first\n_main\n')
    assert build_darwin.ordered_link(record, root/'missing', missing, root/'missing.map') == ['_absent']
    assert '_absent' not in missing.read_text()

    page_size = os.sysconf('SC_PAGESIZE')
    page_sample = root/'sample.pages'
    samples.write(page_sample, page_size, [0])
    training = build_darwin.order(base, base_map, [page_sample], root/'trained.order')
    assert training['orderedFunctions'] > 0
    assert '_main' in (root/'trained.order').read_text()

    probe = root/'probe.dylib'
    subprocess.run(['cc','-dynamiclib','-fPIC','-O2','-Wall','-Wextra','-Werror',
                    str(build_darwin.HERE/'probe_darwin.c'),'-o',str(probe)],check=True)
    exit_sample = root/'exit.pages'
    environment = dict(os.environ, DYLD_INSERT_LIBRARIES=str(probe),
                       DIM_LAYOUT_EXIT_SAMPLE=str(exit_sample))
    assert subprocess.run([str(base)], env=environment).returncode == 42
    observed, pages = samples.read(exit_sample)
    assert observed == page_size
    assert pages

    signal_sample = root/'signal.pages'
    sleeper = root/'sleeper'
    (root / 'sleeper.c').write_text('#include <unistd.h>\nint main(void) { sleep(5); return 0; }\n')
    subprocess.run(['cc','-O0','-o',str(sleeper),str(root/'sleeper.c')],check=True)
    sleeper_error = root/'sleeper.stderr'
    process = subprocess.Popen([str(sleeper)], env=dict(os.environ,
        DYLD_INSERT_LIBRARIES=str(probe), DIM_LAYOUT_SAMPLE=str(signal_sample)),
        stderr=sleeper_error.open('wb'))
    try:
        time.sleep(.5)
        process.send_signal(signal.SIGUSR1)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if signal_sample.exists() and signal_sample.stat().st_size:
                break
            time.sleep(.02)
        assert signal_sample.exists() and signal_sample.stat().st_size, (
            process.poll(), sleeper_error.read_text())
    finally:
        process.kill()
        process.wait()
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});
