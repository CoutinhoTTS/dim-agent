"""Platform-neutral selection of functions from observed executable pages."""

import bisect
from collections import Counter

# A function used by all three flows is placed before one used by two, and a
# startup/shutdown function before one used only by the TUI or the conversation.
PRIORITY = {mask: index for index, mask in enumerate([7, 3, 5, 1, 6, 2, 4])}


def ranks(functions, pages, page_size):
    """Return ``(priority, start, name)`` for functions touching an observed page."""
    ranked = []
    for start, end, name in functions:
        mask = sum(
            1 << index
            for index, used in enumerate(pages)
            if bisect.bisect_left(used, start // page_size)
            < bisect.bisect_left(used, (end + page_size - 1) // page_size)
        )
        if mask:
            ranked.append((PRIORITY[mask], start, name))
    if not ranked:
        raise ValueError("training selected no linkable functions")
    return sorted(ranked)


def select(functions, pages, page_size):
    """Return ordered unique names whose extent intersects an observed page.

    ``functions`` are ``(start, end, name)`` with addresses relative to the
    page-aligned code base; ``pages`` are sorted page indices in that base.
    Original address order is kept within each usage group.
    """
    ranked = ranks(functions, pages, page_size)
    return list(dict.fromkeys(name for _, _, name in ranked))


def unique_functions(functions):
    """Drop names repeated at different addresses, which an order file cannot disambiguate."""
    starts = {}
    for start, _, name in functions:
        starts.setdefault(name, set()).add(start)
    return [function for function in functions if len(starts[function[2]]) == 1]


def validate_sizes(functions_before, functions_after, sizes_before, sizes_after,
                   order_names, label="function"):
    """Check name/count parity and every reliable linker-reported body size.

    ``sizes_before`` and ``sizes_after`` carry one ``(name, size_or_None)`` per
    function, aligned with the function list. ``None`` means the linker map
    attributes the chunk to several symbols, so its size is genuinely ambiguous
    for that symbol. A name with such an entry on either side is compared by
    name and count only, and the number of skipped entries is recorded; every
    name with known sizes on both sides is compared exactly.
    """
    if len(sizes_before) != len(functions_before) or len(sizes_after) != len(functions_after):
        raise RuntimeError(f"missing {label} sizes from the linker map")
    before_by_name, after_by_name = {}, {}
    for (_, _, name), size in zip(functions_before, sizes_before):
        before_by_name.setdefault(name, []).append(size)
    for (_, _, name), size in zip(functions_after, sizes_after):
        after_by_name.setdefault(name, []).append(size)
    before_names = Counter(name for _, _, name in functions_before)
    after_names = Counter(name for _, _, name in functions_after)
    if before_names != after_names:
        before_only = list((before_names - after_names).items())[:20]
        after_only = list((after_names - before_names).items())[:20]
        raise RuntimeError(
            f"relink changed the {label} names or counts: "
            f"before-only {before_only}; after-only {after_only}")

    ambiguous = 0
    mismatches = []
    for name, before in before_by_name.items():
        after = after_by_name[name]
        if None in before or None in after:
            ambiguous += len(before)
            continue
        if Counter(before) != Counter(after):
            mismatches.append((name, sorted(before), sorted(after)))
    if mismatches:
        raise RuntimeError(
            f"relink changed {label} sizes for {len(mismatches)} symbol(s): "
            f"{mismatches[:20]}")
    measurement = _order(functions_before, functions_after, order_names, label)
    measurement["ambiguousFunctionSymbols"] = ambiguous
    return measurement


def _order(functions_before, functions_after, order_names, label):
    names = set(order_names)
    if not names or len(names) != len(order_names):
        raise RuntimeError("the final order must contain unique symbol names")
    if names - {name for _, _, name in functions_after}:
        raise RuntimeError("the final order must name existing function symbols")

    positions = {}
    for start, _, name in functions_after:
        if name in names:
            if name in positions and positions[name] != start:
                raise RuntimeError(f"order file names an ambiguous symbol: {name}")
            positions[name] = start
    requested_addresses = [positions[name] for name in order_names]
    if requested_addresses != sorted(requested_addresses):
        raise RuntimeError("candidate function addresses do not follow the final symbol order")

    baseline_starts = {name: start for start, _, name in functions_before if name in names}
    candidate_starts = {name: start for start, _, name in functions_after if name in names}
    moved = sum(
        1
        for name in names
        if baseline_starts.get(name) != candidate_starts.get(name)
    )
    if not moved:
        raise RuntimeError("the symbol order moved no selected function addresses")
    return {"orderedFunctionSymbols": len(positions), "movedFunctionSymbols": moved}
