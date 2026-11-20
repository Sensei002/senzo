# SENZO licensing

SENZO is licensed under the **GNU General Public License, version 3.0 only**. The SPDX identifier is `GPL-3.0-only`. The root `LICENSE` file is authoritative.

## Nuvio source relationship

The audited Nuvio Desktop repository is also licensed under GNU GPL version 3.0. SENZO's root license text is byte-identical to the upstream GPLv3 license text. Where SENZO adapts or derives code from Nuvio, preserve applicable upstream copyright and license notices, provide the required license text and corresponding source when distributing covered binaries, and comply with GPLv3 obligations.

Do not copy Nuvio logos, artwork, product identity, or unrelated third-party assets. Feature behavior and architecture can be reimplemented in Rust, but source-derived code must be handled under the applicable license terms. This policy is an engineering summary, not legal advice.

## Dependency notices

Every dependency and bundled runtime must be reviewed before distribution. Include required license texts, attribution and source offers/notices as required by each license. The current repository includes an Apache-2.0 notice for `librqbit`; the MPV runtime is distributed under its applicable LGPL terms. These dependency licenses do not replace SENZO's GPLv3 license.

## What GPL-3.0-only means

- Use, study, modification and redistribution are permitted under GPLv3 terms.
- Distribution of covered modified versions requires compliance with GPLv3 source, notice and same-license requirements.
- Commercial distribution is permitted when license obligations are followed.
- GPLv3 does not impose the AGPL network-interaction source-disclosure condition.
- The license does not grant rights to third-party media, metadata, trademarks, services or independently installed add-ons.

## Contributor policy

Contributions intended for inclusion in SENZO are accepted under GPL-3.0-only, subject to applicable copyright ownership and contribution policies. No separate Contributor License Agreement is currently adopted.

## Legal note

This document is an explanatory summary, not legal advice. Consult the root `LICENSE` and the [official GPLv3 text](https://www.gnu.org/licenses/gpl-3.0.html).


## Bundled Windows media runtime

The Windows development bundle includes the x86_64 LGPL libmpv runtime from the upstream [mpv v0.42.0 release](https://github.com/mpv-player/mpv/releases/tag/v0.42.0). The build downloads the upstream `libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip` asset and verifies SHA-256 `8e5f77430efc53ef175804ba6618527e3af0448a0dcabf04d3cc72659d1a6fce` before packaging. The corresponding source is [mpv v0.42.0](https://github.com/mpv-player/mpv/tree/v0.42.0). The app bundle retains license/copyright files present in the runtime archive under `third-party-licenses/mpv`; consult those notices for the exact terms of each included component.
