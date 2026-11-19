"""Read the ``.text`` input sections lld records in its ``--Map`` file."""

# A 64-bit map writes VMA, LMA, Size, Align, Out, In and Symbol in fixed
# columns: four numeric fields separated by one space, then the name column.
_COLUMNS = 16 + 1 + 16 + 1 + 8 + 1 + 5 + 1


def input_sections(map_path):
    """Return ``[[chunk, [symbol, ...]], ...]`` for the map's ``.text`` chunks.

    ``chunk`` is the map's rendering of one input section. Functions listed
    under the same chunk are not independently orderable: lld ranks an input
    section by the best rank among the symbols it names and keeps the section's
    own order inside it.
    """
    chunks, current, in_text = [], None, False
    for line in map_path.read_text(encoding="utf-8", errors="surrogateescape").splitlines():
        if len(line) <= _COLUMNS:
            continue
        tail = line[_COLUMNS:]
        indent = len(tail) - len(tail.lstrip(" "))
        name = tail[indent:]
        if indent == 0:
            in_text = name.startswith(".text")
            current = None
        elif indent == 8 and in_text:
            current = []
            chunks.append([name, current])
        elif indent == 16 and current is not None:
            current.append(name)
        else:
            current = None
    if not chunks:
        raise ValueError("the lld map has no .text input sections")
    return chunks
