# DESIGN_TODO

Paths: CORE = rust/crates/dcpwizard-core/src, CLI = rust/crates/dcpwizard-cli/src/main.rs,
PK = extern/postkit (postkit submodule; bump the pin when postkit changes).
DoM refs (dom#N = https://dcpomatic.com/bugs/view.php?id=N) are DCP-o-matic tracker
feature requests. Shared DSP/parsers belong in postkit (see its DESIGN_TODO); the
user-facing surface is here.

## Open

- Cross-platform embedded preview: all three hosts are implemented in guikit
  (linux GtkGLArea verified live, macos NSOpenGLView layered over the WKWebView,
  windows WS_CHILD window with wgl over the WebView2 child), pinned here and in
  imfwizard, and CI compiles all three platforms green. Remaining: neither the
  macos nor the windows host has run on real hardware, so a hand pass there is
  the last step. The platform contract stays three items: `attach(&tauri::Window)
  -> Result<EmbeddedPreview, String>`, `EmbeddedPreview::player()` and
  `EmbeddedPreview::set_surface(x, y, w, h, visible)`, identical on every
  platform.
- Windows release builds: grok is wired into release.yml and gui-release.yml as
  of 2026-08-14 (the msvc setup step ported verbatim from ci.yml, the cli zip
  ships grokj2k.dll beside the exe, and tauri.windows.conf.json bundles it into
  the installer next to the exes). Unproven until the next tag run. Watch on
  that run: if grok's msvc install drops more dlls that grokj2k.dll depends on,
  copy bin/*.dll in both places instead, and a local windows tauri build now
  fails at bundle time unless the dll is staged at gui/src-tauri/grokj2k.dll.
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
- Automatic source fitting for `create --twok/--fourk` (PK grok_encoder.rs): the
  ffmpeg decode has no scale/pad filter, so the source raster must already equal the
  encode raster and a mismatch is refused (CORE/encode.rs `check_encode_raster`).
  Fitting means scaling to the container preserving aspect and padding with black in
  the decode command, then dropping the guard.
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
- The job queue does not survive a restart, and the GUI's queue does not survive the
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
- `--source-colourspace` accepts postkit's full ColourSpace set but only rec709 and
  xyz encode; p3, rec2020, aces, acescg and logc are refused at runtime, pointing at
  `colour --target xyz` as the separate pass. The in-encode transform they need does
  not exist in a reachable form: PK `colour::convert_colour` refuses any X'Y'Z'
  target without a 3D LUT, and PK `dcdm` has the real P3/Rec.2020 matrices but only
  behind `create_dcdm`, which writes a TIFF sequence. Closing this means a public
  in-memory per-frame transform in postkit (its `dcdm` internals exposed, or a
  generalised `rgb_to_xyz_inplace(buf, space)`), applied with grok's transform off.
  The one ffmpeg route (P3 -> Rec.709 -> grok) silently clips out-of-gamut colour
  and was rejected: a wrong picture is worse than a refusal.
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

- Burn-in costs a whole extra generation and ignores its own styling flag. Today
  `burnin` is a standalone video-to-video pass, so burning subtitles means transcode
  once to burn and again inside `create`, for two lossy generations where the
  encode already decodes every frame through ffmpeg. The obvious fix is a burn option
  on `create` appending the subtitles filter to the decode chain the encode already
  builds, but that only covers one of the three input shapes. PK pipeline.rs matches
  on input type and only `InputType::Video` reaches ffmpeg
  (`stream_encode_inprocess`). `InputType::ImageSequence` goes to `encode_parallel`,
  which reads TIFF/DPX/EXR/PNG natively through `grok::load_tiff`, and a J2K directory
  passes straight through. So an ffmpeg filter leaves a directory of TIFFs, and a held
  still, with no burn at all.
  DCP-o-matic has no such hole because its burn never touches the decoder:
  `render_text` rasterises cues to RGBA bitmaps with positions, the player merges them
  per frame, and `PlayerVideo::image` burns them with one `alpha_blend` onto the
  decoded buffer (src/lib/render_text.h, player.cc, player_video.cc). Compositing at
  the frame buffer is decoder-agnostic, so it covers video, an image sequence, a held
  still and a DCP re-used as content, all the same way.
  Doing it that way needs a text rasteriser, which postkit does not have (font_subset
  subsets glyphs, nothing draws them). Crate settled 2026-08-16: cosmic-text 0.17
  (default-features off, swash), the stack glass2glass g2g-plugins already uses, and
  its textoverlay.rs is the design reference (shaped horizontal cues via cosmic-text,
  ab_glyph column renderer for vertical, RGBA8 in / painted text out). Its bidi
  handling could later replace the hand-rolled `--subtitle-rtl` reshaping. It buys two things at once: burn-in on every
  input shape, and the subtitle appearance controls (outline, shadow, effect colour,
  outline width) that cannot work through ffmpeg's `subtitles` filter at all, since
  that filter takes styling from the subtitle file rather than from our flags. So the
  rasteriser is the real item and the ffmpeg filter is a stopgap for video input.
  Two knock-ons either way: a burnt-in subtitle must not also
  register a timed-text track in the CPL, and the ISDCF name spells a burnt-in
  subtitle language in lower case where an open one is upper case, so whatever lands
  ISDCF naming needs to know which it is.
  Three defects in the current path while it exists, all in PK burnin.rs:
  `font_size`, `font_colour` and `position` are read only in the `drawtext` branch
  used for text watermarks, so `burnin --font-size` is inert for subtitles, which is
  the command's only styling flag; the CLI advertises "SRT, ASS, or SMPTE XML" but
  ffmpeg's `subtitles` filter has no ST 428-7 DCST reader, so that input dies inside
  ffmpeg instead of being refused up front; and the path goes into the filter string
  unescaped, where `:` and `,` and `\` are ffmpeg's own separators.

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
