# Precog CLI

> IDE-style terminal autocomplete for zsh, bash and fish — a floating dropdown that suggests subcommands, flags and arguments as you type.

This project is a community fork of [`aws/amazon-q-developer-cli-autocomplete`](https://github.com/aws/amazon-q-developer-cli-autocomplete) (originally [`withfig/autocomplete`](https://github.com/withfig/autocomplete)), licensed under MIT (originally MIT OR Apache-2.0).
It is **not affiliated with, endorsed by, or sponsored by Amazon Web Services, Inc.**

## What Precog is

Precog is the *autocomplete* part of the upstream project, isolated and rebranded:

- **figterm** — a headless pseudo-terminal that wraps your real shell and watches the edit buffer keystroke-by-keystroke
- **fig_desktop** — a small native app (Tao + Wry webview) that draws a floating dropdown overlay positioned over your cursor
- **autocomplete** — the React UI rendered inside that overlay; it consumes the `@withfig/autocomplete-specs` format to know what subcommands, flags, and arguments each binary accepts

The AI chat / agent / inline-suggestion features that lived in the upstream `q_cli` are **not** part of this fork's goals — those depend on AWS-hosted services and AWS authentication. The fork keeps just the local autocomplete dropdown.

## Status

Early fork. Renaming and decoupling from AWS is in progress. The upstream code builds end-to-end on macOS and Linux; expect breakage as AWS-specific bits are pulled out.

## Project layout

| Path | Purpose |
|------|---------|
| `packages/autocomplete/` | React app rendered in the dropdown overlay |
| `packages/autocomplete-parser/` | Parses Fig autocomplete specs |
| `packages/autocomplete-app/` | Webview shell |
| `crates/figterm/` | PTY wrapper that intercepts shell input |
| `crates/fig_desktop/` | Native overlay app (windowing via `tao`/`wry`) |
| `crates/fig_input_method/` | macOS input method, used to read cursor position |
| `crates/fig_*` | Supporting Rust crates: IPC, settings, telemetry, integrations |
| `extensions/vscode`, `extensions/jetbrains` | Editor integrations (kept upstream-as-is for now) |
| `proto/` | Protobuf IPC schema between the components |

## Local development

Prerequisites:

- macOS (Xcode 13+) or Linux with the build deps below
- Rust toolchain (`rustup`)
- Node 22+, `pnpm`, `mise` for tool versioning
- `protoc`

On Debian/Ubuntu:

```sh
sudo apt update
sudo apt install build-essential pkg-config jq dpkg curl wget cmake clang \
  libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  libdbus-1-dev libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  valac libibus-1.0-dev libglib2.0-dev sqlite3 libxdo-dev protobuf-compiler
```

Build the workspace:

```sh
mise install
pnpm install --ignore-scripts
cargo build
```

## License

This project is distributed under the **MIT license** (see `LICENSE.MIT`). The original copyright by Amazon.com, Inc. is retained as required by the MIT license; modifications by Precog contributors are copyright their respective authors.

The `LICENSE.APACHE` file is preserved so downstream users may also rely on the Apache-2.0 grant if they prefer it (the upstream was dual-licensed).

## Trademark notice

"Amazon", "Amazon Web Services", "AWS", "Amazon Q" and "CodeWhisperer" are trademarks of Amazon.com, Inc. or its affiliates. This project is not affiliated with, endorsed by, or sponsored by AWS. References in source code (e.g., crate names like `amzn-codewhisperer-client`) are nominative — they describe the AWS service the original SDK client talks to — and do not imply endorsement.

## Acknowledgements

Built on the work of the original Fig and AWS Amazon Q CLI teams. See `git log` for full attribution.
