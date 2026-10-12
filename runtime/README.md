# Experimental macOS runtime

`macos.patch` is the complete public integration diff against
[RyanTheTechMan/soloist](https://github.com/RyanTheTechMan/soloist) commit
`01a8b80f83570643fefe623775d8ee9285c07d25`. It includes the native elfuse
patch targeting `4023647491db64adc43de76284206d8ab81ff6e3`.
It contains no official Spotify executable, account state or credentials.

Use Apple Silicon, macOS 27, Xcode Command Line Tools and ARM64 Homebrew.
Build the frontend from this checkout first:

```sh
cargo build --locked --no-default-features -j 1
```

Then build the separate runtime checkout:

```sh
git clone https://github.com/RyanTheTechMan/soloist.git soloist-macos-runtime
cd soloist-macos-runtime
git checkout 01a8b80f83570643fefe623775d8ee9285c07d25
git apply /absolute/path/to/spotifast/runtime/macos.patch
python3 -m venv .venv
.venv/bin/pip install -r packaging/requirements.txt
.venv/bin/python scripts/setup.py --build-only --install-deps
.venv/bin/python scripts/build-package.py \
  --name 'Spotifast Soloist' \
  --frontend /absolute/path/to/spotifast/target/debug/spotifast \
  --output-dir /private/tmp/spotifast-preview
.venv/bin/python scripts/verify-package.py \
  '/private/tmp/spotifast-preview/Spotifast Soloist.app'
```

The setup downloads checksum-locked Debian ARM64 libraries and builds elfuse
with one job. The resulting app uses local ad-hoc signatures and is not
notarized. These instructions produce a personal debug test build; they do
not establish byte-for-byte reproducibility, release performance or
redistribution clearance. The official Soloist executable must be obtained
separately from Spotify and selected locally. Enter a personal Soloist key
only in the native secure setup field.

The raw control listener is remapped to an owner-only Unix socket, with no
TCP fallback. The compatibility runtime still has the invoking user's host
file and network authority. It is not a security sandbox. Output-only audio
uses explicit CoreAudio device modules; restart the receiver after adding
an output device. Actual authenticated lossless output needs live
qualification. See [integration and limitations](../docs/SOLOIST_MACOS.md).
