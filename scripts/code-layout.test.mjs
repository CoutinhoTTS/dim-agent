import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';

const directory=fileURLToPath(new URL('./code-layout/',import.meta.url));

test('Linux layout relinks the same native inputs and preserves executable behavior', {
  skip:process.platform!=='linux'||process.arch!=='x64',
  timeout:60_000,
},()=>{
  const result=spawnSync('python3',['-',directory],{encoding:'utf8',timeout:55_000,input:String.raw`
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
sys.path.insert(0, sys.argv[1])
import build
from train import begin, finish, terminate

with tempfile.TemporaryDirectory(prefix='dim-layout-test-') as temporary:
    root = Path(temporary)
    source = root / 'main.c'
    source.write_text('''#include <stdio.h>
__attribute__((noinline)) int first(void) { return 17; }
__attribute__((noinline)) int second(void) { return 25; }
__attribute__((noinline)) static int shared_name(void) { return 0; }
__attribute__((noinline)) static int discarded_name(void) { return 0; }
int main(void) { printf("%d\\n", first() + second() + shared_name() + discarded_name()); return 0; }
''')
    library_source = root / 'shared.c'
    library_source.write_text('int shared_name(void) { return 7; }\n')
    library = root / 'shared.so'
    subprocess.run(['cc','-shared','-fPIC',str(library_source),'-o',str(library)],check=True)
    obj = root / 'main.o'
    spare_source = root / 'spare.c'
    spare_source.write_text('__attribute__((used,noinline)) static int discarded_name(void) { return 7; }\n')
    spare = root / 'spare.o'
    subprocess.run(['cc','-O0','-ffunction-sections','-c',str(spare_source),'-o',str(spare)],check=True)
    base = root / 'base'
    subprocess.run(['cc', '-O0', '-ffunction-sections', '-c', str(source), '-o', str(obj)], check=True)
    sysroot = Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip())
    linker_dir = sysroot / 'lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld'
    record = {'compiler':'cc', 'arguments':['-B'+str(linker_dir), '-fuse-ld=lld', str(obj),
              str(spare), '-Wl,--gc-sections', '-Wl,--no-as-needed', str(library), '-Wl,--as-needed', '-o', str(base)],
              'cwd':str(root), 'path':os.environ['PATH']}
    build.relink(record, base)
    original = build.symbols(base)
    order = root / 'order'
    order.write_text('second\nfirst\nmain\n')
    changed = root / 'changed'
    build.relink(record, changed, order)
    reordered = build.symbols(changed)
    assert sorted((n,e-s) for s,e,n in original) == sorted((n,e-s) for s,e,n in reordered)
    addresses = {n:s for s,e,n in reordered}
    assert addresses['second'] < addresses['first'] < addresses['main']
    assert subprocess.check_output([str(base)]) == subprocess.check_output([str(changed)]) == b'42\n'
    probe = root / 'probe.so'
    subprocess.run(['cc','-shared','-fPIC','-O2','-Wall','-Wextra','-Werror',
                    str(build.HERE/'probe.c'),'-ldl','-o',str(probe)],check=True)
    process = subprocess.Popen([str(base)], env=dict(os.environ,LD_PRELOAD=str(probe)),
                               stdout=subprocess.PIPE, start_new_session=True)
    try:
        begin(process)
        profile = root / 'lifecycle.smaps'
        finish(process, profile)
        assert process.stdout.read() == b'42\n'
        measurement = build.order(base, [profile,profile,profile], root/'trained.order')
        assert measurement['orderedFunctions'] > 0
        trained = root / 'trained'
        build.ordered_link(record, trained, root/'trained.order')
        assert subprocess.check_output([str(trained)]) == b'42\n'
    finally:
        terminate(process)
        process.stdout.close()
`});
  assert.equal(result.error,undefined);
  assert.equal(result.status,0,result.stdout+result.stderr);
});
