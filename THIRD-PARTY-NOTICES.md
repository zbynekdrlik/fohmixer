# Third-party notices

fohmixer is licensed under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
This file carries the notices that third-party code built into the shipped
programs asks for. It ships in the Windows bundle next to `fohmixer-hub.exe`.

## jpeg-encoder (in `fohmixer-hub.exe`)

- Crate: [jpeg-encoder](https://crates.io/crates/jpeg-encoder) 0.7, used for
  the Pro-Q 4 screen's picture frames.
- Licence: `(MIT OR Apache-2.0) AND IJG`. Its forward DCT (`src/fdct.rs`,
  `src/avx2/fdct.rs`) is ported from mozjpeg and derives from the Independent
  JPEG Group's software, under the IJG licence.

This software is based in part on the work of the Independent JPEG Group.
