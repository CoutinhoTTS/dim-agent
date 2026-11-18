#!/usr/bin/env python3
"""Build and train a native macOS executable, then reorder its functions.

An initial build needs a new output directory. Resume and measurement reuse
the existing directory. Cargo's executable is left unchanged; only
an independently smoke-tested relink is published as OUTPUT/dimcode.
"""

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

from layout import select, unique_functions, validate_sizes
import macho
from replay import file_hash as digest, load_baseline, measure_artifacts, original_inputs, preserve_cargo, run_link, save_baseline
from samples import read as read_sample
from toolchain import rustc as rustc_command
from train_darwin import measure, train

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}


def cargo_link(work, compiler, jobs, target=None):
    rustc = rustc_command()
    host = target or TARGETS[platform.machine()]
    sysroot = Path(subprocess.check_output([rustc, "--print", "sysroot"], cwd=REPO, text=True).strip())
    objcopy = sysroot / "lib/rustlib" / host / "bin/rust-objcopy"
    wrapper = work / "linker"
    rustc_wrapper = work / "rustc-wrapper"
    for path, mode in ((wrapper, "--capture"), (rustc_wrapper, "--rustc")):
        path.write_text(
            "#!/bin/sh\nexec "
            + shlex.join([sys.executable, str(HERE / "build_darwin.py"), mode])
            + ' "$@"\n'
        )
        path.chmod(0o700)
    env = dict(os.environ, RUSTC=rustc, RUSTUP_TOOLCHAIN=str(sysroot),
               DIM_LAYOUT_CAPTURE=str(work), DIM_LAYOUT_CC=compiler,
               DIM_LAYOUT_OBJCOPY=str(objcopy),
               RUSTC_WRAPPER=str(rustc_wrapper),
               DIM_LAYOUT_RUSTC_WRAPPER=os.environ.get("RUSTC_WRAPPER", ""))
    deployment = subprocess.check_output(
        [rustc, "--print", "deployment-target", "--target", host], cwd=REPO, text=True).strip()
    key, value = deployment.split("=", 1)
    if key != "MACOSX_DEPLOYMENT_TARGET":
        raise RuntimeError(f"unexpected deployment target: {deployment}")
    env[key] = value
    env["DYLD_LIBRARY_PATH"] = os.pathsep.join(filter(None, (
        str(sysroot / "lib"), env.get("DYLD_LIBRARY_PATH"))))
    env["DIM_LAYOUT_DYLD_LIBRARY_PATH"] = env["DYLD_LIBRARY_PATH"]
    subprocess.run([str(objcopy), "--version"], env=env, check=True)
    command = [
        str(sysroot / "bin/cargo"), "rustc", "--release", "--locked", "-p", "dimcode", "--bin", "dimcode",
        "--jobs", str(jobs), "--target", host,
    ]
    command.extend(["--", f"-Clinker={wrapper}"])
    subprocess.run(command, cwd=REPO, env=env, check=True)
    record = json.loads((work / "link.json").read_text())
    record["rustc"] = subprocess.check_output([rustc, "-vV"], cwd=REPO, text=True)
    return record


def compile_rust(arguments):
    strip = "none"
    for index, argument in enumerate(arguments):
        codegen = arguments[index + 1] if argument == "-C" else argument.removeprefix("-C")
        if (argument == "-C" or argument.startswith("-C")) and codegen.startswith("strip="):
            strip = codegen.removeprefix("strip=")
    if strip not in ("none", "debuginfo", "symbols"):
        raise RuntimeError(f"unsupported Rust strip mode: {strip}")
    command = arguments.copy()
    if os.environ.get("DIM_LAYOUT_RUSTC_WRAPPER"):
        command.insert(0, os.environ["DIM_LAYOUT_RUSTC_WRAPPER"])
    os.execvpe(command[0], command, dict(os.environ, DIM_LAYOUT_STRIP=strip,
        DYLD_LIBRARY_PATH=os.environ["DIM_LAYOUT_DYLD_LIBRARY_PATH"]))


def capture(arguments):
    root = Path(os.environ["DIM_LAYOUT_CAPTURE"])
    compiler = os.environ["DIM_LAYOUT_CC"]
    arguments = [*arguments, f"-Wl,-map,{root / 'baseline.map'}"]
    inputs = []
    output = None
    for index, argument in enumerate(arguments):
        if index and arguments[index - 1] == "-o":
            output = str(Path(argument).resolve())
        elif argument.startswith(("@", "-Wl,@")):
            raise RuntimeError("response files are not supported by the layout build")
        elif not argument.startswith("-") and Path(argument).is_file():
            target = root / f"{index}-{Path(argument).name}"
            shutil.copy2(argument, target)
            inputs.append({"file": str(target), "source": str(Path(argument).absolute()),
                           "sha256": digest(target)})
    if not output or (root / "link.json").exists():
        raise RuntimeError("expected exactly one final executable link")
    strip = os.environ["DIM_LAYOUT_STRIP"]
    objcopy = os.environ["DIM_LAYOUT_OBJCOPY"] if strip != "none" else None
    record = {"arguments": arguments, "cwd": os.getcwd(), "compiler": compiler,
              "output": output, "inputs": inputs, "path": os.environ["PATH"],
              "strip": strip, "objcopy": objcopy,
              "linkEnv": {key: os.environ.get(key) for key in (
                  "ZERO_AR_DATE", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH",
                  "SDKROOT", "MACOSX_DEPLOYMENT_TARGET"
              )}}
    record["linkEnv"]["DYLD_LIBRARY_PATH"] = os.environ["DIM_LAYOUT_DYLD_LIBRARY_PATH"]
    (root / "link.json").write_text(json.dumps(record, indent=2) + "\n")
    subprocess.run([compiler, *arguments], check=True, env=dict(os.environ,
        DYLD_LIBRARY_PATH=os.environ["DIM_LAYOUT_DYLD_LIBRARY_PATH"]))


def relink(record, output, order_file=None, map_path=None, strict=False):
    arguments = record["arguments"].copy()
    if output != Path(record["output"]):
        arguments[arguments.index("-o") + 1] = str(output)
    if order_file:
        arguments.append(f"-Wl,-order_file,{order_file}")
        arguments.append("-Wl,-order_file_statistics")
    if map_path:
        arguments = [argument for argument in arguments if not argument.startswith("-Wl,-map,")]
        arguments.append(f"-Wl,-map,{map_path}")
    if strict and not record.get("duplicateSystemWarning"):
        arguments.append("-Wl,-fatal_warnings")
    env = dict(os.environ, PATH=record["path"])
    for key, value in record["linkEnv"].items():
        if value is None:
            env.pop(key, None)
        else:
            env[key] = value
    with original_inputs(record["inputs"]):
        result = run_link([record["compiler"], *arguments], cwd=record["cwd"], env=env)
    if result.returncode:
        raise RuntimeError(f"native link failed ({result.returncode}):\n{result.stderr}")
    if record["strip"] != "none":
        flag = {"debuginfo": "--strip-debug", "symbols": "--strip-all"}[record["strip"]]
        stripped = run_link([record["objcopy"], flag, str(output)], cwd=record["cwd"], env=env)
        if stripped.returncode:
            raise RuntimeError(f"native stripping failed ({stripped.returncode}):\n{stripped.stderr}")
    diagnostics = result.stderr.splitlines()
    warning = record.get("duplicateSystemWarning")
    if order_file and warning in diagnostics:
        diagnostics.remove(warning)
    if strict and diagnostics:
        raise RuntimeError(f"unexpected strict native link diagnostic:\n{result.stderr}")
    return "\n".join(diagnostics)


def replay_baseline(record, work, output):
    cargo = Path(record["output"])
    backup = work / "cargo-original"
    baseline = output / "baseline"
    baseline_map = output / "baseline.map"
    shutil.copyfile(work / "baseline.map", baseline_map)
    with preserve_cargo([(cargo, backup)]):
        diagnostics = relink(record, cargo)
        for line in diagnostics.splitlines():
            if line in ("ld: warning: ignoring duplicate libraries: '-lSystem'",
                        "ld64: warning: ignoring duplicate libraries: '-lSystem'"):
                record["duplicateSystemWarning"] = line
        original_hash, replay_hash = digest(backup), digest(cargo)
        if replay_hash != original_hash:
            shutil.copy2(backup, output / "cargo-original")
            shutil.copy2(cargo, output / "baseline-replay")
            (output / "replay.json").write_text(json.dumps(record, indent=2) + "\n")
            raise RuntimeError(
                "captured inputs did not reproduce Cargo's executable: "
                f"original={original_hash}, replay={replay_hash}")
        shutil.copy2(cargo, baseline)
    return baseline, baseline_map


def ordered_link(record, output, order_file, map_path):
    diagnostics = relink(record, output, order_file, map_path, strict=False)
    names = order_file.read_text(encoding="utf-8", errors="surrogateescape").splitlines()
    omitted = set()
    for line in diagnostics.splitlines():
        match = re.fullmatch(
            r"(?:ld|ld64): warning: can't find (?:match for|function/data for) "
            r"order_file entry: (\S+)", line
        )
        if not match:
            match = re.fullmatch(
                r"(?:ld|ld64): warning: (\S+) specified in order_file but it exists in multiple "
                r"\.o files\. Prefix symbol with \.o filename in order_file to disambiguate", line)
        if match and match[1] in names:
            omitted.add(match[1])
            continue
        if re.fullmatch(
            r"(?:ld|ld64): warning: only \d+ out of \d+ order_file symbols were applicable",
            line,
        ):
            continue
        raise RuntimeError(f"unexpected native link diagnostic:\n{diagnostics}")
    if omitted:
        order_file.write_text("\n".join(name for name in names if name not in omitted) + "\n",
                              encoding="utf-8", errors="surrogateescape")
        relink(record, output, order_file, map_path, strict=True)
    return sorted(omitted)


def order(baseline, baseline_map, profiles, output):
    text_addr, text_size, _ = macho.text_range(baseline)
    functions = macho.functions_from_sizes(
        macho.map_symbols(baseline_map, text_addr, text_size))
    page_size, footprints = None, []
    for profile in profiles:
        observed, pages = read_sample(profile)
        if page_size is None:
            page_size = observed
        elif observed != page_size:
            raise ValueError("training samples use different page sizes")
        footprints.append(pages)
    names = select(unique_functions(functions), footprints, page_size)
    output.write_text("\n".join(names) + "\n", encoding="utf-8", errors="surrogateescape")
    return {"pages": [len(pages) for pages in footprints], "pageSize": page_size,
            "orderedFunctions": len(names), "functionSymbols": len(functions)}


def validate(baseline, candidate, baseline_map, candidate_map, order_file):
    text_addr, text_size, _ = macho.text_range(baseline)
    before = macho.map_symbols(baseline_map, text_addr, text_size)
    after_text = macho.text_range(candidate)
    after = macho.map_symbols(candidate_map, *after_text[:2])
    if macho.function_start_count(baseline) != macho.function_start_count(candidate):
        raise RuntimeError("relink changed the function-start table")
    order_names = order_file.read_text(encoding="utf-8", errors="surrogateescape").splitlines()
    measurement = validate_sizes(
        macho.functions_from_sizes(before), macho.functions_from_sizes(after),
        [size for _, size, _ in before], [size for _, size, _ in after],
        order_names, "Mach-O function")
    measurement["textSize"] = text_size
    measurement["candidateTextSize"] = after_text[1]
    return measurement


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", choices=sorted(TARGETS.values()))
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--pairs", type=int, default=2)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--resume", action="store_true", help="retry sorting a verified baseline in OUTPUT")
    mode.add_argument("--measure-only", action="store_true", help="measure the validated executables in OUTPUT")
    args = parser.parse_args()
    if sys.platform != "darwin" or platform.machine() not in TARGETS:
        parser.error("requires native macOS arm64 or x86-64")
    host = TARGETS[platform.machine()]
    if args.target and args.target != host:
        parser.error("only the native macOS architecture is supported")
    if args.jobs < 1:
        parser.error("jobs must be positive")
    if args.pairs < 1:
        parser.error("pairs must be positive")
    if os.environ.get("CARGO_BUILD_TARGET") not in (None, host):
        parser.error("cross compilation is not supported")
    page_size = os.sysconf("SC_PAGESIZE")
    if page_size not in (4096, 16384):
        parser.error("requires a 4 KiB or 16 KiB page size")
    output = args.output.resolve()
    if output.exists() and not (args.resume or args.measure_only):
        parser.error("the output directory must not exist")
    compiler = shutil.which("cc")
    if not compiler:
        parser.error("cc is required")
    with tempfile.TemporaryDirectory(prefix="dim-layout-", dir="/tmp") as temporary:
        work = Path(temporary)
        probe = work / "probe.dylib"
        subprocess.run(
            [compiler, "-dynamiclib", "-fPIC", "-O2", "-Wall", "-Wextra", "-Werror",
             str(HERE / "probe_darwin.c"), "-o", str(probe)],
            check=True)
        baseline = output / "baseline"
        executable = output / "dimcode"
        if args.measure_only:
            measure_artifacts(output, host, baseline, executable,
                              lambda: measure(baseline, executable, work / "measure", probe, args.pairs))
            return
        baseline_map = output / "baseline.map"
        if args.resume:
            if executable.exists():
                parser.error("the output already contains a validated executable")
            record = load_baseline(output, baseline, host)
        else:
            archive = output / "capture"
            archive.mkdir(parents=True)
            capture = Path(tempfile.mkdtemp(dir=archive))
            record = cargo_link(capture, compiler, args.jobs, host)
            replay_baseline(record, capture, output)
            save_baseline(output, baseline, record, host, REPO)
        profiles = train(baseline, work / "training", probe)
        for profile in profiles:
            shutil.copyfile(profile, output / profile.name)
        order_file = output / "symbols.order"
        measurement = order(baseline, baseline_map, profiles, order_file)
        candidate = output / "candidate"
        candidate_map = output / "candidate.map"
        measurement["unorderableSymbols"] = ordered_link(
            record, candidate, order_file, candidate_map)
        measurement["orderedFunctions"] = len(order_file.read_text(
            encoding="utf-8", errors="surrogateescape").splitlines())
        measurement.update(validate(
            baseline, candidate, baseline_map, candidate_map, order_file))
        train(candidate, work / "acceptance")
        manifest = {
            "platform": "macos", "target": host, "pageSize": page_size,
            "codeResidencySource": "mincore-backing-pages",
            "lto": record["lto"], "source": record["source"], "dirty": record["dirty"],
            "rustc": record["rustc"],
            "baselineSha256": digest(baseline),
            "candidateSha256": digest(candidate),
            "orderSha256": digest(order_file),
            "inputs": record["inputs"], "training": measurement,
        }
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        candidate.rename(output / "dimcode")
        print(f"Validated executable: {output / 'dimcode'}", flush=True)


if __name__ == "__main__":
    if sys.argv[1:2] == ["--capture"]:
        capture(sys.argv[2:])
    elif sys.argv[1:2] == ["--rustc"]:
        compile_rust(sys.argv[2:])
    else:
        main()
