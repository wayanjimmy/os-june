# OS Notetaker

macOS-first Tauri MVP for local notes, microphone-only recording, saved audio validation, batch transcription, and generated notes.

## Development

```sh
pnpm install
pnpm tauri:dev
```

The app data directory is resolved by Tauri at runtime. In development, inspect the platform app data path for:

- `notes.sqlite3`
- `recordings/{note_id}/{session_id}.wav`

## macOS Microphone Debugging

The macOS bundle includes:

- `NSMicrophoneUsageDescription` in `src-tauri/Info.plist`
- `com.apple.security.device.audio-input` in `src-tauri/Entitlements.plist`

Local `pnpm tauri:build` output is ad-hoc signed unless a signing identity is configured. Before distribution, verify the signed bundle embeds the expected entitlements:

```sh
codesign -dvvv --entitlements :- "src-tauri/target/release/bundle/macos/OS Notetaker.app"
```

If permission is denied during local testing, reset it from macOS Privacy & Security settings or with:

```sh
tccutil reset Microphone network.opensoftware.os-notetaker
```

## Verification

```sh
pnpm lint
pnpm test
pnpm test:rust
pnpm build
pnpm tauri:build
```

Manual recording reliability checks are tracked in `specs/001-tauri-note-mvp/manual-validation.md`.
