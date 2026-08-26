# DESIGN_TODO

Paths: CORE = rust/crates/dcpwizard-core/src, CLI = rust/crates/dcpwizard-cli/src/main.rs,
PK = extern/postkit (postkit submodule; bump the pin when postkit changes).
DoM refs (dom#N = https://dcpomatic.com/bugs/view.php?id=N) are DCP-o-matic tracker
feature requests. Shared DSP/parsers belong in postkit (see its DESIGN_TODO); the
user-facing surface is here.

## Open

- GUI re-verification owed. None of it has been clicked through in a running
  window: the QC overlays drawn at and across end of file, without freezing and
  without a frame-rate hit (watch the HUD decoder fps), the playlist behaviour
  when rows are cleared (the preview stops or clears when the queue owns it, one
  advance per end of file), the transport bar tracking during playback and its
  skip and frame-step buttons, the decode-resolution menu and HUD, the crop
  overlay and the subtitle/CC render toggles. Everything in guikit's preview
  header is owed the same pass in imfwizard.
- `vf` and `assemble` write no `CompositionMetadataAsset`: both can replace or
  combine the sound, so the source CPL's `MainSoundConfiguration` need not
  describe their output, and they would have to read the sound essence to
  declare it.
- Hints not ported from DCP-o-matic's list, each for a reason. Signing certificate
  checks (utf8 subject strings, a chain valid for more than 15 years) are about
  the configured signer rather than the job, and our signer is held to ST 430-2
  at sign time instead. MPEG2 and VOB inputs do not exist here. Mixed encryption
  cannot happen: `--encrypt` is all or nothing. 3D content in a 2D DCP has no
  equivalent, since a right eye is named per job rather than carried by content.
  The size limits on text assets (an Interop font over 640 kB, a SMPTE reel over
  4096 PNG subtitle resources, a caption XML over 256 kB, a subtitle MXF over
  115 MB) are refusals here or in postkit's font subsetter rather than advice,
  and the ones that are not would need the DCST rendered to measure, which the
  hints pass deliberately does not do.
- Checks left in the front ends on purpose, because each names a flag or a panel
  control and the two spell them differently: every spelling parser (`--rotate`,
  `--flip`, `--upmix`, `--container`, `--marker`, the appearance flags), and the
  pairings that refuse two ways of saying one thing (`--audio-map` beside
  `--upmix`, a channel WAV directory or `--audio-input-order lrc-ls-rs-lfe`;
  `--still-length` with a video; a trim on a still; an appearance flag with no
  track to style; the reel-split sources). `preflight` takes the rules whose
  message names the content, not the control.
- A marker past the composition length is refused after the encode
  (`markers::markers_for_composition`, from `create_dcp`). The plan-time pass
  hints at a marker sitting at or past the picture length rather than refusing
  one, because the frame count it works from is the source's and the packaged
  length also depends on padding the packager applies. Moving the refusal forward
  needs the padded length to be settled in the plan.

- ISDCF naming, two open ends. The title builder takes free-text studio codes and
  territories, where it could pull the current ISDCF registry instead of naming
  from whatever the user typed. And the GUI builds the name at submit time, before
  any frame is encoded, so a package whose picture resolution is Auto is named from
  the flat fallback aspect rather than the raster the encoder lands on. The CLI
  names after the encode and gets it right. CORE isdcf_title.rs, GUI pipeline.rs.
- Neither the macos nor the windows embedded-preview host has run on real
  hardware. All three hosts are implemented in guikit and CI compiles every
  platform, so what is left is a hand pass on a mac and on a windows box.
- Windows release builds are unproven until the next tag run. Watch for grok's
  msvc install dropping more dlls that grokj2k.dll depends on, in which case
  release.yml and gui-release.yml should copy bin/*.dll instead of the one file.
  A local windows tauri build fails at bundle time unless the dll is staged at
  gui/src-tauri/grokj2k.dll.
- Distributed encoding across machines (dom#155, dom#1635, dom#2605). Out of scope
  (user-excluded). The job queue is single-machine and its create path wraps
  pre-encoded J2K rather than running postkit::pipeline, so job progress is
  stage-based, not per-frame.
- Playlist / SPL playback (DCP-o-matic ships this as a separate dcpomatic2_playlist
  tool feeding dcpomatic2_player). Sequence several packages into one list the
  player walks in order. The embedded preview is otherwise at parity with that
  player: postkit `preview` resolves a CPL by uuid, decrypts encrypted picture
  essence with the content key, colour-manages and can drive a GPU decoder. What it
  lacks is the list, since `PlaybackOptions` names one `input` and one `cpl_uuid`,
  so this needs a queue above it plus GUI ordering. Nothing about package
  correctness depends on it.
- Live playback decodes J2K with libavcodec, not grok. Two decode paths exist: the
  frame-accurate preview (PK preview.rs) resolves the CPL, decrypts and decodes
  through grok, but live playback (PK mpv.rs, and the embedded surface through PK
  mpv_render, which is libmpv's render API and still decodes inside mpv) hands the
  picture MXF to mpv, whose decode is ffmpeg's native software jpeg2000 codec. At
  DCP bitrates that decoder runs at a few fps, which is why a 250 Mbps track sits
  black for seconds on load and can read as a frozen GUI. Grok is much faster and is
  ours. Routing playback through it means postkit decoding frames itself (the
  preview.rs machinery already resolves, decrypts and colour-manages) and presenting
  them on the embedded surface, with mpv kept for audio or dropped; feeding mpv raw
  frames over a pipe is the other route, but no pipe format carries 12-bit X'Y'Z',
  so the colour conversion would have to happen before the pipe either way. GUI.
- The daemon queue (CORE/job_queue.rs) is a Mutex around an in-memory map with
  no save or load, so a daemon crash or reboot loses every pending job, and unlike
  DCP-o-matic, whose batch jobs are film projects on disk that can be re-added, our
  jobs carry their whole config as JSON params in memory, so losing the queue loses
  the specifications. The GUI is worse: its Jobs panel is a separate JobQueue in
  tauri state (gui pipeline.rs), never talks to the daemon (only `serve` proxies to
  it), and dies with the window. Both halves are mostly plumbing: Job is already
  Serialize/Deserialize, so persist to an XDG-dir JSONL on submit and state change
  and reload pending jobs on daemon start, and have the GUI submit over the existing
  IPC when the daemon is up, falling back to in-process when it is not.
- `--source-colourspace` refuses aces and acescg correctly, they need a rendering
  transform (LUT). For logc "correctly" is softer: DCP-o-matic handles Sony
  S-Log3/S-Gamut3 analytically (inverse log curve then matrix, libdcp
  `s_gamut3_to_xyz`), and ARRI LogC's EOTF is published math, so a
  transfer-function-ahead-of-matrix arm in `DcdmTransform` is the shape if a
  customer shows up with LogC masters. An image sequence grk_compress reads
  straight from file also stays refused for p3 and rec2020, since it only converts
  Rec.709. A sequence that a burn, a picture change or jpeg/png frames route
  through ffmpeg reaches the same per-frame transform a video does. DCP-o-matic
  also allows fully custom conversions (user chromaticities, white point, gamma),
  a flexibility we do not have anywhere.
- Interop KDM (`kdm --format interop`) is legacy and unvalidated: no reference
  library generates Interop (libdcp only reads it) and the suite has no reference
  Interop KDM to diff against. Validate against real legacy gear before production.
  This one cannot be closed by testing: it needs hardware.
- conform gaps (the formats themselves are in DESIGN.md): AAF video is
  code-complete but untested against a real file, since libaaf's public test
  corpus has video tracks but no video clips. AAF pan and gain automation are
  surfaced in the timeline's skipped list but not applied, deliberate scope.
- Sony RAW / X-OCN is detected but undecodable (ffmpeg can't decode it), same as
  ARRIRAW/R3D/BRAW/Canon: a match only yields a clearer detected-but-undecodable
  error. postkit's detect_format matches Sony's private essence ULs in the .mxf
  header. Those ULs are reverse-engineered from MediaInfo, NOT SMPTE-registered,
  and mark the Sony RAW family without distinguishing X-OCN ST/LT/XT tiers, which
  is fine since the match only sharpens the error. Non-Sony .mxf resolves to
  DNxHR.

- cosmic-text's bidi handling could replace the hand-rolled `--subtitle-rtl`
  reshaping.
- The standalone `burnin` command is redundant for DCP work. `create
  --burn-subtitle` burns in one generation on every input shape, while `burnin` costs
  an extra lossy transcode and its `--font-size` is inert for subtitles (read only in
  the `drawtext` watermark branch). Worth deciding whether it stays as a plain
  video-to-video tool or goes.

### Batch E (easyDCP parity, surveyed 2026-08-16)

From en.easydcp.com easyDCP Plus and IMF Studio (both now EUR 3567.62 permanent or
EUR 164.22/month, so the README's "EUR 2,998" line is stale). Most of what those
pages advertise is already here. These four are not.

- GPU J2K encoding. Both easyDCP products sell GPU/CUDA acceleration, and
  DCP-o-matic reaches it through `config grok-licence`. The grok library has no
  GPU encode path of its own: it is a separately licensed accelerator plugin
  (`grk_plugin_load` and `grk_plugin_init` with a device id and a licence key).
  postkit's DESIGN_TODO has the scoping. One postkit change plus a device and
  licence setting here and in imfwizard.
- HD-SDI monitoring output. easyDCP Player+ and IMF Player both drive Blackmagic
  hardware. This is a port rather than new work: imfwizard already has `sdi-preview`,
  which runs a GStreamer decklink pipeline and probes for the plugin first
  (imfwizard-core `tools.rs`, `has_gst_decklink`). The open question is whether the
  embedded preview should feed it or it stays a separate command.
- Atmos KDM. easyDCP's KDM Generator+ advertises "SMPTE (incl. Dolby Atmos)". Not a
  new item: it is the tail of the encrypted timed text and Atmos bullet above, since
  a KDM can only carry an Atmos key once `wrap_atmos` encrypts the essence.
- imfwizard's GUI frame-rate menu stops at 60 while its CLI takes an arbitrary
  `--fps-num`/`--fps-den`, against easyDCP's advertised 23.98 to 120. Tracked in
  imfwizard's own DESIGN_TODO, noted here so the survey stays whole.

Two claims from those pages we could not judge and should read the specs for before
calling them gaps: "Dolby Vision 4.0 packaging" (imfwizard converts RPU profiles 8.1
and 8.4, unclear whether that is what 4.0 means) and the IMF "Extended" application
alongside App2 and ProRes.

### Transkoder survey (2026-08-17)

From colorfront.com/software/transkoder plus the NAB 2026 press release. Colorfront
publishes no spec sheet, release notes, manual or price; the most detailed feature
list is a reseller page pinned to Transkoder 2022, so their column mixes "verified
current" with "true in 2022, probably still". Confirmed absent or undocumented on
their side: TMS upload, Interop DCPs, watch folders, published pricing, and any
named scope types. Their clear leads are GPU J2K speed with 8K SDI output, camera
RAW ingest, Dolby Vision cinema authoring (DV2, eCMU, licensed), IMF App4/App5/RDD45
breadth, QC detectors, and the render-farm/cloud story. Items worth landing here:

- Two-pass J2K encoding with PSNR/bitrate targeting. We enforce
  `codestream_byte_cap`; nothing targets quality. postkit's encode path. imfwizard
  too.
- Black-frame and repeated/freeze-frame detection. Cheap passes over frames postkit
  already decodes, surfaced in the QC report. imfwizard too.
- Side-by-side / wipe / difference compare in the preview. `frame_compare` has the
  metrics (PSNR/SSIM/VMAF); nothing shows two compositions ganged. guikit, so both
  wizards. Transkoder 2026 adds semantic composition diffing on top; metrics plus a
  visual compare is the part worth matching.
- Waveform and vectorscope in the preview. Transkoder implies scopes ("HDR
  analyzer") but never enumerates them. guikit, both wizards.

## Keep in sync with imfwizard (deliberately duplicated, no clean shared home)

The shared *logic* lives in postkit (mpv::MpvPlayer, packaging writers, escape_xml,
parse_srt, pipeline::run_encode) and the shared GUI glue in guikit. What remains
duplicated is app glue with no clean cross-repo home, left as copies. If you edit
one side, mirror the other:

- gui/vite.config.js: per-app, only partially aligned. The dev port differs,
  and consuming guikit needs a `server.fs.allow` plus a `resolve.dedupe` for the
  bare @tauri-apps imports, since guikit sources sit outside the vite root. Mirror
  any other change.
- extern/guikit/src/shortcuts.js: both wizards import it from guikit. dcpdoctor is
  not a submodule consumer and keeps a vendored copy synced by plain cp, so a guikit
  change to this file still needs a manual copy into dcpdoctor. App-agnostic by
  design: all app specifics enter through initShortcuts, never a per-repo edit.
- gui/src/timeline.js: per-app deliberately, a genuine domain difference rather
  than drift, so it is not a guikit candidate. It is a thin renderer over disjoint
  backend structs: dcpwizard's TimelineEntry reels against imfwizard's SegmentEntry
  segments. Unifying would mean a field-mapping layer larger than the duplication.
- gui/src-tauri/src/lib.rs, gui/src-tauri/src/pipeline.rs: app-specific tauri setup
  and build orchestration. They delegate the encode to postkit::pipeline but have
  diverged enough that unifying would need per-divergence config flags. Everything
  the create panel adds sits on dcpwizard-core (sign_language, pad, audio_route,
  reel, profiles, versions) or is dcpwizard-only by format (atmos, stereoscopic 3D,
  the DCI HDR addendum), so there is nothing to mirror. What is shared arrives
  through postkit instead: upmix would port to imfwizard's panel unchanged, and the
  source colour path and the codestream cap reach it by bumping its postkit pin.
- .github/workflows/ci.yml, release.yml, gui-release.yml: copies across dcpwizard,
  imfwizard, dcpdoctor differing by binary/artifact names + per-app build deps.
  Separate git repos, so no shared reusable-workflow without a central repo. Keep
  aligned by hand. Every job that compiles the rust workspace sets grok up through
  PostPerfection/setup-grok@v1 at grok-ref v20.3.12, windows on a separate msvc
  build of the same tag. All three platforms are required in ci, none are
  continue-on-error.
  imfwizard mirrors the step but only in ci (it runs grk_compress at runtime, does not
  link grok-ffi). dcpdoctor needs no grok.
- tests/cli_flags_test.sh: NOT the same harness as imfwizard's (this one runs the
  binary and checks clap parse errors, imf parses main.js). Different CLIs, leave
  separate.
