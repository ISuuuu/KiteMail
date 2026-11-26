# Security policy

KiteMail reads untrusted content (every message it opens was written by someone
else), so security reports are welcome and taken seriously.

The [Windows security and performance audit (中文, 2026-11-26)](security-performance-audit-2026-11-26.html)
documents network destinations, credential handling, fixes, benchmarks, and remaining risks.

## Reporting a vulnerability

**Please report privately, not as a public issue.**

- **GitHub private vulnerability reporting:** <https://github.com/ISuuuu/KiteMail/security/advisories/new>
  (Security → Report a vulnerability). This keeps the report, the discussion
  and the eventual advisory in one place.

You should get a first reply within **5 days**. If you don't, open a public
issue saying only that you're trying to reach a maintainer privately, with no
details.

Useful things to include, none of them required: affected version, the file and
line if you have it, what an attacker gains, and anything that reproduces it
(a `.eml` file is ideal for reader bugs).

## Disclosure

Report privately, and there is no deadline being run against you. The intent is
to ship a fix, publish a GitHub Security Advisory, and credit you by whatever
name and link you'd like, or not at all, if you'd rather. If you want to write
the finding up, say so and a date can be agreed; **90 days** is a reasonable
default if nothing else is proposed.

Nothing here is meant to restrict what you may say about your own research.

## Supported versions

Fixes go into the next release from `main`. Older versions are not patched
separately. KiteMail ships for Windows x64 as an NSIS installer and a portable
ZIP archive via GitHub Releases.

## Scope

In scope, roughly in order of interest:

- Anything letting message content escape the WebView2 reader's sandbox, run
  script, or read other messages.
- Anything defeating remote-content blocking, external request header /
  `Referrer` stripping, link sanitization, or otherwise causing a network
  request the user didn't ask for.
- Credential handling: Windows Credential Manager storage, OAuth tokens, and
  what reaches disk.
- TLS: verification, downgrade, `STARTTLS` handling.
- In-app updater integrity: release verification, SHA-256 checksum validation,
  and installer/archive handling.

Out of scope: vulnerabilities in the user's own mail server, findings that need
an attacker who already has local code execution as the user, and reports
consisting only of scanner output.

## Notes on the design

Two things are deliberate and are not bugs:

- **Message bodies render with JavaScript disabled**, in a `sandbox`ed iframe
  (`allow-same-origin allow-popups`, without `allow-scripts`), under a CSP of
  `default-src 'none'; img-src ...; style-src 'unsafe-inline'; frame-src 'self'; base-uri 'none'; form-action 'none'`
  with `Referrer-Policy: no-referrer`. Only the outer wrapper document receives
  a trusted host-injected scroll-bridge script via WebView2 `ExecuteScript`;
  the mail frame itself stays scriptless.
- **Remote content is blocked by default** and the blocking follows the user's
  setting, not a guess about whether a message contains remote references. A
  detector miss should cost you the "blocked" banner, never the blocking.

If you find a case where either isn't true, that's exactly the kind of report
this file is for.
