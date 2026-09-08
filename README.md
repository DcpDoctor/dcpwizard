# DCP Wizard

[![CI](https://github.com/PostPerfection/dcpwizard/actions/workflows/ci.yml/badge.svg)](https://github.com/PostPerfection/dcpwizard/actions/workflows/ci.yml)

[Documentation](https://postperfection.github.io/dcpwizard/)

Digital Cinema Package (DCP) creator, CLI tool and desktop GUI.

Version 1.2 creates consistent CPL, PKL, and ASSETMAP identities for SMPTE and Interop packages. Grok is the JPEG 2000 encoder.

## Overview

DCP Wizard creates valid DCPs (Digital Cinema Packages) from video sources,
image sequences, and WAV audio, conforming to SMPTE ST 429 (Bv2.1) and Interop standards.

Free and open-source alternative to easyDCP Creator+.

## Features

### DCP Creation & Packaging
- **Original Version (OV) DCP** creation from J2K + WAV
- **Pre-build check** on `create`: every refusal the packager can make is made before the encode starts, from one shared check, so a job that cannot be packaged fails in a second instead of after an hour of J2K. `create --check` runs that check plus the advisory hints and stops, encoding nothing and writing nothing under `--output`
- **SMPTE & Interop** standard support
- **2K and 4K** resolution (2048×1080, 4096×2160)
- **Frame rates** 24, 25, 30 fps (2K/4K); HFR 48, 50, 60, 96, 100, 120 fps (2K only)
- **Reel splitting** via `create --reel-length <minutes>` (multi-reel CPL, sample-accurate audio and per-reel subtitle boundaries)
- **Explicit reel splits** via `create --split-at <tc>[,<tc>...]` (HH:MM:SS or HH:MM:SS:FF) or `create --split-chapters` (source chapter marks via ffprobe)
- **Custom picture container** via `create --container <2k-scope|...>` or `create --container-dims WxH` (even, within the 2K/4K bound): the container is the raster the picture is encoded at, so a scope package is 2048x858 frames, the CPL's stored area, active area and aspect all read back from that essence, and the container names the 2K or 4K family without `--twok`/`--fourk`
- **Automatic source fitting**: the source is scaled into the container preserving its aspect, so a 1920x1080 HD master with `--container 2k-scope` encodes at 2048x858 with letterbox bars only where the aspects differ. `--twok`/`--fourk` with no container fit the source onto the full 2048x1080 or 4096x2160 frame instead of refusing it
- **Source picture processing** on `create`: `--crop-left/--crop-right/--crop-top/--crop-bottom <px>` (source pixels, before any rotation), `--auto-crop` with `--auto-crop-threshold <0..1>` (black borders measured over the content), `--fill-crop` (crop to the container aspect so the picture fills the frame), `--deinterlace`, `--denoise`, `--rotate 90|180|270` (clockwise) and `--flip horizontal|vertical|both`
- **Head/tail padding** via `create --pad-head <dur> --pad-tail <dur>` (`48f`/`2s`), with `--pad-color <RRGGBB>` for a filled pad instead of black
- **Trim** via `create --trim-start <dur> --trim-end <dur>` (same syntax), cutting the source before any padding; picture, sound and subtitles move together
- **Still images** via `create --video <image> --still-length <dur>`: one image held for a duration, encoded once and repeated
- **Source colour space** via `create --source-colourspace rec709|p3|rec2020|xyz|logc` (default `rec709`), declaring what the source carries; P3, Rec.2020 and ARRI LogC3 are converted to X'Y'Z' during the encode
- **Input decode range** override via `create --input-range full|legal`, correcting wrong or absent source range flags
- **High Bitrate (HBR)**, up to 500 Mbps for demanding content
- **CPL / PKL / ASSETMAP / VOLINDEX** generation
- **Multi-version packages** via `create --versions <file>`: one package with several CPLs sharing the same picture/sound essence, differing by subtitle and/or audio track (multiple language versions over one master)
- **Multi-composition packages** via `create-multi --compositions <manifest>`: one CPL per manifest entry, each with its own picture/sound/subtitle, over one shared PKL/ASSETMAP
- **Assemble** a new OV from existing DCPs via `assemble --input <dcp>... --output --title` (reels in order, essence copied byte-identical by UUID)
- **Edit** CPL metadata (title/annotation/content-kind/issuer) without re-wrapping essence via `edit --input`
- **Decrypt** an encrypted DCP to cleartext via `decrypt --input --output` with `--kdm --recipient-key` or `--keys`. Every essence round-trips: picture, sound, subtitle, closed caption and Atmos
- **Sign-language video** track via `create --sign-language-video <file> --sign-language-lang <rfc5646>` (ISDCF Doc 13, SLVS on sound channel 15)
- **ISDCF content titles** via `create --isdcf-name`: the title is rebuilt to the ISDCF naming convention from what the package actually carries (content type, container aspect, audio and text languages, channel ladder, territory and rating, resolution, studio, date, facility, standard). `--audio-lang`, `--rating AGENCY=LABEL` (repeatable), `--content-version` (repeatable), `--studio`, `--territory-type`, `--isdcf-date`, `--temp-version`, `--pre-release`, `--red-band`, `--two-d-version-of-three-d` and `--version-file` feed it, and the metadata among them lands in the CPL with or without the naming flag
- **Ratings and content version in the CPL**: `--rating` fills the `RatingList` and the first `--content-version` is the `ContentVersion` LabelText
- **Bv2.1 CompositionMetadataAsset** (ST 429-16) in the first reel of SMPTE CPLs, with `MainSoundConfiguration` derived from the packaged channel count
- **CPL markers** (ST 429-7 `MainMarkers`) in every created composition: FFOC/LFOC by default, or place any of the ten defined markers with `create --marker LABEL=timecode` (repeatable, frame number or HH:MM:SS:FF; single-reel only)
- **Re-ingest packaging** via `ingest-package <dir>`: rebuild ASSETMAP and PKL to cover every asset file present (for exported OV/VF folders whose ASSETMAP/PKL omit hardlinked assets), no re-wrap
- **SHA-1 hashing** for integrity verification

### Encoding & Transcoding
- **JPEG 2000 encoding** via Grok (create, pipeline, and DCP transcode paths)
- **CPU and GPU encoding**: CPU encoding uses the available cores by default. `--gpu` or the desktop GPU setting enables Grok's accelerator plugin for JPEG 2000 encode and decode. An explicit `--gpu` fails if the plugin cannot start, while a saved desktop preference warns and continues on the CPU. `--no-gpu` forces the CPU
- **Video file import**, QuickTime (.mov), MP4, MXF, AVI, MKV
- **Video transcoding**, ProRes, H.264, H.265, DNxHR → image sequence → J2K (via ffmpeg)
- **Image sequence input**, DPX, TIFF, EXR, PNG, JPEG, BMP
- **Scale / Crop / Letterbox**, target resolution adaptation
- **Colour conversion** to XYZ (DCI, gamma 2.6)
- **Encode QoL** on `create`: `--start-at` (scheduled wall-clock start), `--resume` (reuse on-disk J2K frames after an interruption, survives restarts), average-fps ETA in progress output, `--shutdown-when-done` (power off after a clean encode)

### Encryption & KDM
- **AES-128 essence encryption** of picture, sound, timed text and Atmos, content keys generated with a CSPRNG, encrypted at wrap time
- **Signed SMPTE KDM** (ST 430-1 / 430-3) carrying the DCP's content keys, one per essence (MDIK, MDAK, MDSK, MDEK)
- **Batch KDM**, generate for multiple screens in one pass
- **DKDM re-wrap**, re-issue KDMs from a Distribution KDM
- **KDM formulation** via `--formulation` on `kdm`/`kdm-batch`/`kdm-rewrap` (the four ISDCF Doc 5 spellings; derived from `--device-cert` when omitted)
- **Forensic marking control** via `-p/--disable-forensic-marking-picture` and `-a/--disable-forensic-marking-audio [CHANNEL]`, for press and festival screenings
- **Certificate generation**, X.509 cert chain (root → intermediate → signer)
- **Certificate inspection**, display subject, issuer, validity, thumbprint, CA status

### Subtitles & Captions
- **SRT → SMPTE / Interop subtitle XML** conversion, anchored near the bottom with configurable `--vposition` (percent from the bottom, default 8%)
- **Subtitle packaging** into a DCP timed-text track (ST 428-7 DCST wrapped as an MXF, registered in the CPL) via `create --subtitle`
- **Input formats** beyond SRT: `create --subtitle` accepts `.ass`/`.ssa` (styling + alignment), `.pac` (EBU binary, Latin codepage), `.mks`/`.mkv` (Matroska subtitle stream via ffmpeg), `.fcpxml` (captions/titles) and an Interop `DCSubtitle` XML carrying PNG bitmap subs. Styling (italic/bold/underline/colour) and per-cue alignment/position are carried into the DCST; supplied SMPTE DCST XML is still wrapped unchanged.
- **Placement controls** on `create`: `--subtitle-halign left|center|right`, `--subtitle-valign top|center|bottom` (top-anchored subtitles grow downward), `--subtitle-vposition <pct>`, so SRT is no longer always centred-bottom
- **3D subtitle depth** via `create --subtitle-zposition <v>` (SMPTE ST 428-7 Zposition emitted on every cue). Note: the timed-text descriptor's "Z-Position In Use" flag is not set by the current asdcplib writer.
- **RTL subtitles** (`create --subtitle-rtl auto|on|off`, default auto): Hebrew/Arabic is reshaped and reordered to visual order so servers that skip the bidi algorithm render it correctly
- **Auto line-wrap** via `create --subtitle-wrap <chars>`: long lines wrap on whitespace (never mid-word), preserving styling
- **Font embedding + subsetting** via `create --subtitle-font <ttf/otf>`: the font is subset to the used glyphs (SMPTE 640 KB limit, fails loud if exceeded) and embedded in the timed-text MXF, referenced by a `LoadFont`; `--subtitle-no-subset` embeds the whole font. Under reel splitting the font is referenced by one shared asset id in every reel.
- **Timed-text appearance** on `create`: `--subtitle-font-size <points>`, `--subtitle-colour <RRGGBB[AA]>`, `--subtitle-effect none|outline|shadow`, `--subtitle-effect-colour <RRGGBB[AA]>`, `--subtitle-fade-up <ms>` and `--subtitle-fade-down <ms>` write the ST 428-7 `Font` attributes and the per-cue fades. Defaults are the ones the track has always carried: 42 points, white on a black shadow, a twelfth of a second each way. An outline is written as the standard's `border`. They style the `--subtitle` track only, so `--ccap` keeps the default appearance
- **Subtitle editing** on standalone files via `subtitle-edit`: `--list` cues, `--shift-ms` all cues, or `--index N` with `--text` / `--set-start-ms`+`--set-end-ms`, written back as SRT (it edits source files, never subtitles inside a finished DCP)
- **Subtitle extraction** from a DCP or subtitle asset back to `.srt` (timed) or `.txt` (text only) via `subtitle-extract`; reads MXF-wrapped ST 428-7 and loose SMPTE/Interop XML, concatenating reels with their timeline offsets
- **Multilingual subtitles** with RFC 5646 language tags
- **Burn-in during the encode** via `create --burn-subtitle <file>` (+ `--burn-subtitle-font <ttf/otf>`): the cues are drawn into the picture as it encodes, so a burnt festival print costs one generation rather than two. Takes the same formats `--subtitle` does, and covers video, image sequences and held stills. Burnt text is part of the image and registers no timed-text track; the same file cannot be both, and burning onto an already-X'Y'Z' source or a J2K directory is refused
- **Burn-in appearance** on `create`: `--burn-font-size <pct of frame height>`, `--burn-colour <RRGGBB[AA]>`, `--burn-effect none|outline|shadow`, `--burn-effect-colour <RRGGBB[AA]>`, `--burn-outline-width <pct of text height>`, `--burn-line-height <multiple of text height>`, `--burn-margin <pct of frame height>`, `--burn-x-scale`, `--burn-y-scale`, `--burn-fade-up <ms>` and `--burn-fade-down <ms>`. Each is laid over postkit's burn defaults, so an unnamed one keeps the value it always had. Any of them without `--burn-subtitle` is refused by name
- **Subtitle burn-in as a standalone pass** via `burnin`, for a review copy rather than a package: it writes a video file, so a DCP burn goes through `create --burn-subtitle` instead of this. `--font-size`, `--colour <RRGGBB>` and `--position top|center|bottom` style the cues, and `--video-codec <encoder>` with `--crf <n>` name the output encoder and its quality rather than leaving both to ffmpeg's guess from the output file name: `--video-codec libx264 --crf 0` writes a lossless copy

### Audio
- **PCM audio wrapping** (48 kHz)
- **Loudness measurement**, EBU R128 / ATSC A/85
- **Loudness normalization** to a target via `create --loudness-target leqm=<db>|lufs=<v>` (with `--true-peak-ceiling`)
- **Channel mapping matrix** via `create --audio-map <IN:OUT[@GAIN],...>`: any source channel to any DCP lane at any gain in dB, summing where several land on one lane. OUT is a lane name (L, R, C, LFE, Ls, Rs, Lc, Rc, BsL, BsR, HI, VI) or a 1-based number, and the track is widened to the smallest DCP sound layout that holds every named lane
- **Packaged channel count** via `create --audio-channels 2|6|8|16`: the sound track is filled with silent channels up to the count, so a stereo source ships in a 16-channel container without an upmix. Without the flag a 5.1 source is widened to 16 and everything else is packaged at its own width. A source wider than the count is refused, with `--audio-map` to fold channels instead
- **Stereo→5.1 upmix** at create via `create --upmix a|b`
- **Audio delay** via `create --audio-delay <ms>` (positive later, negative earlier), keeping the running time
- **Filename channel auto-routing**: point `create --audio` at a directory of mono `name_L.wav`/`_R`/`_C`/`_Lfe`/`_Ls`/`_Rs`… files
- **Crossfade join** of two WAVs via `crossfade --a --b -o --overlap`
- **Mid-side decode** via `mid-side-decode -i -o --mid --side`
- **WAV audio** input

### Quality Control
- **Integrated QC** via dcpdoctor (SMPTE Bv2.1 compliance checking)
- **HTML QC report** generation, with a Sound level section: Leq(m) per ISO 21727 for every sound track, against the content kind's limit (82 dB advertisement, 85 dB trailer). `report --scan-picture` adds a Picture section, decoding each picture track for black and frozen runs; it is off by default because the J2K decode is ffmpeg's software one and a feature takes hours, and a report made without it says the picture was not scanned
- **DCP verification**, validate structure, hashes, XML schemas
- **Markers**, FFOC, LFOC, FFTC, LFTC, FFOI, LFOI, FFEC, LFEC, FFMC, LFMC; the
  default set is FFOC/LFOC, place any of the others with `--marker LABEL=timecode`

### HDR & Dolby Vision
- **HDR detection**, auto-detect SDR, HDR10, HDR10+, Dolby Vision, HLG from source
- **Dolby Vision RPU injection** via dovi_tool
- **HDR10 static metadata** injection (SMPTE ST 2086 + CTA 861.3)
- **HDR format conversion**, HDR10 ↔ HLG ↔ SDR tone mapping
- **HDR source delivery** via `create --hdr-to-dci-lut <lut>` (runs the LUT before J2K encode); `--allow-generic-hdr-tonemap` opts into FFmpeg tone mapping with a warning, and is refused with `--hdr-dci`. `create --hdr-dci` authors a DCI HDR Addendum DCP from an HDR master: the picture MXF is stamped with TransferCharacteristic=ST 2084 (PQ), the CPL carries the ST 429-16 `Image Encoding Parameters` extension with EOTF `ST 2084`, and the ISDCF name takes the `HDR1` content modifier. The master's own colour tags name the grade, or `--hdr-source <hdr10|hlg|pq-p3d65|dolby-vision>` names it (a Dolby Vision profile 8.1 base layer is read as HDR10, and profile 5 is refused). `--hdr-peak-nits <cd/m²>` is where the roll-off starts, defaulting to the master's MaxCLL, then its mastering display maximum, then 1000. An SDR or untagged master is refused. `--hdr-to-dci-lut` and `--hdr-already-pq` are the other two paths to PQ. A `--burn-subtitle` or `--watermark` is refused on any of the three: both draw in display RGB and the frames are no longer that. Not available with 3D or reel splitting

### Timeline Conform
- **Timeline conform** from EDL (CMX 3600) / FCP7 XML (xmeml) / FCPX (fcpxml): parse, or with `--media-dir --output` resolve every reel to media and build a finished multi-reel DCP (per-reel encode + wrap + CPL assembly). The reel/asset plan (`conform_plan.json`) and conform manifest are kept as artifacts

### Export & Playback
- **Export DCP** to ProRes, H.264, H.265, DNxHR, or image sequence
- **Frame extraction**, extract individual frames as images (thumbnails/preview)
- **Frame-accurate preview**: DCP directories, CPLs, picture MXFs and J2K directories decode in process through Grok. The CPU worker pool sustains 2K playback, and the accelerator plugin adds device decode and the X'Y'Z' to display-RGB transform for 4K playback. MP4, ProRes and other sources use libmpv. Encrypted and stereoscopic J2K also use libmpv

### Delivery & Automation
- **Copy to drive** with a free-space precheck and post-copy hash verification (USB/CRU); **format-drive** (ext2/ext3, volume label, mounted-target refusal) and **check-drive** (report fs type + label) for cinema hard-drive delivery
- **Watch folder**: `watch DIR --output OUT` builds a DCP from every video file or frame folder dropped into `DIR` once it stops changing, using the file stem as the title and a same-named `.wav` and `.srt` beside it as sound and subtitle. The job log is written beside the package, the source moves into `done/` or `failed/`, `--webhook-url` posts `dcp.created` or `dcp.failed`, `--interval` sets the poll seconds and any `create` flags after `--` apply to every package
- **Job queue** with progress tracking and cancellation
- **REST API** for headless/batch operation
- **Prometheus metrics endpoint** (`GET /metrics`), job counts, daemon status for monitoring
- **Docker container** for CI/CD pipelines
- **CLI scriptable**, all features accessible from command line
- **Version dashboard**, OV/VF management, territory tracking, distribution matrix export
- **Webhook notifications**, HTTP callbacks on job completion/failure
- **TMS upload**, push a finished package to a theatre management system over sftp (host key checked against `known_hosts`) or ftp, on its own or straight after a build

### Mastering & Compliance
- **DCDM creation**, Digital Cinema Distribution Master (X'Y'Z' 12/16-bit) intermediate
- **Visible watermarking** via `watermark`, a text mark (distributor ID/serial) burnt into an existing DCP's JPEG 2000 picture essence, with `--font-size` as a percent of the frame height, `--colour`, `--position top|center|bottom` and `--font`. The picture is decoded, marked and re-encoded at its own average bandwidth unless `--video-bit-rate` names another; sound and timed text ship unchanged, and `--kdm`/`--recipient-key`/`--keys` mark an encrypted source. `create --watermark TEXT` marks at build time instead
- **Trailer packaging**, ratings cards (MPAA/BBFC/FSK), green/red band, countdown leaders; the packaged mp4 is then encoded and wrapped into a real trailer DCP
- **Content version tracker**, SQLite database of which version delivered where and when
- **Accessibility compliance**, read the closed-caption asset and the HI, VI-N and sign-language channels off a package and check them against CVAA, EAA, AODA or Ofcom. Each list below is dcpwizard's own mapping of the access services an instrument names onto DCP tracks rather than a citation, since the EAA, AODA and Ofcom instruments all name access services for broadcast, on-demand or web delivery and none of them names a cinema deliverable:
  - CVAA requires a closed-caption asset and a VI-N narration channel, and recommends an HI mix channel
  - EAA requires a closed-caption asset and a VI-N narration channel, and recommends sign-language video and an HI mix channel
  - AODA requires a closed-caption asset, and recommends a VI-N narration channel and an HI mix channel
  - Ofcom requires a closed-caption asset and a VI-N narration channel, and recommends sign-language video

## Installation

### Pre-built binaries (recommended)

Download from the [GitHub Releases](https://github.com/PostPerfection/dcpwizard/releases/latest) page:

| Platform | CLI | Desktop GUI |
|----------|-----|-------------|
| **Linux** (x86_64) | `dcpwizard-linux-x86_64.tar.gz` | `.deb`, `.AppImage` |
| **macOS** (Apple Silicon) | `dcpwizard-macos-aarch64.tar.gz` | `.dmg` |
| **Windows** (x86_64) | `dcpwizard-windows-x86_64.zip` | `.msi` |

The CLI links the Grok JPEG 2000 library (libgrokj2k) dynamically, so the Grok runtime must be present. Point `LD_LIBRARY_PATH` at Grok's `lib64` (Linux) or the equivalent before running.

### Install from source

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get install -y pkg-config libxml2-dev libssl-dev libxerces-c-dev
# For GUI: also install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev

# Grok (libgrokj2k) must be discoverable by pkg-config at build time and its
# shared lib loadable at runtime. Build it from source or install a release, then:
export PKG_CONFIG_PATH="/path/to/grok/lib64/pkgconfig:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="/path/to/grok/lib64:$LD_LIBRARY_PATH"

cd rust
cargo build --release
# Binary at rust/target/release/dcpwizard
```

#### macOS

```bash
brew install pkg-config libxml2 openssl@3 xerces-c

export OPENSSL_DIR=$(brew --prefix openssl@3)
export PKG_CONFIG_PATH="$(brew --prefix openssl@3)/lib/pkgconfig:$(brew --prefix libxml2)/lib/pkgconfig:$(brew --prefix xerces-c)/lib/pkgconfig"

cd rust
cargo build --release
```

#### Windows

```powershell
# Using vcpkg (recommended)
vcpkg install libxml2 openssl xerces-c --triplet x64-windows

$env:VCPKG_ROOT = "$env:VCPKG_INSTALLATION_ROOT"

cd rust
cargo build --release
```

### Optional runtime dependencies

| Dependency | Purpose | Install |
|-----------|---------|---------|
| `ffmpeg` | Video transcoding and import | `apt install ffmpeg` / `brew install ffmpeg` / [ffmpeg.org](https://ffmpeg.org/download.html) |
| `mpv` | GUI preview player for sources that are not JPEG 2000 | `apt install mpv` / `brew install mpv` / [mpv.io](https://mpv.io/installation/) |

### Docker

```bash
docker build -t dcpwizard .
docker run -v /path/to/media:/data dcpwizard create \
    --title "My Film" --video /data/j2k --audio /data/audio.wav --output /data/dcp
```

### Desktop GUI

The GUI uses [Tauri 2](https://tauri.app/) (Rust backend + web frontend) with a single-window layout inspired by professional NLEs.

**GUI features:**
- Drag & drop file import (video, audio, subtitle)
- Keyboard shortcuts (Ctrl+N/O/B/P/I, Ctrl+1–7 for views, Space/arrows/Home during preview). Ctrl+K opens the shortcut list, where clicking a shortcut rebinds it (Backspace clears, Escape cancels) and the rebindings are saved
- Recent projects quick-access list
- Right-click context menus on assets (Preview, Remove, Show in Files)
- Asset filter / search
- Auto-detect framerate and resolution from imported video (via ffprobe)
- Pre-build hints: a Before you build dialog lists what will package but is likely to be wrong on a cinema screen, with Build anyway or Go back. Turn it off from the dialog or in Settings ("Show hints before building"), and the hints still reach the job log
- Post-build actions: a finished build offers Play (the new DCP in the embedded preview), Inspect (the Verify view, already pointed at the output and running) and Reveal (the output folder in the file manager), beside the progress bar. Starting another build clears the row
- Per-stage timings in the job log: `[TIMING]` lines next to each stage's own log lines giving preflight, encode, audio, packaging and validation time, plus the total
- Progress in title bar (visible in taskbar during builds)
- Desktop notifications on build complete/fail
- Conditional button enabling (Build disabled until ready)
- Built-in preview player with timeline scrubber (click-to-seek, drag-to-scrub, timecode display). A DCP, a picture MXF, a CPL or a directory of codestreams plays through grok, in process, at 2K real time on the CPU and at 4K real time on the device when the GPU setting is on. Everything else plays through mpv. The header strip carries the QC controls: safe area, aspect mask, centre cross, thirds grid, the crop the build will apply, decode resolution and the subtitle and closed-caption tracks
- Multi-CPL composition tabs, switch, add, remove compositions
- SRT → SMPTE subtitle conversion panel
- Subtitle extraction panel (DCP/asset → SRT or text)
- Subtitle burn-in panel
- Target resolution conversion panel (2K/4K scope/flat/full)
- GPU encoding toggle (grok acceleration)

```bash
cd gui
pnpm install
pnpm tauri dev
pnpm tauri build
```

grok looks for `libgrokj2k_plugin` in the directory `GRK_PLUGIN_PATH` names, then in the working directory, then in the executable's own directory, and never on `LD_LIBRARY_PATH` or `PATH`. A GUI launched without `GRK_PLUGIN_PATH` therefore encodes on the CPU even with the GPU toggle on:

```bash
export LD_LIBRARY_PATH=/path/to/grok/lib64
export GRK_PLUGIN_PATH=/path/to/grok/lib64
./src-tauri/target/release/dcpwizard-gui
```

A desktop launcher inherits neither variable, so put both on the `.desktop` Exec line or in `~/.config/environment.d`. The job log at `<output>/dcpwizard.log` confirms the device ran: the header prints `Accelerator: requested, active` and the encode is followed by `[ENCODE] Frames on the device: N of M`.

## CLI Usage

```bash
# Create a DCP
dcpwizard create --title "My Feature Film" --video ./j2k --audio ./audio.wav --output ./dcp

# Create from video file (full pipeline: decode → J2K encode → MXF wrap → DCP)
dcpwizard create --title "My Film" --video movie.mov --output ./dcp

# Check the job before committing to the encode: every refusal, then every hint.
# Nothing is encoded and nothing is written under --output. Exits 1 on a refusal.
dcpwizard create --title "My Film" --video movie.mov --output ./dcp --check

# Transcode an existing DCP's picture essence to a lower bandwidth (audio and
# subtitle tracks are copied unchanged; encrypted input is rejected)
dcpwizard transcode-dcp --input ./dcp --output ./dcp_light --video-bit-rate 100

# Create with encryption. Content keys are generated with a CSPRNG and every
# essence is AES-128 encrypted at wrap time: picture, sound, subtitle,
# closed caption and Atmos each get their own key. --key-out is required: it is
# the only place the keys are written (never next to the DCP). That file holds
# the plaintext keys, keep it secret and outside the DCP. Feed it to
# `kdm --keys`, which puts every key in the KDM.
dcpwizard create --title "My Film" --video ./j2k --audio ./audio.wav --output ./dcp \
    --encrypt --key-out ./secret/my_film.keys.json

# Create a signed DCP. The CPL and PKL get an XML-DSig ds:Signature (SMPTE
# ST 429-7/-8), which encrypted packages require and validators such as
# ClairMeta expect. --signer-chain takes the CA certificates above the leaf,
# intermediate(s) then root, and is repeatable. Signing is opt-in: without
# --signer-cert the package is written unsigned as before. The private key is
# read straight from the file by dcpwizard, never passed to another process.
dcpwizard create --title "My Film" --video ./j2k --output ./dcp \
    --signer-cert signer.pem --signer-key signer.key \
    --signer-chain intermediate.pem --signer-chain root.pem

# Create Interop DCP
dcpwizard create --title "My Film" --video ./j2k --output ./dcp --standard interop

# Create with content type and resolution
dcpwizard create --title "My Trailer" --video trailer.mov --output ./dcp \
    --content-type TLR --fourk --video-bit-rate 500

# Encode to a quality target instead of a ratio. --quality-psnr is a PSNR target
# in dB, at least 20 and at most 80. The encoder allocates to that quality rather
# than to a compression ratio, and --video-bit-rate becomes a per-frame byte cap
# no frame may exceed: a frame the quality target pushes over it is encoded again
# by ratio to fit. The DCI cap (or the HDR Addendum's, with --hdr-dci) still
# applies, so the cap is the smaller of the two, and a 3D encode gives each eye
# half of it.
dcpwizard create --title "My Film" --video movie.mov --output ./dcp \
    --video-bit-rate 100 --quality-psnr 45

# Create with frame rate override
dcpwizard create --title "My Film" --video ./j2k --output ./dcp --frame-rate 25

# Split a long feature into ~20-minute reels (multi-reel CPL)
dcpwizard create --title "My Feature" --video ./j2k --audio ./audio.wav \
    --output ./dcp --reel-length 20

# Explicit reel splits at timecodes, or at the source's chapter marks
dcpwizard create --title "My Feature" --video movie.mov --output ./dcp \
    --split-at 00:20:00,00:41:30:12
dcpwizard create --title "My Feature" --video movie.mov --output ./dcp --split-chapters

# Custom container: a named DCI container or arbitrary even dimensions
dcpwizard create --title "My Film" --video ./j2k --output ./dcp --container 2k-flat
dcpwizard create --title "My Film" --video ./j2k --output ./dcp --container-dims 1920x1080

# Fit an HD master into the scope container: the black bars are cropped off and
# the picture is scaled to 2048x858, which is the raster the DCP is encoded at
dcpwizard create --title "My Film" --video movie.mov --output ./dcp \
    --container 2k-scope --fill-crop --deinterlace

# Route a stereo source across the DCP lanes: L and R untouched, a -6 dB centre
dcpwizard create --title "My Film" --video movie.mov --audio stereo.wav \
    --output ./dcp --audio-map "1:L,2:R,1:C@-6"

# Head/tail padding with a coloured pad frame (48f or 2s; default black)
dcpwizard create --title "My Film" --video ./j2k --audio ./audio.wav --output ./dcp \
    --pad-head 2s --pad-tail 48f --pad-color 000010

# Force the source decode range (corrects wrong/absent range flags)
dcpwizard create --title "My Film" --video movie.mov --output ./dcp --input-range full

# Name the DCP by the ISDCF convention. --title is the human title the content
# title is built from. The rating and the content version also land in the CPL.
# This one names the package
#   MyFilm_FTR-1_F_EN-XX_20_2K_ABC_20260816_SMPTE_OV
dcpwizard create --title "My Film" --video movie.mov --audio stereo.wav --output ./dcp \
    --isdcf-name --content-type FTR --audio-lang en --studio ABC \
    --rating "http://www.mpaa.org/2003-ratings=PG-13" --content-version "Final Cut"

# Sign-language video track (ISDCF Doc 13, carried on sound channel 15)
dcpwizard create --title "My Film" --video ./j2k --audio ./audio.wav --output ./dcp \
    --sign-language-video signer.mov --sign-language-lang sgn-ase

# HDR source with an HDR-to-DCI 3D LUT (LUT runs before J2K encode)
dcpwizard create --title "My Film" --video hdr.mov --output ./dcp \
    --hdr-to-dci-lut hdr_to_dci.cube

# DCI HDR Addendum package from an HDR10 master graded to 1000 cd/m²
dcpwizard create --title "My Film" --video hdr10.mov --output ./dcp \
    --hdr-dci --hdr-source hdr10 --hdr-peak-nits 1000

# Create with a subtitle track (SRT -> ST 428-7 timed text, wrapped and registered)
dcpwizard create --title "My Film" --video ./j2k --output ./dcp \
    --subtitle subs.srt --subtitle-language en

# Multi-version package: one master, several CPLs differing by subtitle/audio.
# The base flags define the shared picture/sound; each versions.json entry is a CPL.
dcpwizard create --title "My Film" --video ./j2k --audio ./audio.wav \
    --output ./dcp --versions versions.json
# versions.json:
#   [
#     { "title": "My Film (EN)", "subtitle": "en.srt", "subtitle_language": "en" },
#     { "title": "My Film (FR)", "subtitle": "fr.srt", "subtitle_language": "fr" }
#   ]

# Stereoscopic 3D: main input is the left eye, --right-eye is the right eye
# (both encoded at the same settings, wrapped into one ST 429-10 picture MXF)
dcpwizard create --title "My 3D Film" --video left.mov --right-eye right.mov \
    --output ./dcp --frame-rate 24

# Dolby Atmos aux track (ST 429-18). Pass a bitstream file or a directory of
# per-frame payloads. Real-essence conformance needs real Atmos material.
# DTS:X: since ST 429-18/-19 it is delivered as a standard IAB track (ST 2098-2,
# "DTS:X for IAB"), which is this same --atmos path. There is no separate DTS:X UL.
dcpwizard create --title "My Film" --video ./j2k --audio ./audio.wav \
    --output ./dcp --atmos ./atmos.iab

# Closed captions (ST 429-12): an accessibility track with a MainClosedCaption
# CPL role, distinct from open --subtitle. Same input formats. Carried through
# every CPL path: single-reel, reel splitting, versions (a `ccap` manifest field),
# and VF (--add-ccap/--replace-ccap REEL=PATH). Written as ST 429-12
# <tt:ClosedCaption>, the element that namespace declares.
dcpwizard create --title "My Film" --video movie.mov \
    --output ./dcp --ccap captions.srt --ccap-language en

# Accessibility channels: label sound channel 6 as HI and 7 as VI-N
dcpwizard create --title "My Film" --video ./j2k --audio ./8ch.wav \
    --output ./dcp --hi-channel 6 --vi-channel 7

# Audio processing at create time: route a directory of mono channel WAVs
# (name_L.wav, name_R.wav, ...), upmix stereo to 5.1, normalize loudness
dcpwizard create --title "My Film" --video movie.mov --output ./dcp \
    --audio ./channels --upmix a --loudness-target leqm=85 --true-peak-ceiling=-1.0

# Standalone audio tools: equal-power crossfade join, mid-side decode
dcpwizard crossfade --a first.wav --b second.wav -o joined.wav --overlap 1.0
dcpwizard mid-side-decode -i ms.wav -o lr.wav --mid 0 --side 1

# Encode QoL: start at a wall-clock time, resume after an interruption,
# power off when the encode finishes
dcpwizard create --title "My Film" --video movie.mov --output ./dcp \
    --start-at 22:00 --resume --shutdown-when-done

# Full pipeline: video → J2K → DCP in one pass (no intermediate files)
dcpwizard pipeline -i movie.mov -t "My Film" -o ./dcp --audio mix.wav

# Supplemental Version File (VF): replace reel 1's sound against an existing OV.
# Unchanged reels reference the OV by asset id; only the new MXF ships in the VF.
# Replacement can be raw essence (WAV/J2K, gets wrapped) or an already-wrapped MXF.
dcpwizard create-vf --ov ./dcp --output ./dcp_vf --replace-sound 1=./new_mix.wav
# Validate the VF against its OV (resolves cross-references):
dcpdoctor validate ./dcp_vf --ov ./dcp
# Subtitle VF: add or replace a reel's subtitle (SRT or SMPTE XML). A subtitle-only
# VF references the OV picture/sound by id and ships just the new subtitle MXF.
dcpwizard create-vf --ov ./dcp --output ./dcp_vf \
    --add-subtitle 1=./fr.srt --subtitle-language fr

# Assemble a new OV composition from existing DCPs (reels in program order)
dcpwizard assemble --input ./short_dcp --input ./feature_dcp \
    --output ./assembled --title "Short + Feature"

# Edit a DCP's CPL metadata in place (or into --output); refuses encrypted DCPs
dcpwizard edit --input ./dcp --title "My Film (2024 Restoration)" --content-kind FTR

# Multi-composition package: one CPL per manifest entry over a shared PKL/ASSETMAP
dcpwizard create-multi --compositions comps.json --output ./dcp
# comps.json:
#   [
#     { "title": "Ep 1", "j2k_dir": "./ep1/j2k", "audio": "./ep1/mix.wav" },
#     { "title": "Ep 2", "j2k_dir": "./ep2/j2k", "audio": "./ep2/mix.wav" }
#   ]

# Decrypt an encrypted DCP to cleartext (keys from a KDM + recipient key, or KEYS.json)
dcpwizard decrypt --input ./enc_dcp --output ./clear_dcp \
    --kdm kdm.xml --recipient-key recipient.key
dcpwizard decrypt --input ./enc_dcp --output ./clear_dcp --keys ./secret/my_film.keys.json

# Encode images to JPEG 2000 codestreams in ./out/j2k
dcpwizard encode --input ./dpx --output ./out --bandwidth 250 --fps 24

# Transcode video to image sequence (format/bit-depth optional)
dcpwizard transcode --input movie.mov --output ./sequence --format dpx --bit-depth 16

# Export a DCP picture MXF to a delivery format (ProRes/H.264/H.265/DNxHR/image-sequence)
dcpwizard export --input picture.mxf --output out.mp4 --format h264 --audio sound.mxf

# Create DCDM (Digital Cinema Distribution Master)
dcpwizard dcdm -i ./frames -o ./dcdm --colour-space rec709

# Colour space conversion (rec709/p3/rec2020, or the DCDM transforms xyz and p3-d65)
dcpwizard colour -i ./rec709_frames -o ./p3_frames --source rec709 --target p3
dcpwizard colour -i ./rec709_frames -o ./p3d65_frames --source rec709 --target p3-d65

# Verify an existing DCP
dcpwizard verify ./my_dcp

# Verify with options (skip slow hash check, output report)
dcpwizard verify ./my_dcp --no-hash-check --output report.html

# Strict SMPTE Bv2.1 verification
dcpwizard verify ./my_dcp --strict --quiet

# Inspect DCP metadata
dcpwizard info ./my_dcp

# Generate KDM
dcpwizard kdm --cpl-id <uuid> --content-title "My Film" --cert recipient.pem \
    --signer-cert signer.pem --signer-key signer.key \
    --keys ./secret/my_film.keys.json --output kdm.xml

# KDM with validity period (--valid-to accepts a relative duration)
dcpwizard kdm --cpl-id <uuid> --content-title "My Film" --cert recipient.pem \
    --signer-cert signer.pem --signer-key signer.key --keys ./secret/my_film.keys.json \
    --output kdm.xml --valid-from now --valid-to "2 weeks"

# KDM with specific dates
dcpwizard kdm --cpl-id <uuid> --content-title "My Film" --cert recipient.pem \
    --signer-cert signer.pem --signer-key signer.key --keys ./secret/my_film.keys.json \
    --output kdm.xml --valid-from 2024-06-01T00:00:00+00:00 \
    --valid-to 2024-06-30T23:59:59+00:00

# KDM with a custom AnnotationText (default: "<title> KDM for <recipient>")
dcpwizard kdm --cpl-id <uuid> --content-title "My Film" --cert recipient.pem \
    --signer-cert signer.pem --signer-key signer.key --keys ./secret/my_film.keys.json \
    --output kdm.xml --annotation "My Film - Screen 1 - June run"

# Interop (legacy) KDM. --format defaults to smpte; interop uses the digicine
# ETM. Validate against real legacy gear before production use.
dcpwizard kdm --cpl-id <uuid> --content-title "My Film" --cert recipient.pem \
    --signer-cert signer.pem --signer-key signer.key --keys ./secret/my_film.keys.json \
    --output kdm.xml --format interop

# Copy to cinema drive
dcpwizard copy --src ./my_dcp --dst /mnt/cru_drive

# Format a delivery drive ext2/ext3 (refuses any mounted target; needs --yes)
dcpwizard format-drive /dev/sdb --fs ext3 --label MY_FILM --yes
# Check an existing drive's filesystem and label without touching it
dcpwizard check-drive /dev/sdb

# Measure audio loudness
dcpwizard loudness audio.wav

# Generate QC report
dcpwizard report --dcp ./my_dcp --output report.html

# ...and decode the picture too, listing black and frozen runs per reel (slow)
dcpwizard report --dcp ./my_dcp --output report.html --scan-picture

# Start REST API server
dcpwizard serve --bind 127.0.0.1:8080

# Build a DCP from every master dropped into ./incoming. feature.mp4 with
# feature.wav and feature.srt beside it becomes ./packages/feature, its log is
# ./packages/feature.log and the three sources move into ./incoming/done.
dcpwizard watch ./incoming --output ./packages

# Poll every 30 s, notify a service, and build every package as Interop scope
dcpwizard watch ./incoming --output ./packages --interval 30 \
  --webhook-url https://example.com/hooks/dcp \
  -- --standard interop --container 2k-scope

# Job queue daemon. $DCPWIZARD_DAEMON_ADDR sets the address it listens on
# (default 127.0.0.1:9457). The queue is written to
# ~/.local/share/dcpwizard/jobs.jsonl, one JSON line per job on submit and on
# every state change, and read back on start, so a crash or a reboot does not
# lose queued jobs. $DCPWIZARD_JOBS_FILE points a second daemon at another file.
#
# The desktop GUI runs a queue of its own, written to
# ~/.local/share/dcpwizard/gui-jobs.jsonl in the same format and listed in the
# Jobs panel beside the daemon's. $DCPWIZARD_GUI_JOBS_FILE points a second GUI
# at another file. The two variables name different files: DCPWIZARD_JOBS_FILE
# is the daemon's jobs.jsonl, DCPWIZARD_GUI_JOBS_FILE the GUI's gui-jobs.jsonl.
dcpwizard daemon

# Manage job queue
dcpwizard batch list
dcpwizard batch add -T create-dcp -p '{"title":"My Film","video":"./j2k","output":"./dcp"}'
dcpwizard batch cancel <job-id>

# Shell completion
dcpwizard completion bash >> ~/.bashrc
dcpwizard completion zsh >> ~/.zshrc
dcpwizard completion fish > ~/.config/fish/completions/dcpwizard.fish

# Convert SRT subtitles to SMPTE DCP XML
dcpwizard subtitle-convert --input subs.srt --output subs.xml --language en

# Subtitle track with placement, wrap, RTL and an embedded subset font.
# --subtitle accepts .srt/.ass/.pac/.mks/.mkv/.fcpxml/Interop-PNG or SMPTE DCST XML.
dcpwizard create --title "My Film" --video ./j2k --output ./dcp \
    --subtitle subs.ass --subtitle-language ar \
    --subtitle-halign center --subtitle-valign bottom --subtitle-vposition 8 \
    --subtitle-rtl auto --subtitle-wrap 42 --subtitle-font NotoSansArabic.ttf

# How the packaged track looks: 50-point yellow text with no effect and a
# 200 ms fade each way. --ccap keeps the default appearance.
dcpwizard create --title "My Film" --video ./j2k --output ./dcp \
    --subtitle subs.srt --subtitle-font-size 50 --subtitle-colour FFFF00 \
    --subtitle-effect none --subtitle-fade-up 200 --subtitle-fade-down 200

# How burnt-in text looks: yellow, outlined, 8% of the frame height tall
dcpwizard create --title "My Film" --video master.mov --output ./dcp \
    --burn-subtitle subs.srt --burn-colour FFFF00 --burn-effect outline \
    --burn-font-size 8 --burn-outline-width 6

# Edit a standalone subtitle file (any parsable format), written back as SRT
dcpwizard subtitle-edit --input subs.srt --list
dcpwizard subtitle-edit --input subs.srt --shift-ms -500 --output shifted.srt
dcpwizard subtitle-edit --input subs.srt --index 3 --text "Fixed line" \
    --set-start-ms 12000 --set-end-ms 14000 --output subs.srt

# Extract timed text from a DCP back to SRT (or .txt for text only)
dcpwizard subtitle-extract --input ./my_dcp --output subs.srt

# Burn subtitles into a video file, for review rather than for a package
dcpwizard burn-in --input movie.mov --subtitles subs.srt --output movie_burned.mov \
    --colour FFFF00 --position bottom --video-codec libx264 --crf 0

# Scale/crop video to DCI target resolution
dcpwizard convert --input movie.mov --output movie_2k_scope.mov --target 2k-scope
# Targets: 2k-scope (2048×858), 2k-flat (1998×1080), 2k-full (2048×1080),
#          4k-scope (4096×1716), 4k-flat (3996×2160), 4k-full (4096×2160)

# Conform an EDL/FCP7-XML timeline. Parse-only, or with --media-dir/--output
# resolve reels to media and build a finished multi-reel DCP (conform_plan.json
# and the conform manifest are kept as artifacts).
dcpwizard conform -i timeline.edl --json
dcpwizard conform -i timeline.edl --media-dir ./reels --output ./conform_out

# Extract a single frame from MXF/video
dcpwizard frame-extract -i video.mxf -f 100 -o frame100.png

# Inject Dolby Vision RPU into HEVC
dcpwizard dv-inject -i input.hevc -r metadata.bin -o output.hevc

# Inject HDR10 static metadata
dcpwizard hdr10-inject -i input.mov -o output.mov --max-cll 1000 --max-fall 400

# Burn a visible watermark into an existing DCP's picture essence
dcpwizard watermark -i ./MyFilm_DCP -o ./MyFilm_DCP_marked -p "DIST-001-SERIAL" \
    --font-size 6 --colour FFFF00 --position top

# Or mark at build time
dcpwizard create --title "My Film" --video movie.mov --output ./out \
    --watermark "DIST-001-SERIAL"

# Batch KDM: one KDM per recipient certificate in a single pass.
# List certs with repeated --cert, or point --cert-dir at a directory
# of cinema certs (every *.pem/*.crt/*.cer gets a KDM).
dcpwizard kdm-batch --cpl-id <uuid> --content-title "My Film" \
    --cert-dir ./cinema-certs \
    --signer-cert signer.pem --signer-key signer.key \
    --signer-chain intermediate.pem --signer-chain root.pem \
    --keys ./secret/my_film.keys.json --output-dir ./kdms

# ── KDM distribution ────────────────────────────────────────────────────────
# Cinema/screen database (default: ~/.local/share/dcpwizard/cinemas.json,
# override with --db). Screens hold a recipient cert; serial + thumbprint are
# cached for search. No private keys are ever stored.
dcpwizard cinema add --name "Odeon" --email ops@odeon.test --notes "priority"
dcpwizard cinema add-screen --cinema Odeon --name "Screen 1" --cert screen1.pem
dcpwizard cinema add-screen --cinema Odeon --name "Screen 2" --cert screen2.pem --inline
dcpwizard cinema list
# the thumbprint is the base64 ST 430-2 value a KDM carries, not a hex digest
dcpwizard cinema search YGxSeoFg          # by name, or cert serial/thumbprint
dcpwizard cinema import-flm facility.xml  # import an FLM-x (SMPTE 430-7) file

# Named validity templates (default: ~/.local/share/dcpwizard/kdm-templates.json)
dcpwizard kdm-template add --name preshow --duration "1 week" --tz-offset "+02:00"
dcpwizard kdm-template add --name movie --start-offset "0 days" --duration "180 days"
dcpwizard kdm-template list
# use a template for the validity window (explicit --valid-from/--valid-to override)
dcpwizard kdm --cpl-id <uuid> --content-title "My Film" --cert screen1.pem \
    --signer-cert signer.pem --signer-key signer.key --template preshow --output kdm.xml

# Batch to whole cinemas / single screens from the db (--cert/--cert-dir still work)
dcpwizard kdm-batch --cpl-id <uuid> --content-title "My Film" \
    --cinema Odeon --screen "Rex/Screen 1" \
    --signer-cert signer.pem --signer-key signer.key --output-dir ./kdms

# Email the KDMs (one zipped email per cinema, dom#2516) using an SMTP config
# (see "KDM email config" below). --email-only-additional ignores cinema contacts.
dcpwizard kdm-batch --cpl-id <uuid> --content-title "My Film" --cinema Odeon \
    --signer-cert signer.pem --signer-key signer.key --output-dir ./kdms \
    --smtp-config ~/.config/dcpwizard/smtp.toml --email-to distributor@studio.test

# KDM generation history (default: ~/.local/share/dcpwizard/kdm-history.jsonl).
# Every successful kdm/kdm-batch appends a metadata record (never key material).
dcpwizard kdm-history --title "My Film" --since 2026-07

# Download a projector/server recipient cert by vendor + serial. Anonymous
# endpoints: dolby/doremi and qube. christie/gdc/barco need a vendor account
# (--user/--password; the password is never logged). Requires the `curl` binary.
dcpwizard cert-fetch --vendor dolby --serial 218281828 -o screen.pem
dcpwizard cert-fetch --vendor qube --type QXPD --serial 54 -o screen.pem
dcpwizard cert-fetch --vendor christie --serial 218281 --user me --password '***' -o screen.pem
dcpwizard cert-fetch --vendor barco --serial 1234567890 --user me --password '***' -o screen.pem

# Package a trailer (ratings card + countdown leader + content)
dcpwizard trailer -c trailer.mov -o ./trailer_pkg --title "My Film" \
    --rating "PG-13" --rating-system mpaa --band green --countdown 8

# Generate DCP markers for a composition
dcpwizard markers --frames 172800        # FFOC/LFOC list
dcpwizard markers --frames 172800 --xml  # XML MarkerList
# place any of the ten markers (frame number or HH:MM:SS:FF; validated <= length)
dcpwizard markers --frames 172800 --marker FFEC=01:59:00:00 --marker LFEC=02:00:00:00 --xml
# the same markers written into a real CPL MainMarkers asset at create time
dcpwizard create --title "My Film" --video movie.mov --output ./dcp \
    --marker FFEC=01:59:00:00 --marker LFEC=02:00:00:00

# Check accessibility compliance
dcpwizard accessibility ./my_dcp --standard cvaa   # cvaa|eaa|aoda|ofcom

# Send a webhook notification
dcpwizard webhook --url https://example.com/hook --event job.completed \
    --job-id 42 --payload '{"status":"ok"}'

# Upload a finished package to a theatre management system
dcpwizard tms /path/to/MyFilm_FTR_F_EN-XX_OV --tms-config ~/.config/dcpwizard/tms.toml

# Or upload straight after a build
dcpwizard create --title "My Film" --video film.mp4 --audio film.wav \
    -o /out/MyFilm_OV --upload-to-tms

# Content version / delivery tracking (SQLite)
dcpwizard version record --db deliveries.db --package-uuid <uuid> \
    --title "My Film" --version OV --destination "AMC" --method hard_drive --verified
dcpwizard version list --db deliveries.db
dcpwizard version export --db deliveries.db --output deliveries.csv

# OV/VF version dashboard
dcpwizard dashboard register --uuid <uuid> --title "My Film" \
    --version-type OV --territory US --status released
dcpwizard dashboard list
dcpwizard dashboard matrix --output distribution.csv
dcpwizard dashboard serve --port 9090
```

### KDM email config

`kdm`/`kdm-batch` `--smtp-config` points at a TOML file. It holds the SMTP
password, so keep it outside the repo (e.g. `~/.config/dcpwizard/smtp.toml`,
mode 600). The password is never logged or echoed in errors.

| key | required | notes |
|-----|----------|-------|
| `host` | yes | SMTP server hostname |
| `port` | yes | e.g. 465 (tls) or 587 (starttls) |
| `security` | no | `tls` (default), `starttls`, or `none` |
| `username` | no | SMTP auth user |
| `password` | no | SMTP auth password |
| `from` | yes | sender address |
| `subject_template` | no | supports `{title}` and `{cinema}` |
| `body_template` | no | supports `{title}` and `{cinema}` |

```toml
host = "smtp.example.com"
port = 587
security = "starttls"
username = "keys@studio.example"
password = "..."
from = "Studio Keys <keys@studio.example>"
subject_template = "KDMs for {title} — {cinema}"
```

With `--cinema`, one email per cinema is sent to its stored contact emails (plus
any `--email-to`), each with that cinema's KDMs zipped. `--email-only-additional`
sends only to `--email-to`.

### TMS upload config

`tms` and `create --upload-to-tms` read a TOML file, by default
`~/.config/dcpwizard/tms.toml` (`--tms-config` points elsewhere). It holds the TMS
password or the key passphrase, so keep it outside the repo, mode 600. Neither is
ever logged, echoed in errors or passed as a command-line argument.

| key | required | notes |
|-----|----------|-------|
| `protocol` | yes | `sftp` or `ftp` |
| `host` | yes | TMS hostname or IP |
| `port` | no | defaults to 22 for sftp, 21 for ftp |
| `path` | yes | remote directory the package directory is created under |
| `user` | yes | login user |
| `password` | unless `private_key` is set | login password |
| `private_key` | no | sftp only, path to an ssh private key, the password is not sent when it is set |
| `private_key_passphrase` | no | only for an encrypted `private_key` |

```toml
protocol = "sftp"
host = "tms.cinema.example"
path = "/mnt/dcp"
user = "projectionist"
password = "..."
```

Every file of the package is uploaded into `<path>/<package directory name>/`.
An sftp host must be in `~/.ssh/known_hosts`: an unknown or changed host key is
refused rather than handed the password, and the error prints the fingerprint and
the `ssh-keyscan` line that records it once the cinema has confirmed it. Plain
`ftp` sends the login and the package unencrypted, so prefer `sftp` wherever the
TMS offers it, and an ftp config that names a `private_key` is refused.

## REST API

Start the server:
```bash
dcpwizard serve --bind 0.0.0.0:8080
```

Or via Docker:
```bash
docker run -p 8080:8080 -v /path/to/media:/data dcpwizard serve --bind 0.0.0.0:8080
```

`--api-key <key>` requires that key in `X-Api-Key` or `Authorization: Bearer` on
every request but `GET /health`, and answers 401 without it. Without the flag the
API is open to anyone who can reach the bind address, so bind to `127.0.0.1` or
put it behind something that authenticates. The key is compared without stopping
at the first differing byte, and a key on the command line is visible to anything
that can read the process list:
```bash
dcpwizard serve --bind 0.0.0.0:8080 --api-key "$(cat /run/secrets/dcpwizard-api-key)"
```

Every job route (`GET /jobs`, `POST /create`, `POST /verify`, `GET /metrics`)
proxies to the job daemon, and answers 503 naming `dcpwizard daemon` when it is
not running.

## Comparison with easyDCP Creator+

| Feature | DCP Wizard | easyDCP Creator+ |
|---------|-----------|------------------|
| SMPTE & Interop | ✅ | ✅ |
| Version Files (VF) | ✅ | ✅ |
| Multi-CPL timeline | ✅ (`create --versions`) | ✅ |
| CLI scriptable | ✅ | ✅ |
| Up to 4K | ✅ | ✅ |
| Stereoscopic 3D | ✅ | ✅ |
| Frame rates 24–60 fps | ✅ | ✅ |
| High Bitrate (500 Mbps) | ✅ | ✅ |
| DPX/TIFF/EXR/PNG/JPEG/BMP/QuickTime input | ✅ | ✅ |
| Scale/Crop/Letterbox | ✅ | ✅ |
| J2K Transcoder | ✅ | ✅ |
| Audio (PCM 5.1) | ✅ | ✅ |
| Immersive audio (Dolby Atmos) | ✅ | ✅ |
| MCA channel labeling (2.0/5.1/7.1 + HI/VI) | ✅ | ✅ |
| SRT→SMPTE subtitles | ✅ | ✅ |
| Subtitle packaging into DCP | ✅ | ✅ |
| Subtitle burn-in | ✅ | ✅ |
| Integrated QC | ✅ (dcpdoctor) | ✅ (Fraunhofer) |
| KDM (SMPTE + Interop) | ✅ | ✅ |
| AES-128 essence encryption | ✅ | ✅ |
| DKDM re-wrap | ✅ | ✅ |
| Desktop GUI | ✅ (Tauri) | ✅ (native) |
| REST API / Docker | ✅ | ❌ |
| Watch folder automation | ✅ | ❌ |
| Visible watermarking | ✅ | ❌ |
| DCDM intermediate format | ✅ | ❌ |
| Trailer packaging (ratings/leaders) | ✅ | ❌ |
| Content version tracking | ✅ | ❌ |
| Accessibility compliance (CVAA/EAA) | ✅ | ❌ |
| Dolby Vision / HDR10 / HLG | ✅ | ❌ |
| Timeline conform (EDL/FCP7 XML) | ✅ | ❌ |
| Version dashboard & distribution matrix | ✅ | ❌ |
| Open source | ✅ (AGPL-3.0) | ❌ |
| **Price** | **Free** | **Commercial** |

## Architecture

```
dcpwizard/
├── rust/                # Rust workspace
│   ├── crates/
│   │   ├── dcpwizard-core/  # Core library, 59 modules, DCP creation, encoding, encryption, KDM, QC
│   │   └── dcpwizard-cli/   # CLI binary (dcpwizard)
│   └── Cargo.toml
├── gui/                 # Tauri 2 desktop application
│   ├── src/             # Frontend (Vite + vanilla JS)
│   └── src-tauri/       # Rust backend (plugin shell)
└── docs/                # GitHub Pages site
```

DCP Wizard shares common functionality with [IMF Wizard](https://github.com/PostPerfection/imfwizard)
via the [postkit](https://github.com/PostPerfection/postkit) library (encoding, transcoding, hashing,
job queue, preferences, REST API, watch folders, and more).

## License

AGPL-3.0-or-later. Copyright (C) 2026 Grok Image Compression Inc. See [LICENSE](LICENSE).
