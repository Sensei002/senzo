# SENZO licensing policy and decision record

## Current status: undecided

No project license has been selected or granted. The Cargo package is currently marked `UNLICENSED`. Until a license text is committed, all rights are reserved by the copyright holder. A public GitHub repository is not automatically open source and does not grant reuse, modification, or redistribution rights.

The project owner has expressed a preference for a highly restrictive license intended to reduce misuse. The exact restrictions must be clarified before choosing and publishing a license.

## Important distinction: open source vs. source available

An OSI-approved open-source license cannot prohibit use by a particular person or group, or prohibit use in a field of endeavor. It also cannot impose a general non-commercial-only restriction. Those restrictions conflict with the Open Source Definition.

A strong copyleft license can require source-code sharing under specified conditions, but it does not generally prohibit otherwise lawful use. For example, GNU AGPL-3.0 adds source-offer obligations for modified versions users interact with over a network; it does not create a general "good use only" rule.

A source-available license can impose additional limits, such as non-commercial use or restrictions on redistribution/modification. That may better match a misuse-prevention goal, but it is not an OSI-approved open-source license.

## Candidate directions

These are options to evaluate, not a selected license:

### GNU AGPL-3.0

- Strong copyleft for a recognized free/open-source license.
- Requires source availability under its terms when distributing covered works and has additional obligations for modified network-interactive versions.
- Does not prohibit commercial use or restrict users based on a field of activity.
- Best fit if the priority is reciprocal sharing and keeping derivative deployments open, not preventing all misuse.

Official text: https://www.gnu.org/licenses/agpl-3.0.en.html

### PolyForm Noncommercial 1.0.0

- Source-available, not OSI-approved open source.
- Allows specified non-commercial use, modification, and distribution under its terms.
- Restricts commercial use, with the license's definitions and exceptions controlling.
- Better fit if the primary goal is to prevent commercial exploitation while allowing community use.

Official text: https://polyformproject.org/licenses/noncommercial/1.0.0/

### PolyForm Strict 1.0.0

- Highly restrictive source-available option.
- Permits non-commercial use under its terms but does not grant general rights to modify or redistribute copies.
- Likely too restrictive if the goal is to build a contributor community or permit forks.

Official text: https://polyformproject.org/licenses/strict/1.0.0/

### Custom ethical-use restriction

- Could prohibit specifically described harmful uses.
- Such a restriction would make the project source-available rather than OSI-approved open source.
- A vague phrase such as "bad use" is not precise enough for a legal license. The prohibited conduct, definitions, enforcement, exceptions, and governing legal framework would need careful drafting and legal review.
- A custom license may create compatibility and enforcement uncertainty.

## Questions that determine the right license

Before adding a LICENSE file, decide:

1. **What does "bad use" mean?** Commercial resale/repackaging, closed-source forks, piracy/copyright infringement, malicious activity, surveillance, harmful content, or another specific category?
2. **Should people be allowed to modify and redistribute SENZO?** This determines whether a strict license that withholds those permissions is acceptable.
3. **Should commercial use be prohibited?** If yes, the project will be source-available, not OSI-approved open source.
4. **Should community contributions and forks be allowed?** If yes, the license must clearly grant those permissions and state how derivatives are licensed.

## Third-party components and media services

SENZO depends on third-party crates and plans integrations with GPUI, MPV, TMDB, Discord, and external add-ons. The project license cannot override their licenses or terms. Before release:

- Audit direct and transitive dependency licenses.
- Check redistribution requirements for MPV and any bundled runtime libraries.
- Keep third-party notices and attributions.
- Respect TMDB API terms and attribution requirements.
- Do not imply that a project license grants rights to copyrighted films, television, provider streams, or third-party services.

## Decision rule

Do not add a guessed license merely to populate GitHub's license badge. Once the project owner chooses the intended permissions and restrictions, use the exact official license text where applicable, update Cargo metadata, add third-party notices as needed, and review the result before the first public release.

This document is informational, not legal advice. Obtain qualified legal advice before publishing a custom or restrictive license.
