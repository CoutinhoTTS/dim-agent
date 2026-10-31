# bigwork

Two independent subsystems, each with its own directory and its own failing
test suite:

- `parser/` — integer expression parser
- `cache/` — bounded LRU cache with TTL

They share no code. Run everything with `python3 -m unittest discover -s tests`.
