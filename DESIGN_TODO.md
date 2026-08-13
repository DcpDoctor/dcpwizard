# DESIGN_TODO

Paths: CORE = rust/crates/dcpwizard-core/src, CLI = rust/crates/dcpwizard-cli/src/main.rs,
PK = extern/postkit (postkit submodule; bump the pin when postkit changes).
DoM refs (dom#N = https://dcpomatic.com/bugs/view.php?id=N) are DCP-o-matic tracker
feature requests. Shared DSP/parsers belong in postkit (see its DESIGN_TODO); the
user-facing surface is here.

## Open

- guikit phase 2 (phase 1 landed 2026-08-13): preview_server.rs and
  preview_surface.rs into a small crate in guikit, consumed as a path dep through
  the extern/guikit submodule (not postkit, which must stay free of a tauri dep).
  Gated on phase 1 surviving one real change cycle, meaning a guikit edit that
  reaches both wizards through a pin bump. Subsumes the preview_server.rs entry
  in "Keep in sync" below.
- postkit compiled twice, fixed 2026-08-12. `postkit` was a path dep on
  `extern/postkit` while `dcpdoctor-core` came from git carrying its own path dep
  on the postkit inside that checkout, so cargo resolved two copies. dcpdoctor
  declares postkit by git now, and `rust/Cargo.toml` and `gui/src-tauri/Cargo.toml`
  each carry a `[patch]` redirecting that git source at `extern/postkit`, so both
  references collapse onto the submodule this workspace already builds. The
  edit-the-submodule-and-rebuild loop is unchanged and `cargo tree -d` reports no
  postkit duplicate. imfwizard has the same shape.
- Distributed encoding across machines (dom#155, dom#1635, dom#2605). Out of scope
  (user-excluded). The job queue is single-machine and its create path wraps
  pre-encoded J2K rather than running postkit::pipeline, so job progress is
  stage-based, not per-frame.
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

- gui/src-tauri/src/preview_server.rs — near-identical (only the MpvPlayer app name
  differs). NOT moved to postkit: it is all `#[tauri::command]` wrappers and postkit
  has no tauri dep (also used by the CLI and wasm). The reusable part (MpvPlayer) is
  already in postkit::mpv. dcpwizard also keeps a windows preview_server_stub the imf
  side lacks. Linux builds with the embedded-preview feature draw the preview in
  the app window via the libmpv render API (state in postkit's DESIGN_TODO, shared
  with imfwizard); other platforms and builds without the feature spawn a separate
  mpv window.
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
