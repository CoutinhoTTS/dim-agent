#!/usr/bin/env python3
"""Build and train a native Windows executable, then reorder its functions.

The output directory must not exist. Cargo's executable is left unchanged; only
an independently smoke-tested relink is published as OUTPUT/dimcode.exe.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile

from layout import select, unique_functions, validate_sizes
import pe
from replay import preserve_cargo, run_link
from samples import read as read_sample
from toolchain import rustc as rustc_command

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TARGETS = {"AMD64": "x86_64-pc-windows-msvc", "ARM64": "aarch64-pc-windows-msvc"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def split_response(text):
    arguments = []
    index = 0
    while index < len(text):
        while index < len(text) and text[index] in " \t\r\n":
            index += 1
        if index >= len(text):
            break
        argument = []
        quoted = False
        while index < len(text) and (quoted or text[index] not in " \t\r\n"):
            if text[index] == "\\":
                count = 0
                while index < len(text) and text[index] == "\\":
                    count += 1
                    index += 1
                if index < len(text) and text[index] == '"':
                    argument.append("\\" * (count // 2))
                    if count % 2:
                        argument.append('"')
                        index += 1
                else:
                    argument.append("\\" * count)
            elif text[index] == '"':
                quoted = not quoted
                index += 1
            else:
                argument.append(text[index])
                index += 1
        arguments.append("".join(argument))
    return arguments


def expand(arguments):
    result = []
    for argument in arguments:
        if argument.startswith("@"):
            result.extend(expand(split_response(Path(argument[1:]).read_text())))
        else:
            result.append(argument)
    return result


def option(arguments, name, value):
    prefix = name.lower()
    found = False
    for index, argument in enumerate(arguments):
        if argument.lower().startswith(prefix):
            arguments[index] = f"{name}{value}"
            found = True
    return found


def quote(argument):
    return subprocess.list2cmdline([argument])


def relink(record, output, order_file=None, map_path=None, strict=False, preserve_pdb=False):
    arguments = record["arguments"].copy()
    if not option(arguments, "/OUT:", str(output)):
        raise RuntimeError("the captured link has no /OUT option")
    if record.get("pdb") and not preserve_pdb:
        option(arguments, "/PDB:", str(output.with_suffix(".pdb")))
    if map_path and not option(arguments, "/lldmap:", str(map_path)):
        arguments.append(f"/lldmap:{map_path}")
    if order_file:
        arguments.append(f"/order:@{order_file}")
    if strict:
        arguments.append("/WX")
    response = output.with_name(output.name + ".rsp")
    response.write_text("\n".join(quote(argument) for argument in arguments) + "\n")
    env = dict(os.environ, PATH=record["path"])
    try:
        result = run_link([*record["compiler"], "@" + str(response)], cwd=record["cwd"], env=env)
    finally:
        response.unlink(missing_ok=True)
    if result.returncode:
        raise RuntimeError(f"native link failed ({result.returncode}):\n{result.stderr}")
    return result.stderr


def capture(arguments):
    root = Path(os.environ["DIM_LAYOUT_CAPTURE"])
    linker = json.loads(os.environ["DIM_LAYOUT_LINKER"])
    arguments = list(arguments)
    if not any(argument.lower() == "/brepro" for argument in expand(arguments)):
        arguments.append("/Brepro")
    expanded = expand(arguments)
    saved = []
    inputs = []
    output = None
    pdb = None
    for index, argument in enumerate(expanded):
        lower = argument.lower()
        if lower.startswith("/out:"):
            output = str(Path(argument[5:]).resolve())
            saved.append("/OUT:" + output)
        elif lower.startswith("/pdb:"):
            pdb = str(Path(argument[5:]).resolve())
            saved.append("/PDB:" + pdb)
        elif lower.startswith("/natvis:") or (
                not argument.startswith(("/", "-")) and Path(argument).is_file()):
            prefix, source = ("/NATVIS:", argument[8:]) if lower.startswith("/natvis:") else ("", argument)
            target = root / f"{index}-{Path(source).name}"
            shutil.copyfile(source, target)
            saved.append(prefix + str(target))
            inputs.append({"file": target.name, "sha256": digest(target)})
        else:
            saved.append(argument)
    if not output or (root / "link.json").exists():
        raise RuntimeError("expected exactly one final executable link")
    if not option(saved, "/lldmap:", str(root / "baseline.map")):
        saved.append(f"/lldmap:{root / 'baseline.map'}")
    record = {"arguments": saved, "cwd": os.getcwd(), "compiler": linker,
              "output": output, "pdb": pdb, "inputs": inputs, "path": os.environ["PATH"]}
    (root / "link.json").write_text(json.dumps(record, indent=2) + "\n")
    relink(record, Path(output), preserve_pdb=True)


def replay_baseline(record, work, output):
    baseline = output / "baseline.exe"
    baseline_map = output / "baseline.map"
    shutil.copyfile(work / "baseline.map", baseline_map)
    cargo = Path(record["output"])
    backup = work / "cargo.exe"
    backup_pdb = work / "cargo.pdb"
    pdb = Path(record["pdb"]) if record.get("pdb") else cargo.with_suffix(".pdb")
    with preserve_cargo([(cargo, backup), (pdb, backup_pdb)]):
        relink(record, cargo, preserve_pdb=True)
        original_hash, replay_hash = digest(backup), digest(cargo)
        if replay_hash != original_hash:
            shutil.copyfile(backup, output / "cargo-original.exe")
            shutil.copyfile(cargo, output / "baseline-replay.exe")
            if backup_pdb.exists():
                shutil.copyfile(backup_pdb, output / "cargo-original.pdb")
                shutil.copyfile(pdb, output / "baseline-replay.pdb")
            (output / "replay.json").write_text(json.dumps(record, indent=2) + "\n")
            raise RuntimeError(
                "captured inputs did not reproduce Cargo's executable: "
                f"original={original_hash}, replay={replay_hash}")
        shutil.copyfile(cargo, baseline)
    return baseline, baseline_map


def ordered_link(record, output, order_file, map_path):
    diagnostics = relink(record, output, order_file, map_path, strict=False)
    names = order_file.read_text().splitlines()
    omitted = set()
    for line in diagnostics.splitlines():
        if not line.strip():
            continue
        match = re.fullmatch(r"(?:lld-link|rust-lld): warning: .*: missing symbol: (\S+) \[LNK4037\]", line)
        if not match or match[1] not in names:
            raise RuntimeError(f"unexpected native link diagnostic:\n{diagnostics}")
        omitted.add(match[1])
    if omitted:
        order_file.write_text("\n".join(name for name in names if name not in omitted) + "\n")
        relink(record, output, order_file, map_path, strict=True)
    return sorted(omitted)


def order(baseline_map, profiles, output):
    functions, _ = pe.symbols(baseline_map)
    page_size, footprints = None, []
    for profile in profiles:
        observed, pages = read_sample(profile)
        if page_size is None:
            page_size = observed
        elif observed != page_size:
            raise ValueError("training samples use different page sizes")
        footprints.append(pages)
    names = select(unique_functions(functions), footprints, page_size)
    output.write_text("\n".join(names) + "\n")
    return {"pages": [len(pages) for pages in footprints], "pageSize": page_size,
            "orderedFunctions": len(names), "functionSymbols": len(functions)}


def validate(baseline_map, candidate_map, order_file):
    before, before_sizes = pe.symbols(baseline_map)
    after, after_sizes = pe.symbols(candidate_map)
    return validate_sizes(before, after, before_sizes, after_sizes,
                          order_file.read_text().splitlines(), "PE function")


def lld_linker(host):
    found = shutil.which("lld-link")
    if found:
        return [found]
    for root in (
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")),
        Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")),
    ):
        for candidate in root.glob("Microsoft Visual Studio/2022/*/VC/Tools/Llvm/*/bin/lld-link.exe"):
            return [str(candidate)]
        for candidate in root.glob("LLVM/bin/lld-link.exe"):
            return [str(candidate)]
    rustc = rustc_command()
    sysroot = Path(subprocess.check_output(
        [rustc, "--print", "sysroot"], cwd=REPO, text=True).strip())
    for candidate in (
        sysroot / "lib/rustlib" / host / "bin" / "rust-lld.exe",
        sysroot / "bin" / "rust-lld.exe",
    ):
        if candidate.is_file():
            return [str(candidate), "-flavor", "link"]
    return None


def msvc_linker(host):
    key = "CARGO_TARGET_" + host.upper().replace("-", "_") + "_LINKER"
    if os.environ.get(key):
        return os.environ[key]
    compiler = shutil.which("cl")
    if compiler:
        candidate = Path(compiler).with_name("link.exe")
        if candidate.is_file():
            return str(candidate)
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", choices=sorted(TARGETS.values()))
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--pairs", type=int, default=2)
    args = parser.parse_args()
    machine = platform.machine().upper()
    if sys.platform != "win32" or machine not in TARGETS:
        parser.error("requires native Windows x86-64 or ARM64")
    host = TARGETS[machine]
    if args.target and args.target != host:
        parser.error("only the native Windows architecture is supported")
    if args.jobs < 1:
        parser.error("jobs must be positive")
    if args.pairs < 1:
        parser.error("pairs must be positive")
    if os.environ.get("CARGO_BUILD_TARGET") not in (None, host):
        parser.error("cross compilation is not supported")
    linker = lld_linker(host)
    if not linker:
        parser.error("lld-link or the Rust toolchain's rust-lld is required")
    version = subprocess.run([*linker, "--version"], capture_output=True, text=True)
    if version.returncode or "LLD" not in version.stdout + version.stderr:
        parser.error(f"cannot use the LLD linker at {linker[0]}")
    output = args.output.resolve()
    if output.exists():
        parser.error("the output directory must not exist")
    from train_windows import measure, train

    with tempfile.TemporaryDirectory(prefix="dim-layout-") as temporary:
        work = Path(temporary)
        wrapper = work / "linker.exe"
        rustc = rustc_command()
        host_linker = msvc_linker(host)
        if not host_linker:
            parser.error("the MSVC link.exe is required to compile the capture wrapper")
        subprocess.run(
            [rustc, "--edition", "2021", "-O", "-Clinker=" + host_linker,
             str(HERE / "link_capture.rs"), "-o", str(wrapper)],
            check=True)
        env = dict(
            os.environ,
            RUSTC=rustc,
            DIM_LAYOUT_CAPTURE=str(work),
            DIM_LAYOUT_LINKER=json.dumps(linker),
            DIM_LAYOUT_PYTHON=sys.executable,
            DIM_LAYOUT_SCRIPT=str(HERE / "build_windows.py"),
            **{"CARGO_TARGET_" + host.upper().replace("-", "_") + "_LINKER": host_linker},
        )
        if host == "aarch64-pc-windows-msvc":
            env.setdefault("CC_aarch64_pc_windows_msvc", "clang-cl")
            env.setdefault("CXX_aarch64_pc_windows_msvc", "clang-cl")
        command = [
            "cargo", "rustc", "--release", "--locked", "-p", "dimcode", "--bin", "dimcode",
            "--jobs", str(args.jobs), "--target", args.target or host,
        ]
        command.extend(["--", f"-Clinker={wrapper}"])
        subprocess.run(command, cwd=REPO, env=env, check=True)
        record = json.loads((work / "link.json").read_text())
        record["rustc"] = subprocess.check_output([rustc, "-vV"], cwd=REPO, text=True)
        output.mkdir(parents=True)
        baseline, baseline_map = replay_baseline(record, work, output)
        profiles = train(baseline, work / "training", sampling=True)
        for profile in profiles:
            shutil.copyfile(profile, output / profile.name)
        order_file = output / "symbols.order"
        measurement = order(baseline_map, profiles, order_file)
        candidate = output / "candidate.exe"
        candidate_map = output / "candidate.map"
        measurement["unorderableSymbols"] = ordered_link(
            record, candidate, order_file, candidate_map)
        measurement["orderedFunctions"] = len(order_file.read_text().splitlines())
        measurement.update(validate(baseline_map, candidate_map, order_file))
        train(candidate, work / "acceptance", sampling=False)
        measurements = measure(baseline, candidate, work / "measure", args.pairs)
        manifest = {
            "platform": "windows", "target": host, "pageSize": pe_page_size(),
            "lto": os.environ.get("CARGO_PROFILE_RELEASE_LTO", "profile"),
            "source": subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
            "dirty": bool(subprocess.check_output(
                ["git", "status", "--porcelain"], cwd=REPO)),
            "rustc": record["rustc"],
            "baselineSha256": digest(baseline),
            "candidateSha256": digest(candidate),
            "orderSha256": digest(order_file),
            "inputs": record["inputs"], "training": measurement,
            "measurements": measurements,
        }
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        candidate.rename(output / "dimcode.exe")
        print(f"Validated executable: {output / 'dimcode.exe'}", flush=True)


def pe_page_size():
    import win32
    return win32.page_size()


if __name__ == "__main__":
    if sys.argv[1:2] == ["--capture"]:
        capture(sys.argv[2:])
    else:
        main()
