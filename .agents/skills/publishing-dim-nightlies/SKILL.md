---
name: publishing-dim-nightlies
description: Prepares, publishes and verifies Dim nightly CLI releases for the curl channel. Use for nightly tag pushes, manual Actions runs or retention checks, not npm alpha releases.
---

# Publish a Dim nightly

Read the [nightly release contract](../../../docs/release/README.md#nightly-curl-channel). Before creating a tag, check the target `origin/main` tree for `.github/workflows/nightly.yml` and `scripts/install-nightly.sh`. If either is absent, report [#2131](https://github.com/arcships/dim/issues/2131) as a prerequisite rather than using the alpha workflow.

Fetch `origin/main` and read remote tags with `git ls-remote --tags --refs origin 'refs/tags/v*-nightly.*'`. Use the `X.Y.Z` core from that commit's workspace version unless the user specifies another. Within that core, compare exact nightly suffixes numerically, including tags whose assets have expired. Start at `.1` or increment the largest `N`. A preparation request reports the proposed tag and source SHA without triggering a run.

For an authorized publication, choose one entry point:

- By default, or to pin an exact source commit, push only the new tag at the selected main SHA.
- For a requested manual run, first check that `origin/main` has `workflow_dispatch` in `.github/workflows/nightly.yml`. In **Publish curl nightly → Run workflow**, select `main` and enter the unused tag. The run creates the tag at the dispatch commit; do not push it yourself. Verify the resulting tag's source before reporting the release.

If the tag is taken, inspect its source and run; never move or reuse it. On failure, report the failed job and run URL without rerunning it or triggering a replacement build. Select the next number on the next release request.

On success, inspect the rolling `Nightly` release, not a release named after the build tag. Compare its title and asset sets to the published versions and retention rule across all version cores. Report the tag, source, release URL and verified platforms. Report mismatches rather than deleting releases by hand. No nightly step publishes to npm.
