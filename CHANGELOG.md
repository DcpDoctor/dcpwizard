# Changelog

## [Unreleased]

### Added
- **KDM distribution** — cinema/screen database (`cinema add|list|remove|add-screen|remove-screen|search|import-flm`, no private keys stored), named validity templates (`kdm-template`, used via `kdm/kdm-batch --template`), generation history (`kdm-history`, metadata-only JSONL), FLM-x / SMPTE 430-7 import (`cinema import-flm`), email delivery (`kdm/kdm-batch --email-to --smtp-config`, one zipped email per cinema), and `kdm-batch --cinema/--screen/--cert-dir` recipient resolution. `cert-fetch --vendor --serial` downloads recipient certs from the credential-free public endpoints (dolby/doremi, qube)
- **`decrypt` subcommand** — turn an encrypted DCP into a cleartext DCP of the same reel structure. Content keys come from a KDM + recipient RSA key (`--kdm --recipient-key`) or the DCP's `KEYS.json` (`--keys`); a wrong key or an uncovered KeyId fails loud. Key material stays in memory only
- **`assemble` subcommand** — build one new OV composition (single CPL) whose reels are the input DCPs' reels in program order; essence is copied byte-identical and referenced by its existing UUIDs. Inputs must share standard/rate/resolution and must not be encrypted
- **`edit` subcommand** — rewrite a DCP's CPL metadata (title/annotation/content-kind/issuer) without re-wrapping essence, assigning a new CPL id and refreshing PKL/ASSETMAP atomically. Refuses encrypted DCPs
- **`create-multi` subcommand** — multi-composition package: one CPL per manifest entry, each with its own picture/sound/subtitle, over one shared PKL/ASSETMAP (contrast `create --versions`, several CPLs over shared essence)
- **Subtitle VF** — `create-vf --replace-subtitle`/`--add-subtitle REEL=PATH` (with `--subtitle-language`) wraps an SRT or SMPTE XML into a per-reel timed-text MXF; a subtitle-only VF references the OV picture/sound by id and ships only the new subtitle MXF
- **Explicit reel splits** — `create --split-at <tc>[,...]` (HH:MM:SS or HH:MM:SS:FF) and `create/pipeline --split-chapters` (source chapter marks via ffprobe) feed the same multi-reel path as `--reel-length`, rejecting any reel shorter than 1 second
- **Custom container** — `create --container <2k-scope|...>` and `create --container-dims WxH` (even, within the 2K/4K bound; threads into CPL ScreenAspectRatio and MainPicture stored/active area)
- **Head/tail padding** — `create --pad-head <dur> --pad-tail <dur>` (`48f`/`2s`) with sample-accurate silence; `--pad-color <RRGGBB>` fills pad frames with an sRGB colour (routed through the DCI X'Y'Z' transform) instead of black. Rejected loud when combined with reel-length/3D/Atmos or head-padding a supplied SMPTE subtitle XML
- **Input decode range** — `create/pipeline --input-range full|legal` forces the source's decode range, correcting wrong or absent range flags (re-decodes into a lossless FFV1 intermediate)
- **Disk writer** — `format-drive <target> --fs ext2|ext3 [--label] --yes [--image]` (refuses mounted targets) and `check-drive <target>` (report fs type + label, works unprivileged on image files)
- **Sign-language video** — `create --sign-language-video <file> --sign-language-lang <rfc5646>` (ISDCF Doc 13): VP9-packed into 288 KB PCM blocks on sound channel 15, labeled SLVS via MCA, with the ISDCF SignLanguageVideo ExtensionMetadata in the CPL
- **Create-time audio processing** — `create --upmix a|b` (stereo→5.1), `--loudness-target leqm=<db>|lufs=<v>` with `--true-peak-ceiling`, and filename channel auto-routing (a directory of mono `name_L.wav`/`_R`/... files). Standalone `crossfade --a --b -o --overlap` (equal-power) and `mid-side-decode -i -o --mid --side`
- **Encode QoL on `create`** — `--start-at HH:MM|RFC3339|+offset` (scheduled wall-clock start), `--resume` (reuse on-disk J2K frames after an interruption, survives restarts), average-fps ETA in progress output, `--shutdown-when-done` (power off after a clean encode)
- **Subtitle input formats + authoring** — `create --subtitle` accepts `.ass`/`.ssa`, `.pac`, `.mks`/`.mkv`, `.fcpxml` and Interop `DCSubtitle` XML with PNG bitmap subs (styling and per-cue alignment carried into the DCST). Placement (`--subtitle-halign/--subtitle-valign/--subtitle-vposition`), 3D depth (`--subtitle-zposition`), RTL (`--subtitle-rtl auto|on|off`), auto-wrap (`--subtitle-wrap <chars>`) and font embedding + subsetting (`--subtitle-font`, `--subtitle-no-subset`, cross-reel font dedup)
- **`subtitle-edit` subcommand** — edit a standalone subtitle file (any parsable format): `--list`, `--shift-ms`, or `--index N` with `--text`/`--set-start-ms`+`--set-end-ms`, written back as SRT
- **HDR source delivery** — `create --hdr-to-dci-lut <lut>` runs the LUT before J2K encode; `--allow-generic-hdr-tonemap` opts into FFmpeg tone mapping with a warning. APV source files decode through ffmpeg when the local build has the decoder
- **`subtitle-extract` subcommand** — export timed text from a DCP (or a bare subtitle asset) back to `.srt` (timing preserved) or `.txt` (text only), format chosen by output extension. Reads MXF-wrapped ST 428-7 resources via asdcplib and loose SMPTE/Interop XML, resolves subtitle assets through the CPL/ASSETMAP, and concatenates reels with their timeline offsets applied. GUI extraction panel added
- **Destination free-space check (DoM 3150)** — `create` and `copy` now fail early with a clear message when the required bytes (essence for create, source total for copy) exceed the free space on the destination filesystem, instead of running out mid-write. Cross-platform via `statvfs` (Unix) and `GetDiskFreeSpaceExW` (Windows)
- **CompositionMetadataAsset (Bv2.1)** — SMPTE CPLs now carry a ST 429-16 `CompositionMetadataAsset` in the first reel with `MainSoundConfiguration` (e.g. `51/L,R,C,LFE,Ls,Rs` with `-` padding for the silent fill channels), `MainSoundSampleRate`, `MainPictureStoredArea`/`ActiveArea`, `FullContentTitleText` and the Bv2.1 `ExtensionMetadata` marker. The sound configuration is derived from the packaged channel count, so validators no longer flag "0 channels but sound assets have N". Interop CPLs are unchanged. Verified: the asset validates against the ST 429-16 XSD and the whole CPL against ST 429-7 via xmllint
- **`ingest-package` subcommand** — metadata-only repackaging: scans a directory, reads each MXF's embedded asset UUID (asdcplib), and regenerates ASSETMAP/PKL/VOLINDEX covering every asset file present, reusing hashes from the old PKL where available. Fixes exported OV/VF folders (e.g. a Sony server's VF export) whose ASSETMAP/PKL omit hardlinked assets referenced by the CPL. Essence passes through untouched; the old ASSETMAP/PKL are replaced. Both SMPTE and Interop naming
- **Configurable subtitle position** — `subtitle-convert --vposition <percent>` (default 8) sets the bottom line's distance from the bottom of the screen
- **Stereoscopic 3D** — `create --right-eye <input>` (main input is the left eye) encodes both eyes at the same settings, wraps them into one ST 429-10 stereoscopic picture MXF (postkit `wrap_stereoscopic`) and emits the CPL `MainStereoscopicPicture` element (429-10/2008 namespace, EditRate at the composition rate, FrameRate doubled for interleaved L/R). Eye frame counts must match. Verified: a short 3D DCP passes `dcpdoctor validate` with 0 errors and the stereoscopic element validates against the clairmeta 429-10 XSD via xmllint
- **MCA channel labeling** — every PCM sound wrap now carries ST 429-12 / 377-4 MCA labels derived from the probed channel count (2.0 / 5.1 / 7.1). `create --hi-channel <n>` / `--vi-channel <n>` label a channel index as the standalone HI / VI-N accessibility track. Verified by reading the labels back out of the MXF with asdcplib (`mca_labels`)
- **Dolby Atmos aux track** — `create --atmos <file-or-dir>` wraps a Dolby Atmos / DCData bitstream (postkit `EssenceType::Atmos`) and registers it as a ST 429-18 `AuxData` element (Dolby `2012/AD` namespace, IAB data-essence UL) in the CPL/PKL/ASSETMAP. Verified: an Atmos DCP with a synthetic payload passes `dcpdoctor validate`; real-essence conformance needs real Atmos material
- **Interop KDM** — `kdm --format smpte|interop` (default smpte) and `kdm-batch --format`. Interop uses postkit's digicine ETM (134-byte key block). Verified: an Interop KDM carries the digicine namespace and verifies with xmlsec1. Legacy output, mark for real-gear validation before production use
- **`transcode-dcp` subcommand** — re-encode an existing DCP's picture essence to a lower bandwidth (`--video-bit-rate`, optional `--width`/`--height` rescale). Extracts each J2K frame from the picture MXF (asdcplib), decodes with grk_decompress, re-encodes with grok at the target ratio, rewraps, and emits a fresh CPL/PKL/ASSETMAP with correct hashes. Audio and subtitle tracks are copied unchanged; encrypted input is rejected
- **Reel splitting**: `create --reel-length <minutes>` splits the picture, audio (sample-accurate at the frame edge) and subtitles (DCST re-split and rebased per reel) into one MXF set per reel and emits a multi-reel CPL over a shared PKL/ASSETMAP; every reel is at least 1 second, a sub-second remainder merges into the previous reel. Absent the flag the single-reel path is unchanged
- **HFR validation**: legal DCP rates are 24/25/30/48/50/60/96/100/120 (SMPTE; Interop stays 24/25/30/48); 4K is capped at 30 fps. Illegal fps/resolution combos are rejected loudly before encoding; the GUI frame-rate menus offer exactly the legal set
- **Subtitle packaging** — `create --subtitle <srt> [--subtitle-language]` converts SRT to ST 428-7 DCST XML, wraps it as a timed-text MXF and registers the track in the reel/CPL/PKL/ASSETMAP
- **`export` subcommand** — ProRes/H.264/H.265/DNxHR/image-sequence export of a picture MXF (previously only reachable via the batch daemon)
- **`transcode --format` / `--bit-depth`** — wired the GUI transcode controls to the real backend
- **`watch --webhook-url`** — POST a JSON notification when a new DCP is detected

### Changed
- **Encode bandwidth** — the GUI now maps the target Mbps to the J2K compression ratio (was hardcoded)
- **`colour --target xyz`** — routes through the real DCDM Rec.709/P3/Rec.2020 → X'Y'Z' transform (fails loud on an unsupported source)
- **PCM wrap** — rejects non-DCP audio sample rates (48/96 kHz only) instead of mislabeling
- **KDM validity** — duration-based end times keep the start's UTC offset
- **GUI resolution** — scope/flat/full map to the correct container dimensions; content kind `episode` maps to a real ContentKind
- **Subtitle timecodes** — SMPTE DCST now emits frame-based `HH:MM:SS:FF` (schema-valid), not `HH:MM:SS.mmm`

### Removed
- **DTS:X** — was silently mapped to the Atmos (IAB) essence UL, which is wrong for DTS:X. Removed the `MxfType::DtsX` variant and dropped DTS:X from the docs; blocked on a confirmed DataEssenceCoding UL (see DESIGN_TODO)
- **Dead scaffolding modules** — atmos.rs, audio.rs, stereo3d.rs, dtsx.rs (shelled out to a nonexistent `asdcp-wrap`, zero callers; superseded by the real postkit-based 3D/Atmos/MCA wiring)
- **OpenJPEG encoder** — grok is now the only J2K encoder. Dropped the `create --encoder` flag and the openjpeg branch; the postkit dependency enables `grok-ffi` (not `openjpeg`). libgrokj2k must be discoverable by pkg-config at build time and loadable at runtime
- **KDM `--formulation`** — was inert (MessageType is fixed by ST 430-1); removed from CLI/config/docs
- **`verify --output .pdf`** — silently produced plain text; option dropped
- **Dead modules** — dcp_diff, plugin, preferences, geometry, and the prores re-export shim (zero callers)

### Fixed
- **`edit` no longer leaves a stale signature** — editing a signed DCP rewrote the CPL, PKL and ASSETMAP but kept their `ds:Signature`, so the output verified as tampered (dcpdoctor `signature_invalid`). `edit` now drops the signature from every document it rewrites, and warns which ones lost it. The package comes out unsigned rather than falsely signed. Re-sign it afterwards if it has to stay signed
- **Subtitle vertical position** — SMPTE and Interop subtitle generators anchored the block at the top (Vposition 85 with Valign="bottom") so subtitles rendered near the top of the screen. The bottom line now sits at 8% from the bottom with lines stacked upward (a two-line cue renders at 15% and 8%)
- **GUI "Show in Files"** — uses the tauri opener plugin (`revealItemInDir`); the shell `open` call only accepted URLs
- **Copy-to-drive verify** — flushes to the device and drops the page cache before reading back, so verification is real
- **FFOC marker value** — default markers now emit FFOC = 1 (was 0). libdcp's Bv2.1 verifier raises `INCORRECT_FFOC` unless FFOC == 1; LFOC stays at the last frame

## [1.1.0] — 2026-05-28

### Added
- **Timeline view** — Visual DCP timeline with multi-reel navigation, duration bars, and asset type indicators
- **CLI flag consistency test** — `tests/cli_flags_test.sh` verifies GUI invocations match actual CLI flags
- **CI: CLI flag check** — Automated verification in GitHub Actions pipeline

### Fixed
- **Properties panel → build pipeline** — All properties (standard, resolution, framerate, bandwidth, colour, content_kind, encrypt, stereo_3d, channels) now correctly passed to the DCP creation pipeline
- **Verify flags** — `--check-mxf` → `--no-picture-check`, `--skip-hashes` → `--no-hash-check` (correct inverted logic)
- **Encode flags** — Removed non-existent `--resolution`/`--framerate`, added `-i` input flag
- **Transcode flags** — Removed non-existent `--format`/`--bit-depth`, uses `-i` flag
- **Copy flags** — Positional args replaced with `--src`/`--dst` named args
- **Report flags** — Uses `--dcp` and `-o` instead of non-existent `--format`
- **Subtitle conversion** — `Command.create` → `Command.sidecar`, `--framerate` → `--fps`, added `-i`/`-l` flags
- **Burn-in** — `Command.create` → `Command.sidecar`, correct `-i`/`-s`/`-o` flags
- **Target conversion** — `Command.create` → `Command.sidecar`, correct `-i`/`-t`/`-m` flags
- **KDM** — Added CPL ID, content title, and output path fields; uses correct `--cpl-id`/`--content-title`/`--cert`/`-o` flags
- **Encrypt standalone** — Shows informational message (standalone re-encryption not supported by CLI)

## [1.0.0] — 2025-01-20

### Added
- **CLI: Create subcommand** — Full DCP creation from JPEG 2000 + audio MXF inputs
  - `--content-type` (feature, trailer, advertisement, etc.)
  - `--frame-rate` (24, 25, 30, 48, 60)
  - `--twok` / `--fourk` resolution selection
  - `--video-bit-rate` for encoding control
  - `--threads` for parallel encoding
  - `--container-ratio` (flat, scope, full)
  - `--audio-channels` configuration
- **CLI: Verify subcommand** — Full DCP verification with options
  - `--no-hash-check` to skip hash verification
  - `--no-picture-check` to skip picture essence inspection
  - `--strict` for SMPTE-strict mode
  - `--output` for writing reports to file (text or HTML)
  - `--quiet` to suppress progress output
- **CLI: KDM subcommand** — Key Delivery Message generation
  - `--valid-from` / `--valid-to` for validity periods (`--valid-to` accepts a
    relative duration: "2 weeks", "30 days", "4 hours")
  - `--keys` to carry the DCP's content keys (from `create --encrypt`)
  - `--signer-cert` / `--signer-key` (required) for the ETM signature
  - `--formulation` (modified-transitional-1, dci-any, dci-specific)
- **Panic hook** — User-friendly crash messages with issue tracker link
- **CLI integration tests** — 10 end-to-end tests using assert_cmd
- **Release CI** — GitHub Actions workflow for building release binaries on tag push
- **GUI Release CI** — Tauri build workflow producing .deb, .AppImage, .dmg, .msi

### Changed
- Version unified to 0.5.0 across all workspace crates
- Git dependencies pinned to v0.5.0 tags (asdcplib-rs, dcpdoctor, postkit)
- ContentType, Resolution, Standard enums now in dcpwizard-core::lib

### Fixed
- OpenJPEG thread-safety issues resolved via Grok JPEG 2000 encoder
- Clippy warnings cleaned up across entire workspace
