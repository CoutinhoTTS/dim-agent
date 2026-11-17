"""Read a PE32+ image and the symbols lld-link records in its ``/lldmap`` file."""

import re
import struct

IMAGE_FILE_MACHINE_AMD64 = 0x8664
IMAGE_FILE_MACHINE_ARM64 = 0xAA64
MACHINES = {
    IMAGE_FILE_MACHINE_AMD64: ("x86_64", 8),
    IMAGE_FILE_MACHINE_ARM64: ("aarch64", 8),
}

_ENTRY = re.compile(r"^([0-9a-fA-F]+) ([0-9a-fA-F]+)\s+\d+( +)(\S.*)$")


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
    compared by name only. Labels at a chunk's exclusive end have no body and
    are excluded regardless of where the chunk is placed in ``.text``.
    """
    text_start = text_size = None
    section = None
    entries = []
    chunk_start = chunk_size = None
    chunk_symbols = []

    def finish():
        nonlocal chunk_symbols
        if chunk_symbols and (chunk_start is None or chunk_size is None):
            raise ValueError("the lld map has symbols without an input chunk")
        if any(address < chunk_start or address > chunk_start + chunk_size
               for address, _ in chunk_symbols):
            raise ValueError("the lld map has symbols outside their input chunk")
        chunk_symbols = [(address, name) for address, name in chunk_symbols
                         if chunk_start <= address < chunk_start + chunk_size]
        for address, name in chunk_symbols:
            entries.append((address, name, chunk_size if len(chunk_symbols) == 1 else None,
                            chunk_start + chunk_size))
        chunk_symbols = []

    for line in map_path.read_text(encoding="utf-8").splitlines():
        match = _ENTRY.fullmatch(line)
        if not match:
            continue
        address, size = int(match[1], 16), int(match[2], 16)
        indent, name = len(match[3]), match[4]
        if indent == 1:
            if section == ".text":
                finish()
            section = name
            if section == ".text":
                text_start, text_size = address, size
            chunk_start = chunk_size = None
            continue
        if indent == 9:
            if section == ".text":
                finish()
            chunk_start, chunk_size = address, size
        elif indent == 17 and section == ".text":
            chunk_symbols.append((address, name))
        elif section == ".text":
            raise ValueError(f"unexpected lld map column: {line}")
    if section == ".text":
        finish()
    if text_start is None or text_size is None:
        raise ValueError("the lld map has no .text section")
    entries = sorted(
        entry for entry in entries if text_start <= entry[0] < text_start + text_size
    )
    if not entries:
        raise ValueError("the lld map has no .text function symbols")
    starts = sorted(set(address for address, _, _, _ in entries))
    end_by_start = {address: text_start + text_size for address in starts}
    for current, following in zip(starts, starts[1:]):
        end_by_start[current] = following
    functions = [
        (address - text_start, min(end_by_start[address], chunk_end) - text_start, name)
        for address, name, _, chunk_end in entries
    ]
    return functions, [size for _, _, size, _ in entries]
