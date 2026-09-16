# Now Plexing

Watch your Plex Media Server's active playback sessions from the COSMIC panel.

The panel shows a symbolic icon and how many streams are playing right now.
Clicking it lists each session — who is watching, what they are watching, how
far in they are, and the elapsed and total time.

## Setup

Open the popup, press the gear, paste a [Plex token][plex-token], and save.

That is the only setting. The server itself is discovered from the token using
Plex's resource API, preferring a local connection over a remote one and
treating the Plex relay as a last resort. If the account owns more than one
server, the online one is chosen, with alphabetical order breaking ties.

### Where the token is kept

The token is stored with the rest of the COSMIC configuration, in
`~/.config/cosmic/com.github.gnarr.now-plexing/v1/token`. COSMIC writes its
configuration world-readable, so Now Plexing narrows that file to `0600` and its
directory to `0700` after every write. It is still a plaintext file on disk —
moving it into the Secret Service is the obvious next step.

The token is never written to a log, never placed in a URL, and is redacted from
every `Debug` rendering in the codebase.

## Installation

A [justfile](./justfile) is included by default for the [casey/just][just] command runner.

- `just` builds the application with the default `just build-release` recipe
- `just run` builds and runs the application
- `just install` installs the project into the system
- `just vendor` creates a vendored tarball
- `just build-vendored` compiles with vendored dependencies from that tarball
- `just check` runs clippy on the project to check for linter warnings
- `just check-json` can be used by IDEs that support LSP

## Translators

[Fluent][fluent] is used for localization of the software. Fluent's translation files are found in the [i18n directory](./i18n). New translations may copy the [English (en) localization](./i18n/en) of the project, rename `en` to the desired [ISO 639-1 language code][iso-codes], and then translations can be provided for each [message identifier][fluent-guide]. If no translation is necessary, the message may be omitted.

## Packaging

If packaging for a Linux distribution, vendor dependencies locally with the `vendor` rule, and build with the vendored sources using the `build-vendored` rule. When installing files, use the `rootdir` and `prefix` variables to change installation paths.

```sh
just vendor
just build-vendored
just rootdir=debian/now_plexing prefix=/usr install
```

It is recommended to build a source tarball with the vendored dependencies, which can typically be done by running `just vendor` on the host system before it enters the build environment.

## Developers

Developers should install [rustup][rustup] and configure their editor to use [rust-analyzer][rust-analyzer].

[fluent]: https://projectfluent.org/
[fluent-guide]: https://projectfluent.org/fluent/guide/hello.html
[iso-codes]: https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes
[just]: https://github.com/casey/just
[plex-token]: https://support.plex.tv/articles/204059436-finding-an-authentication-token-x-plex-token/
[rustup]: https://rustup.rs/
[rust-analyzer]: https://rust-analyzer.github.io/
