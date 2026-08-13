# Security Policy

Please do not disclose a suspected vulnerability in a public GitHub Issue,
Pull Request, Discussion, commit, or screenshot. Do not include credentials,
private endpoints, raw captures, or exploit details in a public report.

## Supported versions

Security fixes are developed on `main` and are backported at the maintainers'
discretion. The latest non-draft release line is currently `v0.4.x`.

| Version | Security support |
| --- | --- |
| `main` | Supported; fixes land here first |
| Latest non-draft release (`v0.4.x`) | Best-effort fixes when practical |
| Older releases | No routine security backports |

Update this table when a new non-draft release line becomes the supported
baseline.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting form when the repository displays
**Report a vulnerability** under **Security → Advisories**. This keeps the
technical report private while maintainers triage it. Private vulnerability
reporting is a repository setting and is separate from this file; if the form
is not available, do not disclose the details publicly. Open an Issue containing
only a request for a private security contact, or use a private contact channel
already provided to you by the maintainers.

Include the following information through a private channel, when available:

- affected version, commit, package, route, or feature;
- impact and realistic attack preconditions;
- a minimal reproduction or proof of concept that does not include real
  credentials, private addresses, or experimental data;
- any known workaround and the conditions under which it is effective.

Do not assume that a successful build, static check, or local hardware test
proves that a security boundary is safe. The repository's public development
and validation rules are described in `AGENTS.md` and
`docs/project/requirements.md`.

## Coordinated disclosure

Maintainers will make a best-effort acknowledgement and may request more
information, a safer reproduction, or coordination on a release and disclosure
date. We do not promise a response time or a backport for every supported
surface. Please do not publish vulnerability details until a fix or disclosure
plan has been agreed privately.
