"""A bounded LRU cache with per-entry TTL.

`Cache(capacity, ttl)` keeps at most `capacity` entries. `get(key)` returns the
value or returns None for a missing or expired entry; an expired entry is
dropped. `put(key, value)` inserts or updates and evicts the least recently
used entry when over capacity. `get` and `put` both count as a use.

The clock is injected so the tests can move time: `Cache(capacity, ttl, now)`
where `now` is a zero-argument callable returning a float of seconds.

TODO(open): get, put, eviction, expiry.
"""


class Cache:
    def __init__(self, capacity, ttl, now=None):
        raise NotImplementedError("Cache.__init__")

    def get(self, key):
        raise NotImplementedError("Cache.get")

    def put(self, key, value):
        raise NotImplementedError("Cache.put")

    def __len__(self):
        raise NotImplementedError("Cache.__len__")
