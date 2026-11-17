"""Read the function symbols and code range of a thin 64-bit Mach-O executable."""

import os
import re
import struct

_MAP_SYMBOL = re.compile(r"^0x([0-9a-fA-F]+)\t0x([0-9a-fA-F]+)\t\[\s*\d+\]\s+(.*)$")

MH_MAGIC_64 = 0xFEEDFACF
LC_SEGMENT_64 = 0x19
LC_SYMTAB = 0x2
LC_FUNCTION_STARTS = 0x26
SECTION_TYPE = 0xFF
S_ZEROFILL = 0x1
N_STAB = 0xE0
N_TYPE = 0x0E
N_SECT = 0x0E


def _load_commands(data):
    magic, _, _, _, commands, _, _ = struct.unpack_from("<IiiIIII", data, 0)
    if magic != MH_MAGIC_64:
        raise ValueError("expected a thin little-endian 64-bit Mach-O")
    at = 32
    for _ in range(commands):
        command, size = struct.unpack_from("<II", data, at)
        if size < 8 or at + size > len(data):
            raise ValueError("malformed Mach-O load command")
        yield command, at, size
        at += size


def _cstring(data, at, size):
    end = data.index(b"\0", at, at + size)
    return data[at:end].decode()


def text_range(binary):
    """Return ``(addr, size, file_offset)`` of ``__TEXT,__text``."""
    data = binary.read_bytes()
    found = None
    for command, at, size in _load_commands(data):
        if command != LC_SEGMENT_64:
            continue
        segments = _cstring(data, at + 8, 16)
        fileoff, filesize = struct.unpack_from("<QQ", data, at + 40)
        sections = struct.unpack_from("<I", data, at + 64)[0]
        for index in range(sections):
            section = at + 72 + index * 80
            name = _cstring(data, section, 16)
            segment = _cstring(data, section + 16, 16)
            addr, section_size, offset = struct.unpack_from("<QQI", data, section + 32)
            if segments == "__TEXT" and segment == "__TEXT" and name == "__text":
                if found:
                    raise ValueError("the executable has more than one __text section")
                found = (addr, section_size, offset)
    if not found:
        raise ValueError("Mach-O executable has no __TEXT,__text section")
    return found


def function_start_count(binary):
    """Return the number of entries in ``LC_FUNCTION_STARTS``, or 0 if absent."""
    data = binary.read_bytes()
    for command, at, _ in _load_commands(data):
        if command != LC_FUNCTION_STARTS:
            continue
        offset, size = struct.unpack_from("<II", data, at + 8)
        payload = data[offset:offset + size]
        count = 0
        value = 0
        at = 0
        while at < len(payload):
            byte = payload[at]
            at += 1
            value += byte & 0x7F
            if byte & 0x80:
                shift = 7
                while at < len(payload):
                    byte = payload[at]
                    at += 1
                    value += (byte & 0x7F) << shift
                    shift += 7
                    if not byte & 0x80:
                        break
            if value == 0:
                break
            count += 1
        return count
    return 0


def _page_size():
    return os.sysconf("SC_PAGESIZE")


def map_symbols(map_path, text_addr, text_size):
    """Return linker-reported ``[(start, size, name)]`` relative to the text base.

    ld64's map records the exact atom size, so parity does not have to infer a
    function's extent from the next symbol, which would include alignment
    padding that moves when functions are reordered.
    """
    base = text_addr - text_addr % _page_size()
    end = text_addr + text_size
    entries = []
    in_symbols = False
    for line in map_path.read_text().splitlines():
        if line.startswith("# Symbols:"):
            in_symbols = True
            continue
        if line.startswith("# Address"):
            continue
        if line.startswith("# Dead"):
            in_symbols = False
            continue
        if line.startswith("#"):
            in_symbols = False
            continue
        if not in_symbols:
            continue
        match = _MAP_SYMBOL.fullmatch(line)
        if not match:
            continue
        address, size = int(match[1], 16), int(match[2], 16)
        if text_addr <= address < end:
            entries.append((address - base, size, match[3]))
    if not entries:
        raise ValueError("the ld64 map has no __text symbols")
    return sorted(entries)


def functions_from_sizes(entries):
    """Convert ``[(start, size, name)]`` into ``[(start, end, name)]``."""
    return [(start, start + size, name) for start, size, name in entries]
