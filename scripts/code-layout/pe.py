"""Read a PE32+ image and the symbols lld-link records in its ``/lldmap`` file."""

import re
import struct

IMAGE_FILE_MACHINE_AMD64 = 0x8664
IMAGE_FILE_MACHINE_ARM64 = 0xAA64
MACHINES = {
    IMAGE_FILE_MACHINE_AMD64: ("x86_64", 8),
    IMAGE_FILE_MACHINE_ARM64: ("aarch64", 8),
}

_SECTION = re.compile(r"^([0-9a-fA-F]+) ([0-9a-fA-F]+)\s+\d+\s+(\.[A-Za-z0-9_$]+)\s*$")
_CHUNK = re.compile(r"^([0-9a-fA-F]+) ([0-9a-fA-F]+)\s+\d+\s+(.*)$")


def sections(binary):
    """Return ``{name: (virtual_address, virtual_size, raw_offset, raw_size)}``."""
    data = binary.read_bytes()
    if data[:2] != b"MZ":
        raise ValueError("expected a PE image")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("expected a PE signature")
    machine, count = struct.unpack_from("<HH", data, pe + 4)
    if machine not in MACHINES:
        raise ValueError(f"unsupported PE machine 0x{machine:x}")
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    table = pe + 24 + optional_size
    result = {}
    for index in range(count):
        at = table + index * 40
        name = data[at:at + 8].rstrip(b"\0").decode()
        virtual_size, virtual_address, raw_size, raw_offset = struct.unpack_from("<IIII", data, at + 8)
        result[name] = (virtual_address, virtual_size, raw_offset, raw_size)
    return result


def text_range(binary):
    """Return ``(rva, size)`` of ``.text``."""
    section = sections(binary).get(".text")
    if not section:
        raise ValueError("PE image has no .text section")
    return section[0], section[1]


def symbols(map_path):
    """Return ``(functions, sizes)`` from an lld-link map.

    ``functions`` are ``(start, end, name)`` relative to the ``.text`` RVA;
    ``sizes`` carries the COMDAT body size when the map attributes exactly one
    symbol to a chunk. A chunk holding several symbols gets ``None`` and is
    compared by name only.
    """
    text_start = text_size = None
    section = None
    entries = []
    chunk_size = None
    chunk_symbols = []

    def finish():
        nonlocal chunk_symbols
        for address, name in chunk_symbols:
            entries.append((address, name, chunk_size if len(chunk_symbols) == 1 else None))
        chunk_symbols = []

    for line in map_path.read_text().splitlines():
        header = _SECTION.fullmatch(line)
        if header:
            if section == ".text":
                finish()
            section = header[3]
            if section == ".text":
                text_start, text_size = int(header[1], 16), int(header[2], 16)
            chunk_size = None
            continue
        match = _CHUNK.fullmatch(line)
        if not match:
            continue
        address = int(match[1], 16)
        size = int(match[2], 16)
        rest = match[3].strip()
        if ":" in rest:
            if section == ".text":
                finish()
            chunk_size = size
        elif section == ".text" and rest:
            chunk_symbols.append((address, rest))
    if section == ".text":
        finish()
    if text_start is None or text_size is None:
        raise ValueError("the lld map has no .text section")
    entries = sorted(
        entry for entry in entries if text_start <= entry[0] < text_start + text_size
    )
    if not entries:
        raise ValueError("the lld map has no .text function symbols")
    starts = sorted(set(address for address, _, _ in entries))
    end_by_start = {address: text_start + text_size for address in starts}
    for current, following in zip(starts, starts[1:]):
        end_by_start[current] = following
    functions = [
        (address - text_start, end_by_start[address] - text_start, name)
        for address, name, _ in entries
    ]
    return functions, [size for _, _, size in entries]
