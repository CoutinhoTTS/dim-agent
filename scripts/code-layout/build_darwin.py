#!/usr/bin/env python3
"""Build and train a native macOS executable, then reorder its functions.

The output directory must not exist. Cargo's executable is left unchanged; only
an independently smoke-tested relink is published as OUTPUT/dimcode.
"""

import argparse
import hashlib
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
from samples import read as read_sample
from train_darwin import measure, train

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cargo_link(work, compiler, jobs, target=None):
    wrapper = work / "linker"
    wrapper.write_text(
        "#!/bin/sh\nexec "
        + shlex.join([sys.executable, str(HERE / "build_darwin.py"), "--capture"])
        + ' "$@"\n'
    )
    wrapper.chmod(0o700)
    env = dict(os.environ, DIM_LAYOUT_CAPTURE=str(work), DIM_LAYOUT_CC=compiler)
    command = [
        "cargo", "rustc", "--release", "--locked", "-p", "dimcode", "--bin", "dimcode",
        "--jobs", str(jobs),
    ]
    if target:
        command.extend(["--target", target])
    command.extend(["--", f"-Clinker={wrapper}"])
    subprocess.run(command, cwd=REPO, env=env, check=True)
    return json.loads((work / "link.json").read_text())


def capture(arguments):
    root = Path(os.environ["DIM_LAYOUT_CAPTURE"])
    compiler = os.environ["DIM_LAYOUT_CC"]
    saved = []
    inputs = []
    output = None
    for index, argument in enumerate(arguments):
        if index and arguments[index - 1] == "-o":
            output = str(Path(argument).resolve())
            saved.append(output)
        elif argument.startswith(("@", "-Wl,@")):
            raise RuntimeError("response files are not supported by the layout build")
        elif not argument.startswith("-") and Path(argument).is_file():
            target = root / f"{index}-{Path(argument).name}"
            shutil.copyfile(argument, target)
            saved.append(str(target))
            inputs.append({"file": target.name, "sha256": digest(target)})
        else:
            saved.append(argument)
    if not output or (root / "link.json").exists():
        raise RuntimeError("expected exactly one final executable link")
    record = {"arguments": saved, "cwd": os.getcwd(), "compiler": compiler,
              "output": output, "inputs": inputs, "path": os.environ["PATH"]}
    (root / "link.json").write_text(json.dumps(record, indent=2) + "\n")
    subprocess.run([compiler, *arguments], check=True)


def relink(record, output, order_file=None, map_path=None, strict=False):
    arguments = record["arguments"].copy()
    arguments[arguments.index("-o") + 1] = str(output)
    if order_file:
        arguments.append(f"-Wl,-order_file,{order_file}")
        arguments.append("-Wl,-order_file_statistics")
    if map_path:
        arguments.append(f"-Wl,-map,{map_path}")
    if strict:
        arguments.append("-Wl,-fatal_warnings")
    env = dict(os.environ, PATH=record["path"])
    result = subprocess.run([record["compiler"], *arguments], cwd=record["cwd"], env=env,
                            capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"native link failed ({result.returncode}):\n{result.stderr}")
    return result.stderr


def ordered_link(record, output, order_file, map_path):
    diagnostics = relink(record, output, order_file, map_path, strict=False)
    names = order_file.read_text().splitlines()
    omitted = set()
    for line in diagnostics.splitlines():
        match = re.fullmatch(
            r"(?:ld|ld64): warning: can't find (?:match for|function/data for) "
            r"order_file entry: (\S+)", line
        )
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
        order_file.write_text("\n".join(name for name in names if name not in omitted) + "\n")
        relink(record, output, order_file, map_path, strict=True)
    return sorted(omitted)


def order(baseline, profiles, output):
    functions = macho.symbols(baseline)
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


def validate(baseline, candidate, baseline_map, candidate_map, order_file):
    text_addr, text_size, _ = macho.text_range(baseline)
    before = macho.map_symbols(baseline_map, text_addr, text_size)
    after_text = macho.text_range(candidate)
    after = macho.map_symbols(candidate_map, *after_text[:2])
    if macho.function_start_count(baseline) != macho.function_start_count(candidate):
        raise RuntimeError("relink changed the function-start table")
    order_names = order_file.read_text().splitlines()
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
    if output.exists():
        parser.error("the output directory must not exist")
    compiler = shutil.which("cc")
    if not compiler:
        parser.error("cc is required")
    with tempfile.TemporaryDirectory(prefix="dim-layout-", dir="/tmp") as temporary:
        work = Path(temporary)
        record = cargo_link(work, compiler, args.jobs, args.target or host)
        baseline = output / "baseline"
        output.mkdir(parents=True)
        relink(record, baseline)
        cargo = Path(record["output"])
        if digest(baseline) != digest(cargo):
            raise RuntimeError("captured inputs did not reproduce Cargo's executable")
        baseline_map = output / "baseline.map"
        relink(record, baseline, map_path=baseline_map)
        if digest(baseline) != digest(cargo):
            raise RuntimeError("a diagnostic map changed the baseline executable")
        probe = work / "probe.dylib"
        subprocess.run(
            [compiler, "-dynamiclib", "-fPIC", "-O2", "-Wall", "-Wextra", "-Werror",
             str(HERE / "probe_darwin.c"), "-o", str(probe)],
            check=True)
        profiles = train(baseline, work / "training", probe)
        for profile in profiles:
            shutil.copyfile(profile, output / profile.name)
        order_file = output / "symbols.order"
        measurement = order(baseline, profiles, order_file)
        candidate = output / "candidate"
        candidate_map = output / "candidate.map"
        measurement["unorderableSymbols"] = ordered_link(
            record, candidate, order_file, candidate_map)
        measurement["orderedFunctions"] = len(order_file.read_text().splitlines())
        measurement.update(validate(
            baseline, candidate, baseline_map, candidate_map, order_file))
        train(candidate, work / "acceptance")
        measurements = measure(baseline, candidate, work / "measure", probe, args.pairs)
        manifest = {
            "platform": "macos", "target": host, "pageSize": page_size,
            "lto": os.environ.get("CARGO_PROFILE_RELEASE_LTO", "profile"),
            "source": subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
            "dirty": bool(subprocess.check_output(
                ["git", "status", "--porcelain"], cwd=REPO)),
            "rustc": subprocess.check_output(["rustc", "-vV"], text=True),
            "baselineSha256": digest(baseline),
            "candidateSha256": digest(candidate),
            "orderSha256": digest(order_file),
            "inputs": record["inputs"], "training": measurement,
            "measurements": measurements,
        }
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        candidate.rename(output / "dimcode")
        print(f"Validated executable: {output / 'dimcode'}", flush=True)


if __name__ == "__main__":
    if sys.argv[1:2] == ["--capture"]:
        capture(sys.argv[2:])
    else:
        main()
