# Local patches

Each change to vendored source is its own commit, with the reason in the
message, so it can be compared against upstream at `SOURCE_REV`. Modified files
carry a `// Modified by dim-agent:` line at the top.

- `xai-grok-markdown/build.rs`: precompile the existing two-face syntax set without
  the extra `Swift.sublime-syntax` asset. The asset is omitted because its
  redistribution license has not been verified. Swift uses the existing syntax
  set's lookup/fallback behavior.
- `xai-grok-markdown/src/syntax.rs`: drop the Swift special case (`patched_swift`)
  and the tests of the patched grammar's string and interpolation scopes, which
  left with the asset; two-face's Swift is found like any other syntax.
