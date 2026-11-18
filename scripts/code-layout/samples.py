"""Read and write the page-residency sample shared by the native probes."""


def read(path):
    lines = path.read_text().splitlines()
    if not lines or not lines[0].startswith("pagesize "):
        raise ValueError(f"invalid page sample: {path}")
    page_size = int(lines[0].split()[1])
    if page_size not in (4096, 16384):
        raise ValueError(f"unexpected page size: {page_size}")
    offsets = [int(line) for line in lines[1:] if line]
    if not offsets:
        raise ValueError(f"empty executable footprint: {path}")
    return page_size, sorted(set(offset // page_size for offset in offsets))


def write(path, page_size, pages):
    lines = [f"pagesize {page_size}"]
    lines.extend(str(page * page_size) for page in sorted(set(pages)))
    path.write_text("\n".join(lines) + "\n")
