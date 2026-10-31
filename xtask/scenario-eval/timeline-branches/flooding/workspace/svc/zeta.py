"""Service zeta."""

RECENT = []


def record(event):
    RECENT.append(event)
    return len(RECENT)


def latest():
    return RECENT[-1]
