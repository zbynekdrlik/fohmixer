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

## libopus (in `fohmixer-hub.exe`)

- Crates: [opus](https://crates.io/crates/opus) 0.4 (`MIT OR Apache-2.0`) and
  [opusic-sys](https://crates.io/crates/opusic-sys) 0.7 (`BSD-3-Clause`), which
  builds libopus 1.6 from source; Listen's audio to the pages (#210).
- Licence: BSD-3-Clause, whose second clause asks a binary to reproduce its
  notice. libopus's `COPYING`:

```text
Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic,
                    Jean-Marc Valin, Timothy B. Terriberry,
                    CSIRO, Gregory Maxwell, Mark Borgerding,
                    Erik de Castro Lopo, Mozilla, Amazon

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of Internet Society, IETF or IETF Trust, nor the
names of specific contributors, may be used to endorse or promote
products derived from this software without specific prior written
permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
```
