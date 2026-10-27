# RuntimeManager

Status: design in progress ([#476](https://github.com/arcships/dim/issues/476)).

RuntimeManager is a process plugin that supplies the Node and Python rutis runtimes for plugin code. It downloads them into the home, verifies them against pinned hashes, keeps their versions and verification results in its own file, and starts each runtime once it is ready. TypeScript and Python plugin rows gate on their runtime's name and start by themselves when RuntimeManager provides it.

The sections below are written as the open questions in #476 are settled.
