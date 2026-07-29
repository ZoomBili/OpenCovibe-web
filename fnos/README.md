# fnOS FPK

This directory contains the native fnOS package template. It does not use
Docker and does not bundle Claude Code or Codex.

Build the package from existing Linux release binaries:

```powershell
npm run fnos:package -- --version v0.2.7
```

The builder looks for the two binaries in `src-tauri/target` first, then reads
them from matching archives under `dist-release`. The output is written to
`dist-fnos/opencovibe-web_v0.2.7.fpk` with a matching `.sha256` file.

Install the `.fpk` from fnOS App Center's manual installation page. During
installation, set a browser token, workspace, CLI login HOME, and optional
absolute Claude Code/Codex paths. At least one existing and authenticated CLI
is required.

The local release publisher builds this FPK after the Linux `amd64` and
`arm64` binaries, then uploads the package and its SHA256 file to the matching
GitHub Release together with the Linux archives.
