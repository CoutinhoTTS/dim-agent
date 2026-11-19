#!/usr/bin/env python3
"""Build and train a native executable, then reorder its functions.

Use a new output directory for the initial build. On macOS and Windows,
--resume and --measure-only reuse an existing build directory.
"""

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile

from elf import input_sections
from layout import ranks, unique_functions
from toolchain import rustc as rustc_command

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cargo_link(work, compiler, jobs, target=None):
    rustc = rustc_command()
    sysroot = Path(subprocess.check_output([rustc, "--print", "sysroot"], cwd=REPO, text=True).strip())
    linker_dir = sysroot / "lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld"
    if not (linker_dir / "ld.lld").is_file():
        raise RuntimeError(f"the selected Rust toolchain has no LLD at {linker_dir / 'ld.lld'}")
    wrapper = work / "linker"
    wrapper.write_text("#!/bin/sh\nexec " + shlex.join([sys.executable, str(HERE / "build.py"), "--capture"]) + ' "$@"\n')
    wrapper.chmod(0o700)
    env = dict(os.environ, RUSTC=rustc, DIM_LAYOUT_CAPTURE=str(work), DIM_LAYOUT_CC=compiler)
    command = ["cargo", "rustc", "--release", "--locked", "-p", "dimcode", "--bin", "dimcode", "--jobs", str(jobs)]
    if target:
        command.extend(["--target", target])
    command.extend(["--", f"-Clinker={wrapper}", f"-Clink-arg=-B{linker_dir}", "-Clink-arg=-fuse-ld=lld"])
    subprocess.run(command, cwd=REPO, env=env, check=True)
    record = json.loads((work / "link.json").read_text())
    record["rustc"] = subprocess.check_output([rustc, "-vV"], cwd=REPO, text=True)
    return record


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


def symbols(binary):
    data = binary.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected an unstripped ELF64 little-endian executable")
    header = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    if header[2] != 62:
        raise ValueError("only x86-64 is supported")
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, header[6] + i * header[11])
                for i in range(header[12])]
    names_section = sections[header[13]]
    names = data[names_section[4]:names_section[4] + names_section[5]]
    section_names = [names[s[0]:names.index(b"\0", s[0])].decode() for s in sections]
    text_index = section_names.index(".text")
    code = sections[text_index]
    table = sections[section_names.index(".symtab")]
    strings_section = sections[table[6]]
    strings = data[strings_section[4]:strings_section[4] + strings_section[5]]
    functions = []
    for at in range(table[4], table[4] + table[5], table[9]):
        name, info, _, section, value, size = struct.unpack_from("<IBBHQQ", data, at)
        name = strings[name:strings.index(b"\0", name)].decode()
        if info & 15 == 2 and section == text_index and size:
            offset = value - code[3] + code[4]
            functions.append((offset, offset + size, name))
    if not functions:
        raise ValueError("no function symbols; do not strip the training binary")
    return sorted(functions)


def footprint(path, binary):
    pages, current = set(), None
    for line in path.read_text().splitlines():
        match = re.match(r"^([0-9a-f]+)-([0-9a-f]+) (\S+) ([0-9a-f]+) \S+ \S+\s*(.*)$", line)
        if match:
            start, end, perm, offset, name = match.groups()
            current = (int(start, 16), int(end, 16), int(offset, 16)) if perm == "r-xp" and name == str(binary) else None
        elif current and line.startswith("Referenced:") and int(line.split()[1]):
            start, end, offset = current
            if end - start != 4096:
                raise ValueError("the training executable must have per-page mappings")
            pages.add(offset // 4096)
    if not pages:
        raise ValueError(f"empty executable footprint: {path}")
    return sorted(pages)


def order(binary, map_path, profiles, output):
    """Write the order file for ``binary``, keeping each ``.text`` chunk together.

    lld ranks an input section by the best rank of the symbols it names and keeps
    the section's own order inside it, so functions sharing a section cannot be
    placed independently. Each chunk is emitted contiguously in address order at
    the position of its best-ranked member, which is where one link puts it. A
    name the map attributes to more than one chunk cannot be placed at all and
    is left out.
    """
    functions = symbols(binary)
    pages = [footprint(profile, binary) for profile in profiles]
    chunks_of = {}
    for chunk, symbols_in_chunk in input_sections(map_path):
        for name in symbols_in_chunk:
            chunks_of.setdefault(name, set()).add(chunk)
    ranked = [entry for entry in ranks(unique_functions(functions), pages, 4096)
              if len(chunks_of.get(entry[2], ())) < 2]
    if not ranked:
        raise ValueError("every selected function was ambiguous between .text chunks")
    frames = {name: index for index, (_, _, name) in enumerate(functions)}
    groups = {}
    for priority, _, name in ranked:
        chunks = chunks_of.get(name, ())
        key = ("chunk", next(iter(chunks))) if len(chunks) == 1 else ("symbol", name)
        groups.setdefault(key, []).append((priority, name))
    ordered = [name for members in groups.values()
               for _, name in sorted(members, key=lambda member: frames[member[1]])]
    output.write_text("\n".join(ordered) + "\n")
    return {"pages": [len(p) for p in pages], "orderedFunctions": len(ordered),
            "sectionCoupledFunctions": sum(1 for members in groups.values() if len(members) > 1),
            "functionSymbols": len(functions)}


def relink(record, output, order_file=None, strict=True, map_path=None):
    arguments = record["arguments"].copy()
    arguments[arguments.index("-o") + 1] = str(output)
    if order_file:
        arguments.append(f"-Wl,--symbol-ordering-file={order_file}")
    if map_path:
        arguments.extend([f"-Wl,-Map={map_path}", "-Wl,--no-demangle"])
    if strict:
        arguments.append("-Wl,--fatal-warnings")
    env = dict(os.environ, PATH=record["path"])
    result = subprocess.run([record["compiler"], *arguments], cwd=record["cwd"], env=env,
                            capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"native link failed ({result.returncode}):\n{result.stderr}")
    return result.stderr


def ordered_link(record, output, order_file):
    """Link ``output`` with ``order_file`` and return the names it was linked with.

    lld reports names it cannot place; those are removed and the link repeats, so
    the returned names are the ones the candidate was actually laid out by.
    """
    diagnostics = relink(record, output, order_file, strict=False)
    names = order_file.read_text().splitlines()
    omitted = set()
    for line in diagnostics.splitlines():
        match = re.fullmatch(r"(?:rust-lld|ld\.lld): warning: .*: unable to order (?:shared|discarded) symbol: (\S+)", line)
        if not match or match[1] not in names:
            raise RuntimeError(f"unexpected native link diagnostic:\n{diagnostics}")
        omitted.add(match[1])
    if omitted:
        names = [name for name in names if name not in omitted]
        order_file.write_text("\n".join(names) + "\n")
        relink(record, output, order_file)
    return names, sorted(omitted)


def validate_layout(baseline, candidate, order_names):
    """Check ``candidate`` follows ``order_names``, the order it was linked with."""
    before, after = symbols(baseline), symbols(candidate)
    if Counter((name, end - start) for start, end, name in before) != Counter((name, end - start) for start, end, name in after):
        raise RuntimeError("relink changed the function set or function sizes")
    ranks = {name: rank for rank, name in enumerate(order_names)}
    if not ranks or set(ranks) - {name for _, _, name in after}:
        raise RuntimeError("the final order must name existing function symbols")
    positions, labels = {}, {}
    for start, _, name in after:
        if name in ranks and ranks[name] < positions.get(start, len(ranks)):
            positions[start], labels[start] = ranks[name], name
    priorities = [(start, rank) for start, rank in sorted(positions.items())]
    for (start, rank), (previous_start, previous_rank) in zip(priorities[1:], priorities):
        if rank < previous_rank:
            raise RuntimeError(
                "candidate function addresses do not follow the final symbol order: "
                f"{labels[start]} (rank {rank}) at {start:#x} follows "
                f"{labels[previous_start]} (rank {previous_rank}) at {previous_start:#x}")
    selected_before = Counter((name, start, end - start) for start, end, name in before if name in ranks)
    selected_after = Counter((name, start, end - start) for start, end, name in after if name in ranks)
    moved = sum((selected_before - selected_after).values())
    if not moved:
        raise RuntimeError("the symbol order moved no selected function addresses")
    return {"orderedFunctionSymbols": sum(selected_after.values()), "movedFunctionSymbols": moved}


def main():
    if sys.platform == "darwin":
        from build_darwin import main as platform_main
        return platform_main()
    if sys.platform == "win32":
        from build_windows import main as platform_main
        return platform_main()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", choices=["x86_64-unknown-linux-gnu"])
    parser.add_argument("--jobs", type=int, default=2)
    args = parser.parse_args()
    if sys.platform != "linux" or platform.machine() != "x86_64" or os.sysconf("SC_PAGESIZE") != 4096:
        parser.error("requires native Linux x86-64 with 4 KiB pages")
    if args.jobs < 1:
        parser.error("jobs must be positive")
    if os.environ.get("CARGO_BUILD_TARGET") not in (None, "x86_64-unknown-linux-gnu"):
        parser.error("cross compilation is not supported")
    from train import train

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    compiler = shutil.which("cc")
    if not compiler:
        parser.error("cc is required")
    with tempfile.TemporaryDirectory(prefix="dim-layout-") as temporary:
        work = Path(temporary)
        record = cargo_link(work, compiler, args.jobs, args.target)
        baseline = output / "baseline"
        baseline_map = output / "baseline.map"
        relink(record, baseline, map_path=baseline_map)
        if digest(baseline) != digest(Path(record["output"])):
            raise RuntimeError("captured inputs did not reproduce Cargo's executable")
        probe = work / "probe.so"
        subprocess.run([compiler, "-shared", "-fPIC", "-O2", "-Wall", "-Wextra", "-Werror",
                        str(HERE / "probe.c"), "-ldl", "-o", str(probe)], check=True)
        profiles = train(baseline, work / "training", probe)
        for profile in profiles:
            shutil.copyfile(profile, output / profile.name)
        order_file = output / "symbols.order"
        measurement = order(baseline, baseline_map, profiles, order_file)
        candidate = output / "candidate"
        order_names, omitted = ordered_link(record, candidate, order_file)
        measurement["unorderableSymbols"] = omitted
        measurement["orderedFunctions"] = len(order_names)
        measurement.update(validate_layout(baseline, candidate, order_names))
        train(candidate, work / "acceptance")
        manifest = {"source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
                    "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO)),
                    "rustc": record["rustc"],
                    "baselineSha256": digest(baseline), "candidateSha256": digest(candidate),
                    "orderSha256": digest(order_file), "inputs": record["inputs"],
                    "training": measurement}
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        candidate.rename(output / "dimcode")
        print(f"Validated executable: {output / 'dimcode'}", flush=True)


if __name__ == "__main__":
    if sys.argv[1:2] == ["--capture"]:
        capture(sys.argv[2:])
    else:
        main()
