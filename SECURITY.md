# Security Policy

## Supported versions

Monitra is pre-1.0. Security fixes are made on the latest release and on `main`. Older
versions are not patched, so please upgrade before reporting an issue you found there.

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Report it privately through GitHub's advisory form:
<https://github.com/rustiqz/monitra/security/advisories/new>

Please include:

- The affected version (`monitra version`) and how you installed it.
- A description of the issue and its impact.
- Steps or a minimal proof of concept to reproduce it.
- Any suggested fix, if you have one.

## What to expect

- **Acknowledgement** within 3 working days.
- **An initial assessment** within 10 working days, including whether we consider it a
  vulnerability and a rough timeline.
- **Coordinated disclosure.** We will agree a disclosure date with you, publish an advisory
  with the fix, and credit you if you wish.

This is a volunteer-run project, so these are good-faith targets rather than guarantees.

## Scope

In scope: the `monitra` binary and its crates, the HTTP/WebSocket API, API-token and
agent-token handling, and the embedded web dashboard.

Out of scope: vulnerabilities in third-party services Monitra is configured to talk to, and
risks that come from exposing the daemon without following
[Deployment](docs/site/src/deployment.md), for example binding it to a public interface
without TLS or a reverse proxy.
