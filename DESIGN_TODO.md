# DESIGN_TODO

Paths: CORE = rust/crates/dcpwizard-core/src, CLI = rust/crates/dcpwizard-cli/src/main.rs,
PK = extern/postkit (postkit submodule; bump the pin when postkit changes).
DoM refs (dom#N = https://dcpomatic.com/bugs/view.php?id=N) are DCP-o-matic tracker
feature requests. Shared DSP/parsers belong in postkit (see its DESIGN_TODO); the
user-facing surface is here.

## Open

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
- A marker past the composition length is still refused after the encode
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
- Cross-platform embedded preview: all three hosts are implemented in guikit
  (linux GtkGLArea verified live, macos NSOpenGLView layered over the WKWebView,
  windows WS_CHILD window with wgl over the WebView2 child), pinned here and in
  imfwizard, and CI compiles all three platforms green. Remaining: neither the
  macos nor the windows host has run on real hardware, so a hand pass there is
  the last step. The platform contract stays three items: `attach(&tauri::Window)
  -> Result<EmbeddedPreview, String>`, `EmbeddedPreview::player()` and
  `EmbeddedPreview::set_surface(x, y, w, h, visible)`, identical on every
  platform.
- Windows release builds: release.yml and gui-release.yml build grok through
  the PostPerfection/setup-grok@v1 action (grok v20.3.10, the same step ci.yml
  uses), the cli zip ships grokj2k.dll beside the exe, and
  tauri.windows.conf.json bundles it into the installer next to the exes.
  Unproven until the next tag run. Watch on that run: if grok's msvc install
  drops more dlls that grokj2k.dll depends on, copy bin/*.dll in both places
  instead, and a local windows tauri build now fails at bundle time unless the
  dll is staged at gui/src-tauri/grokj2k.dll.
- postkit compiled twice, fixed 2026-08-12. `postkit` was a path dep on
  `extern/postkit` while `dcpdoctor-core` came from git carrying its own path dep
  on the postkit inside that checkout, so cargo resolved two copies. dcpdoctor
  declares postkit by git now, and `rust/Cargo.toml` and `gui/src-tauri/Cargo.toml`
  each carry a `[patch]` redirecting that git source at `extern/postkit`, so both
  references collapse onto the submodule this workspace already builds. The
  edit-the-submodule-and-rebuild loop is unchanged and `cargo tree -d` reports no
  postkit duplicate. imfwizard has the same shape.
- Encrypted timed text and Atmos (PK mxf_wrap.rs): `wrap_timed_text` and
  `wrap_atmos` never call `setup_encryption` and pass no AES/HMAC contexts to the
  writers, though asdcplib-rs takes them on `write_timed_text_resource`,
  `write_ancillary_resource` and the Atmos `write_frame`. Until that lands,
  `--encrypt` refuses a package carrying a subtitle, closed-caption or Atmos track
  (CORE/encrypt.rs `check_encryptable_tracks`). Then: an MDSK key type for timed
  text, the key ids into the CPL/AuxData and the keys file, and a KDM covering them.
- Source fitting from the GUI without a crop (GUI pipeline.rs `job_geometry`): the
  create panel names a container, never a raster, so it forces a raster only when
  Fill container is ticked. A letterboxed HD source with 2K Scope selected and the
  box clear therefore still encodes at its own raster and the package is refused
  for declaring an active area wider than the frames, as it was before fitting
  landed. It needs a raster control of its own, or the container select split into
  raster plus active area the way the CLI's `--twok` and `--container` are.
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
- MPEG2 picture essence (DCP-o-matic 2.18 `VideoEncoding`, an alternative to J2K for
  legacy Interop gear). Every encode path here is grok J2K (postkit `pipeline` and
  `grok_encoder`) and the CPL and MXF writers assume a J2K picture descriptor, so
  this is a second essence type end to end rather than a flag. Only worth it if a
  customer actually has gear that needs it.
- TMS upload after build (DCP-o-matic config `tms_protocol` ftp or sftp, plus
  `tms_ip`, `tms_path`, `tms_user`, `tms_password` and `upload_after_make_dcp`):
  push the finished package to a theatre management system. Delivery here is drive
  copy (`copy`, `format-drive`), webhook and the REST API, all local or callback
  based. The credential half already has a pattern to copy: `email.rs` reads an SMTP
  config TOML whose password field redacts itself in Debug and never reaches argv or
  logs.
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
  so the colour conversion would have to happen before the pipe either way.
  GUI. The daemon queue (CORE/job_queue.rs) is a Mutex around an in-memory map with
  no save or load, so a daemon crash or reboot loses every pending job, and unlike
  DCP-o-matic, whose batch jobs are film projects on disk that can be re-added, our
  jobs carry their whole config as JSON params in memory, so losing the queue loses
  the specifications. The GUI is worse: its Jobs panel is a separate JobQueue in
  tauri state (gui pipeline.rs), never talks to the daemon (only `serve` proxies to
  it), and dies with the window. Both halves are mostly plumbing: Job is already
  Serialize/Deserialize, so persist to an XDG-dir JSONL on submit and state change
  and reload pending jobs on daemon start, and have the GUI submit over the existing
  IPC when the daemon is up, falling back to in-process when it is not.
- `--source-colourspace`: p3 and rec2020 encode as of PK 02d3002 / 8d128f2
  (PK `colour::DcdmTransform` per frame on the encoder threads, grok's transform
  off, verified against independent matrix math within one code value). Still
  refused: aces and acescg, correctly, they need a rendering transform (LUT), and
  logc, where "correctly" is softer: DCP-o-matic handles Sony S-Log3/S-Gamut3
  analytically (inverse log curve then matrix, libdcp `s_gamut3_to_xyz`), and ARRI
  LogC's EOTF is published math, so a transfer-function-ahead-of-matrix arm in
  DcdmTransform is the shape if a customer shows up with LogC masters. Image
  sequences also stay refused for p3/rec2020 (`encode_parallel` hands files to
  grk_compress, which only converts Rec.709), GUI-only surface. DCP-o-matic also
  allows fully custom conversions (user chromaticities, white point, gamma), a
  flexibility we do not have anywhere.
- Interop KDM (`kdm --format interop`) is legacy and unvalidated: no reference
  library generates Interop (libdcp only reads it) and the suite has no reference
  Interop KDM to diff against. Validate against real legacy gear before production.
  This one cannot be closed by testing: it needs hardware.
- conform gaps (the formats themselves are in DESIGN.md): AAF video is
  code-complete but untested against a real file, since libaaf's public test
  corpus has video tracks but no video clips. AAF pan and gain automation are
  surfaced in the timeline's skipped list but not applied, deliberate scope.
- Accessibility check is a real structural probe as of postkit c6406d1 (element and
  MCA-token evidence, three-state present/absent/undeterminable). Burned-in open
  captions and director commentary are undeterminable by construction: nothing in a
  package declares either.
- Sony RAW / X-OCN is detected but undecodable (ffmpeg can't decode it), same as
  ARRIRAW/R3D/BRAW/Canon: a match only yields a clearer detected-but-undecodable
  error. postkit's detect_format matches Sony's private essence ULs in the .mxf
  header. Caveat: those ULs are reverse-engineered from MediaInfo, NOT
  SMPTE-registered, and mark the Sony RAW family without distinguishing X-OCN
  ST/LT/XT tiers (fine, since the match only sharpens the error). Non-Sony .mxf
  still resolves to DNxHR.

- cosmic-text's bidi handling could replace the hand-rolled `--subtitle-rtl`
  reshaping.
- The standalone `burnin` command is now redundant for DCP work. `create
  --burn-subtitle` burns in one generation on every input shape, while `burnin` costs
  an extra lossy transcode and its `--font-size` is inert for subtitles (read only in
  the `drawtext` watermark branch). Worth deciding whether it stays as a plain
  video-to-video tool or goes.

### Batch E (easyDCP parity, surveyed 2026-08-16)

From en.easydcp.com easyDCP Plus and IMF Studio (both now EUR 3567.62 permanent or
EUR 164.22/month, so the README's "EUR 2,998" line is stale). Most of what those
pages advertise is already here. These five are not.

- GPU J2K encoding. Both easyDCP products sell GPU/CUDA acceleration, and
  DCP-o-matic reaches grok's GPU path (`dcpomatic2_cli list-gpus` and `config
  grok-licence`). Nothing here touches it: postkit `grok_encoder.rs` and `grok.rs`
  mention no device, no gpu and no cuda. The concept exists on the decode side only,
  where postkit `PlaybackOptions` carries a `gpu_device`. Worth checking what the
  pinned grok tag actually exposes through grok-ffi before scoping this.
- HD-SDI monitoring output. easyDCP Player+ and IMF Player both drive Blackmagic
  hardware. This is a port rather than new work: imfwizard already has `sdi-preview`,
  which runs a GStreamer decklink pipeline and probes for the plugin first
  (imfwizard-core `tools.rs`, `has_gst_decklink`). The open question is whether the
  embedded preview should feed it or it stays a separate command.
- JPEG and BMP still input. easyDCP takes DPX, TIFF, JPEG, BMP, PNG, J2K and
  QuickTime. `create` takes DPX, TIFF, EXR and PNG (EXR is ours, they have no
  equivalent). Small, and it belongs with whatever lands `--still-length`.
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

### Storm DCP Studio survey (2026-08-16)

From pixelstorm-media.com/pages/storm-dcp-studio (unreleased at survey time, v0.1,
every price and platform "coming soon", so screenshots not a shipping product).
Their Pro tier is roughly our existing feature set. Items marked "imfwizard too"
should also land there and are noted in its DESIGN_TODO.

- Playback overlays, decode resolution and HUD landed in guikit's preview header
  (safe area 95/90%, aspect mask 1.85/1.90/2.39, centre cross, thirds grid as one
  mpv `vf` chain; full/half/quarter through the J2K decoder's `lowres`, proven on
  real J2K with the mpv CLI; frame, fps, buffer depth and dropped frames in the
  metadata poll), the crop overlay and the subtitle/CC render toggles with them.
  Not yet clicked through in the running window. imfwizard too.
- QC report addition, in CORE report.rs on top of what dcpdoctor verifies:
  Leq(m) per ISO 21727 with the content-kind limits stated inline (advertisement
  <= 82, trailer <= 85). postkit `loudness::measure_leq_m` already measures it
  (CCIR 468 weighted) and dcpdoctor's `qc-report` prints it, dcpwizard's own
  report does not yet. Cinema-only. The codestream forensics half is served: `report` runs the verify
  with `scan_every_frame`, so dcpdoctor's `j2k_codestream_summary` line reaches
  the report's Info rows.
- Post-build actions. Done: a finished build shows Play / Inspect / Reveal beside
  the progress bar, wired to the embedded preview, the Verify view and the file
  manager. The row belongs to the build in front of you, so a job picked out of the
  Jobs list does not bring it back.
- Ident and rating-card library. A drag-drop library of head idents, tail idents,
  rating cards and anti-piracy clips joined onto the build. Real ad and trailer
  workflow, but it is conform/concatenation work, so a real lift. DCP-only.
- Encode log detail. The per-stage half is done: `[TIMING]` lines in the job log
  give preflight, encode, audio, packaging, validation and the total. What is still
  open is the breakdown inside an encode, colour convert against frame prep against
  the J2K encode itself: postkit's `PipelineProgress` carries a stage name, a frame
  count and an elapsed clock, and nothing that separates the three, so it would take
  a wider progress payload from postkit first. Their claim that the MXF write
  overlaps the encode is untouched here. imfwizard too.

## Done 2026-08-12

- `edit` drops the signature from every document it rewrites (the CPL, any PKL
  carrying its entry, and the ASSETMAP) instead of leaving one that no longer
  covers the bytes. A stale signature is worse than none: dcpdoctor reports
  `signature_invalid` and a verifier reads it as tampering, where an unsigned
  package only reads as unsigned. `package_signature::strip_signature` matches the
  element by local name, so a third-party `dsig:` prefix is caught too, and takes
  any `Signer` with it. The strip runs before the CPL is written, so the PKL keeps
  hashing what actually lands on disk. Warn-only, no new flag: the edit already
  mints a new composition id, so the output was never the signed artifact. Re-signing
  would need signer arguments `edit` does not take.
- GUI create panel, second batch: sign language, pad head/tail, and audio input
  order / filename channel routing now go through `submit_job` -> JobConfig ->
  run_job. Sign language calls `sign_language::build_slvs_sound` after the encode
  (frame count from the encode result, or the J2K dir when the input was already
  J2K) and carries the tag + leading channel count into DcpConfig. Pad head/tail
  and pad colour are parsed in `submit_job` (`pad::parse_pad_frames` /
  `parse_pad_color`) so a bad spec fails before the encode. A channel WAV
  directory routes through `audio_route::route_directory` before loudness, and the
  six-channel input order (dcp | lrc-ls-rs-lfe) reaches `DcpConfig`. run_job's
  config building was split out into `build_dcp_config` / `prepare_audio` /
  `frame_rate_of`, which the new pipeline.rs tests drive (the tauri command itself
  has no test seam). index.html gained Padding and Sign Language fieldsets plus the
  Audio channel-directory and channel-order controls; main.js added the browse
  handlers and submit_job args.
- GUI create panel, third batch: upmix, reel splitting, delivery profiles and
  versions/multi. Upmix (a|b) runs `postkit::upmix::upmix_wav` in prepare_audio
  between routing and loudness, same order as the CLI. Reel splitting carries
  `reel_length_minutes` and `reel_split_frames` into DcpConfig: the panel's
  timecodes parse in `submit_job` via `reel::parse_timecode`, chapter marks resolve
  at the top of run_job (ffprobe -> `reel::parse_chapter_starts`) so a source with
  no chapters fails before the encode, and the three split sources are mutually
  exclusive like the CLI's clap conflicts. Versions loads the manifest in
  `submit_job` (`versions::load_versions`, which validates it) and run_job picks
  `create_versioned_dcp` over `create_dcp`; a manifest plus a panel subtitle/CCAP is
  rejected, matching `conflicts_with = "versions"`, and so is a manifest plus explicit
  split points, since `create_versioned_dcp` reels by `reel_length_minutes` alone and
  would drop them (the CLI accepts that combination and ignores the splits). Profiles are a panel action, not
  a job field: a `list_profiles` command maps `profiles::all_profiles` to panel
  control values and picking one fills standard / resolution / frame rate /
  bandwidth / content kind, marks those controls, and names them in a hint; a later
  edit wins and clears the mark. The panel resolution-key <-> container table is now
  one const shared by `build_dcp_config` and the profile mapping. Reading a source's
  chapters duplicates ~10 lines of ffprobe invocation from the CLI (both sides parse
  with the same core function); worth moving to core if a third caller appears.

## Done 2026-07-23

App-side feature batch (create surface, KDM, conform, GUI). All items have tests.

- Credentialed vendor cert download (dom#2705/2706): `cert-fetch` gained
  christie/gdc/barco with `--user`/`--password`. Path builders (christie 12-digit
  zero-pad + F-IMB->IMB-S2 fallback, gdc /SHA256, barco 10-char + first7-xxx dir)
  are unit-tested. Credentials go to curl via a stdin config (`-K -`), never argv
  or logs. Anonymous dolby/qube paths unchanged. No config-file storage exists for
  cert-fetch, so creds are CLI-only.
- DCI HDR Addendum (dom#2374/2799): `create --hdr-dci` authors an HDR DCP. The
  picture MXF is wrapped through asdcplib `jp2k::open_write_hdr`
  (CORE/mxf_wrap.rs `wrap_j2k_hdr_files`), setting TransferCharacteristic=ST 2084
  (PQ) + ColorPrimaries=P3-D65 on the essence descriptor. Validates the flag combo
  (needs `--hdr-to-dci-lut` or `--hdr-already-pq`) and the raised per-codestream cap
  up front; fails loud with 3D or reel splitting. Roundtrip test reads the descriptor
  back and asserts both ULs.
- Closed captions (CCAP, ST 428-10/429-12): `create --ccap <file>` wraps timed text
  with a MainClosedCaption role, carried through every CPL path: single-reel
  (CORE/cpl.rs + CORE/dcp.rs), reel splitting (reel.rs), versions (a `ccap` manifest
  field, versions.rs), and VF (vf.rs, `--add-ccap`/`--replace-ccap` REEL=PATH). Same
  input formats as `--subtitle`. Tests mirror the subtitle-path tests.
- conform full assembly: `conform --media-dir <dir> --output <dir>` resolves every
  EDL/xmeml reel to media (fails loud on unresolved reels), then drives the reel plan
  to a finished multi-reel DCP (per-reel grok encode + MXF wrap via create_dcp, then
  assemble.rs CPL assembly). conform_plan.json + the postkit conform manifest stay as
  artifacts. Gated e2e test builds a 2-reel DCP from a tiny EDL over synthetic media
  and verifies it with dcpdoctor.
- Trailer: `trailer` now encodes the packaged mp4 to J2K and builds a real trailer
  DCP (ContentKind=trailer) in `<output>/dcp` via the grok encode + create_dcp path.
- Markers: `markers --marker LABEL=timecode` (repeatable) places any of the ten
  defined markers, validating the label and the offset (frame or HH:MM:SS:FF) against
  the composition length. Default set stays FFOC/LFOC.
- ingest `--lut`: threads a 3D LUT through postkit ingest (ffmpeg lut3d). Was
  hardcoded false.
- KDM `--annotation`: CLI flag -> postkit `KdmConfig.annotation` (CORE/kdm.rs). None
  keeps the derived "<title> KDM for <recipient>" text. Test asserts the override
  lands, escaped, in the KDM XML.
- Colour `--target p3-d65`: routes through postkit `DcdmTarget::P3D65` alongside the
  xyz branch (CLI `parse_dcdm_target`), both through the real DCDM transform. Unit
  test covers the mapping. (grok/postkit RGB->X'Y'Z' harmonization landed earlier in
  postkit 32838ea; colour.rs tests assert agreement with grok's [2817,2183,870].)
- Job queue: create jobs report coarse stage progress (dcp::ProgressSink) instead of
  0->100; cancel affects running jobs (per-job AtomicBool checked in the job loop +
  between create_dcp stages, worker on its own thread); `serve` proxies every job
  route to the shared daemon queue over IPC (one queue) and returns 503 when the
  daemon is down.
- combine.rs dedup: dropped the local `inject_annotation` string-splice; the merged
  PKL/ASSETMAP carry AnnotationText via the postkit packaging fields
  (`generate_pkl`/`generate_assetmap` gained an `annotation` arg). Output stays
  byte-identical (combine tests pass). `combine --annotation` exposes the override.
- DTS:X: the mxf_wrap essence-type comment and the docs point DTS:X at the IAB
  (`--atmos`, ST 429-18) path. There is no separate DTS:X CLI surface. Rationale: no
  public DTS:X DataEssenceCoding UL exists (SMPTE registers have only DTS private
  nodes; asdcplib/libdcp carry nothing). Since ST 429-18/-19 (2019), DTS:X is
  delivered as a standard IAB track per ST 2098-2 ("DTS:X for IAB").
- GUI create panel: the tauri `submit_job` -> JobConfig -> run_job path carries
  right-eye 3D (second run_encode_with_ratio into `<output>/right/j2k`, stereo_3d
  derived from its presence), Atmos track, subtitle file + language, CCAP + language,
  and loudness target + true-peak ceiling (loudness::adjust_loudness on the WAV before
  wrapping). content kind / bandwidth / encryption were already wired. index.html
  gained the 3D / Audio / Subtitles-&-Captions fieldsets; main.js added the browse
  handlers + submit_job args. `--hdr-dci` deliberately skipped (see Open). Fixed in
  passing: gui create_vf built a ReplacementReel without the `ccap` field added this
  batch (the gui crate did not compile at 209a83d); set `ccap: None`.

## Keep in sync with imfwizard (deliberately duplicated, no clean shared home)

The shared *logic* already lives in postkit (mpv::MpvPlayer, packaging writers,
escape_xml, parse_srt, pipeline::run_encode). What remains duplicated is app/framework
glue with no clean cross-repo home, left as copies. If you edit one side, mirror the
other:

- gui/src-tauri/src/preview_server.rs and preview_surface.rs — moved to the guikit
  crate 2026-08-13, no longer duplicated. Both wizards depend on
  extern/guikit/rust and register its commands. It did not go to postkit, which
  must stay free of a tauri dep since the CLI and wasm use it too.
- gui/src/preview.js — moved to guikit 2026-08-13, no longer duplicated. Both
  wizards import extern/guikit/src/preview.js from their own gui/src.
- gui/vite.config.js — still per-app, only partially aligned: the dev port differs,
  and consuming guikit added a server.fs.allow plus a resolve alias pointing its
  bare @tauri-apps imports at gui/node_modules, since guikit sources sit outside
  the vite root. Mirror any other change.
- gui/src/shortcuts.js — moved to guikit 2026-08-13 for the two wizards, which
  import extern/guikit/src/shortcuts.js. dcpdoctor is not a submodule consumer
  and keeps a vendored copy synced by plain cp, so a guikit change to this file
  still needs a manual copy into dcpdoctor. App-agnostic by design: all app
  specifics enter through initShortcuts, never a per-repo edit.
- gui/src/timeline.js — classified 2026-08-13 as genuine domain difference, not
  drift, so it stays per-app deliberately and is not a guikit candidate. It is a
  thin renderer over disjoint backend structs: dcpwizard's TimelineEntry reels
  against imfwizard's SegmentEntry segments. Unifying would mean a field-mapping
  layer larger than the duplication.
- gui/src-tauri/src/lib.rs, gui/src-tauri/src/pipeline.rs — app-specific tauri setup
  and build orchestration; they delegate the encode to postkit::pipeline but diverged
  enough that unifying would need per-divergence config flags. The 2026-07-23
  create-panel wiring edited dcpwizard pipeline.rs (right-eye 3D via a second
  run_encode_with_ratio, atmos_path, subtitle + language, ccap + language, loudness
  normalize before wrap) and main.js/index.html (new pickers). imfwizard already
  submits compositions with subtitles/audio; the atmos + loudness-normalize-before-wrap
  and single-DCP 3D right-eye bits are dcpwizard-specific (IMF has no atmos aux track /
  stereoscopic DCP concept), so nothing to mirror unless imfwizard adds a loudness step.
  The 2026-08-12 batches added sign language, pad head/tail/colour, filename channel
  routing, reel splitting, delivery profiles and versions/multi to the same path; those
  sit on dcpwizard-core (sign_language, pad, audio_route, reel, profiles, versions), so
  there is nothing to mirror there either. Upmix is postkit::upmix and would port to
  imfwizard unchanged if its panel ever wants it. The HDR batch is the same shape:
  the shared half (source colour path, codestream cap) is in postkit::pipeline /
  postkit::encode where imfwizard picks it up by bumping its pin, and the DCI HDR
  Addendum half is dcpwizard-only, since IMF carries HDR in its own descriptors and
  has no DCI addendum, so nothing to mirror.
- .github/workflows/ci.yml, release.yml, gui-release.yml — copies across dcpwizard,
  imfwizard, dcpdoctor differing by binary/artifact names + per-app build deps.
  Separate git repos, so no shared reusable-workflow without a central repo. Keep
  aligned by hand. Every job that compiles the rust workspace has a cached "Setup grok"
  step that builds grok v20.3.8 from source, installs to $GITHUB_WORKSPACE/grok-install,
  and exports PKG_CONFIG_PATH/LD_LIBRARY_PATH via $GITHUB_ENV (actions/cache keyed on
  grok tag + runner os). Windows uses a separate msvc build of the same tag. All three
  platforms are required, none are continue-on-error.
  imfwizard mirrors the step but only in ci (it runs grk_compress at runtime, does not
  link grok-ffi); dcpdoctor needs no grok.
- tests/cli_flags_test.sh — NOT the same harness as imfwizard's (this one runs the
  binary and checks clap parse errors; imf parses main.js). Different CLIs, leave
  separate.
