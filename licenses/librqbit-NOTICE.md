# Third-party notices

## librqbit 9.1.0

SENZO embeds the `librqbit` Rust BitTorrent engine (version 9.1.0) as a library. It is statically linked into the SENZO application; it is not installed as a separate service or background process.

- Upstream project: https://github.com/ikatson/rqbit
- Version: 9.1.0
- License: Apache License 2.0
- Copyright: Copyright 2021 Igor Katson
- SENZO build configuration: default features disabled; Rust TLS and `disable-upload` enabled. Upload/seeding support is compiled out.

The full Apache License 2.0 text is included in [Apache-2.0.txt](Apache-2.0.txt). Upstream license: https://github.com/ikatson/rqbit/blob/v9.1.0/LICENSE.

This notice covers the direct `librqbit` dependency. Other direct and transitive dependencies retain their respective licenses and notices; the final release process must include the complete dependency inventory.
