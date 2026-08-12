use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

/// W5 create-time audio + encode QoL options, boxed into the Create variant.
#[derive(Args)]
struct CreateAudioQol {
    /// Normalize audio to a loudness target before wrapping (dom#1382):
    /// leqm=<db> (ISO 21727 Leq(m)) or lufs=<value> (EBU R128 integrated).
    #[arg(long)]
    loudness_target: Option<String>,
    /// True-peak ceiling in dBTP for --loudness-target (default -1.0). The
    /// gain is refused loud if it would breach this.
    #[arg(long)]
    true_peak_ceiling: Option<f64>,
    /// Upmix stereo audio to 5.1 before wrapping (dom#921/#1080): variant
    /// a (band-split) or b (passthrough + delayed surrounds).
    #[arg(long, value_parser = ["a", "b"])]
    upmix: Option<String>,
    /// Wait until this wall-clock time before encoding (dom#2359): HH:MM,
    /// an RFC 3339 timestamp, or a +offset (+30m, +2h).
    #[arg(long)]
    start_at: Option<String>,
    /// Resume an interrupted encode, reusing J2K frames already on disk
    /// (dom#344). Requires the same source and settings as the first run.
    #[arg(long)]
    resume: bool,
    /// Power the machine off after a successful encode (dom#1394). Fails
    /// loud up front if no shutdown command is available.
    #[arg(long)]
    shutdown_when_done: bool,
}

/// W6 subtitle placement / RTL / wrap / font options, boxed into the Create
/// variant so it stays under the clippy large-variant threshold.
#[derive(Args)]
struct CreateSubtitleOpts {
    /// Subtitle horizontal alignment: left, center, or right (default center)
    #[arg(long, value_parser = ["left", "center", "right"])]
    subtitle_halign: Option<String>,
    /// Subtitle vertical anchor: top, center, or bottom (default bottom)
    #[arg(long, value_parser = ["top", "center", "bottom"])]
    subtitle_valign: Option<String>,
    /// Subtitle vertical position: percent from the valign edge (default 8)
    #[arg(long)]
    subtitle_vposition: Option<f64>,
    /// 3D subtitle depth: SMPTE Zposition emitted on every cue (stereoscopic)
    #[arg(long)]
    subtitle_zposition: Option<f64>,
    /// RTL subtitle reordering: auto, on, or off (default auto)
    #[arg(long, default_value = "auto", value_parser = ["auto", "on", "off"])]
    subtitle_rtl: String,
    /// Auto-wrap subtitle lines longer than this many characters
    #[arg(long)]
    subtitle_wrap: Option<usize>,
    /// TTF/OTF font to embed in the subtitle track (subset to used glyphs)
    #[arg(long)]
    subtitle_font: Option<String>,
    /// Embed the whole font instead of subsetting it to the used glyphs
    #[arg(long)]
    subtitle_no_subset: bool,
    /// Closed-caption (ST 429-12) input, wrapped with a MainClosedCaption role
    /// (accessibility track, distinct from --subtitle). SRT/styled or SMPTE DCST.
    #[arg(long, conflicts_with = "versions")]
    ccap: Option<String>,
    /// Closed-caption language code (e.g. "en", "fr")
    #[arg(long, default_value = "en")]
    ccap_language: String,
}

/// CPL/PKL signing identity, boxed into the Create variant so it stays under
/// the clippy large-variant threshold. Absent leaves the package unsigned.
#[derive(Args)]
struct CreateSignerOpts {
    /// Signer leaf certificate file. With --signer-key, signs the CPL and PKL.
    #[arg(long, requires = "signer_key")]
    signer_cert: Option<String>,
    /// Signer private key file, required with --signer-cert
    #[arg(long, requires = "signer_cert")]
    signer_key: Option<String>,
    /// Signer CA certificate above the leaf (repeatable: intermediate(s) then root)
    #[arg(long, requires = "signer_cert")]
    signer_chain: Vec<String>,
}

/// The signer for a `create`, or None when no --signer-cert was given. clap's
/// `requires` guarantees the key is present whenever the certificate is.
fn create_package_signer(
    opts: &CreateSignerOpts,
) -> Option<dcpwizard_core::package_signature::PackageSigner> {
    let cert = opts.signer_cert.as_ref()?;
    let key = opts
        .signer_key
        .as_ref()
        .expect("clap requires --signer-key alongside --signer-cert");
    Some(dcpwizard_core::package_signature::PackageSigner {
        signer_cert: PathBuf::from(cert),
        signer_key: PathBuf::from(key),
        signer_chain: opts.signer_chain.iter().map(PathBuf::from).collect(),
    })
}

#[derive(Parser)]
#[command(
    name = "dcpwizard",
    version,
    about = "DCP Wizard — Digital Cinema Package creator"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new DCP
    Create {
        /// DCP title
        #[arg(short, long)]
        title: String,
        /// Video file (mp4/mov/mkv) or J2K/image sequence directory
        #[arg(long)]
        video: String,
        /// Audio WAV file
        #[arg(long)]
        audio: Option<String>,
        /// Six-channel WAV order: dcp (L,R,C,LFE,Ls,Rs) or lrc-ls-rs-lfe
        #[arg(long, default_value = "dcp")]
        audio_input_order: String,
        /// HDR-to-DCI 3D LUT. Required for HDR source video unless generic tone mapping is enabled.
        #[arg(long)]
        hdr_to_dci_lut: Option<String>,
        /// Allow generic FFmpeg HDR tone mapping. It is not a delivery transform.
        #[arg(long)]
        allow_generic_hdr_tonemap: bool,
        /// Author a DCI HDR Addendum DCP (ST 2084 PQ / P3-D65 stamped on the
        /// picture MXF). Requires --hdr-to-dci-lut or --hdr-already-pq. Not
        /// supported with 3D or reel splitting.
        #[arg(long)]
        hdr_dci: bool,
        /// Acknowledge the source is already ST 2084 PQ (DCI HDR), so --hdr-dci
        /// needs no LUT conversion.
        #[arg(long)]
        hdr_already_pq: bool,
        /// Sign-language video (ISDCF Doc 13): encoded to VP9 and packed onto
        /// channel 15 of the sound track. Requires --sign-language-lang.
        #[arg(long, requires = "sign_language_lang")]
        sign_language_video: Option<String>,
        /// RFC 5646 sign-language tag for --sign-language-video (e.g. sgn-ase).
        #[arg(long)]
        sign_language_lang: Option<String>,
        /// SRT file to convert, or supplied SMPTE subtitle XML to package unchanged
        #[arg(long, conflicts_with = "versions")]
        subtitle: Option<String>,
        /// Multi-version manifest (JSON array): one CPL per entry over shared essence
        #[arg(long)]
        versions: Option<String>,
        /// Subtitle language code (e.g. "en", "fr")
        #[arg(long, default_value = "en")]
        subtitle_language: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// DCP standard (smpte|interop)
        #[arg(long, default_value = "smpte")]
        standard: String,
        /// Delivery profile
        #[arg(long)]
        profile: Option<String>,
        /// Encrypt the DCP
        #[arg(long)]
        encrypt: bool,
        /// Where to write the content keys (required with --encrypt). Holds the
        /// plaintext AES keys: point it outside the DCP and keep it secret.
        #[arg(long, required_if_eq("encrypt", "true"))]
        key_out: Option<String>,
        /// Content type: FTR, SHR, TLR, TST, XSN, RTG, TSR, POL, PSA, ADV
        #[arg(long)]
        content_type: Option<String>,
        /// DCP frame rate (auto-detected from source if not specified)
        #[arg(long)]
        frame_rate: Option<u32>,
        /// Force 2K resolution
        #[arg(long)]
        twok: bool,
        /// Force 4K resolution
        #[arg(long)]
        fourk: bool,
        /// Picture container: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, or 4k-full
        #[arg(long)]
        container: Option<String>,
        /// Number of encoding threads (default: auto-detect CPU count)
        #[arg(short = 'j', long)]
        threads: Option<u32>,
        /// J2K bandwidth in Mbit/s (default: 250 for 2K, 500 for 4K)
        #[arg(long)]
        video_bit_rate: Option<u32>,
        /// Split into reels of at most N minutes each (default: single reel)
        #[arg(long)]
        reel_length: Option<u32>,
        /// Right-eye video/J2K for a stereoscopic 3D DCP (main input is left eye)
        #[arg(long)]
        right_eye: Option<String>,
        /// Dolby Atmos / DCData bitstream to wrap as a ST 429-18 auxiliary track
        #[arg(long)]
        atmos: Option<String>,
        /// Sound channel index (0-based) carrying the Hearing Impaired (HI) track
        #[arg(long)]
        hi_channel: Option<u32>,
        /// Sound channel index (0-based) carrying the Visually Impaired (VI-N) track
        #[arg(long)]
        vi_channel: Option<u32>,
        /// Pad the head with black frames + silence. Duration with a unit:
        /// frames (48f) or seconds (2s). Shifts subtitles by the same offset.
        #[arg(long)]
        pad_head: Option<String>,
        /// Pad the tail with black frames + silence. Same syntax as --pad-head.
        #[arg(long)]
        pad_tail: Option<String>,
        /// Background/pad colour as hex sRGB (RRGGBB or #RRGGBB). Default: black.
        /// Applied to head/tail pad frames via the DCDM colour transform.
        #[arg(long)]
        pad_color: Option<String>,
        /// Custom container dimensions WxH (e.g. 1920x1080). Must be even and fit
        /// within the 2K (2048x1080) or 4K (4096x2160) container. Overrides --container.
        #[arg(long, conflicts_with = "container")]
        container_dims: Option<String>,
        /// Split into reels at these timecodes (comma-separated HH:MM:SS or
        /// HH:MM:SS:FF). Each reel must be >= 1s. Conflicts with --reel-length.
        #[arg(long, conflicts_with = "reel_length")]
        split_at: Option<String>,
        /// Split into reels at the source video's chapter marks (ffprobe).
        /// Conflicts with --reel-length and --split-at.
        #[arg(long, conflicts_with_all = ["reel_length", "split_at"])]
        split_chapters: bool,
        /// Force the ffmpeg decode range: full or legal. Corrects wrong or missing
        /// source range metadata (video input only).
        #[arg(long, value_parser = ["full", "legal"])]
        input_range: Option<String>,
        // boxed so the Create variant stays small (clippy large_enum_variant).
        #[command(flatten)]
        audio_qol: Box<CreateAudioQol>,
        #[command(flatten)]
        subtitle_qol: Box<CreateSubtitleOpts>,
        #[command(flatten)]
        signer_opts: Box<CreateSignerOpts>,
    },
    /// Rebuild ASSETMAP and PKL to cover every asset file present (metadata-only
    /// repackaging; no re-wrap or re-encode). For re-ingesting exported OV/VF
    /// folders whose ASSETMAP/PKL omit hardlinked assets.
    IngestPackage {
        /// DCP package directory to repackage in place
        dir: String,
    },
    /// Combine several complete DCPs into one delivery volume with a merged
    /// ASSETMAP/VOLINDEX (and, by default, a single merged PKL). CPLs and essence
    /// are copied byte-identical, so signatures/hashes stay valid.
    Combine {
        /// Input DCP directories (two or more)
        #[arg(required = true, num_args = 1..)]
        inputs: Vec<String>,
        /// Output volume directory
        #[arg(short, long)]
        output: String,
        /// Keep each input's PKL as its own file instead of one merged PKL
        #[arg(long)]
        separate_pkls: bool,
        /// Order CPL entries alphabetically by content title
        #[arg(long)]
        sort: bool,
        /// AnnotationText for the merged PKL/ASSETMAP (default: derived from titles)
        #[arg(long)]
        annotation: Option<String>,
    },
    /// Create a supplemental Version File (VF) DCP against an Original Version
    CreateVf {
        /// Original Version (OV) DCP directory
        #[arg(long)]
        ov: String,
        /// Output VF directory
        #[arg(short, long)]
        output: String,
        /// VF title (defaults to "<OV title>_VF")
        #[arg(short, long, default_value = "")]
        title: String,
        /// Replace a reel's picture essence: --replace-picture REEL=PATH (repeatable)
        #[arg(long = "replace-picture", value_name = "REEL=PATH")]
        replace_picture: Vec<String>,
        /// Replace a reel's sound essence: --replace-sound REEL=PATH (repeatable)
        #[arg(long = "replace-sound", value_name = "REEL=PATH")]
        replace_sound: Vec<String>,
        /// Replace a reel's subtitle: --replace-subtitle REEL=PATH (SRT or SMPTE XML)
        #[arg(long = "replace-subtitle", value_name = "REEL=PATH")]
        replace_subtitle: Vec<String>,
        /// Add a subtitle to a reel that has none: --add-subtitle REEL=PATH
        #[arg(long = "add-subtitle", value_name = "REEL=PATH")]
        add_subtitle: Vec<String>,
        /// Replace a reel's closed caption: --replace-ccap REEL=PATH (SRT or SMPTE XML)
        #[arg(long = "replace-ccap", value_name = "REEL=PATH")]
        replace_ccap: Vec<String>,
        /// Add a closed caption to a reel that has none: --add-ccap REEL=PATH
        #[arg(long = "add-ccap", value_name = "REEL=PATH")]
        add_ccap: Vec<String>,
        /// Language code for wrapped subtitle tracks
        #[arg(long, default_value = "en")]
        subtitle_language: String,
    },
    /// Assemble a new OV composition from existing DCPs: one new CPL whose reels
    /// are the inputs' reels in order. Essence is copied byte-identical and
    /// referenced by its existing UUIDs. Inputs must share standard/rate/
    /// resolution and must not be encrypted.
    Assemble {
        /// Input DCP directories (two or more), in program order
        #[arg(long = "input", required = true, num_args = 1..)]
        input: Vec<String>,
        /// Output OV directory
        #[arg(short, long)]
        output: String,
        /// Title for the assembled composition
        #[arg(short, long, default_value = "")]
        title: String,
    },
    /// Edit a DCP's CPL metadata (title/annotation/content-kind/issuer) without
    /// re-wrapping essence. Assigns a new CPL id and refreshes PKL/ASSETMAP.
    /// Refuses encrypted DCPs (the KDM binds the CPL id).
    Edit {
        /// DCP directory to edit
        #[arg(long)]
        input: String,
        /// Write the edited DCP here (copied first); omit to edit in place
        #[arg(short, long)]
        output: Option<String>,
        /// New content title
        #[arg(long)]
        title: Option<String>,
        /// New CPL AnnotationText
        #[arg(long)]
        annotation: Option<String>,
        /// New content kind (abbrev FTR/TLR/... or a raw kind string)
        #[arg(long)]
        content_kind: Option<String>,
        /// New Issuer
        #[arg(long)]
        issuer: Option<String>,
    },
    /// Build a multi-composition DCP: one CPL per manifest entry, each with its
    /// own j2k_dir/audio/subtitle, over one shared PKL/ASSETMAP. Contrast
    /// `create --versions` (multiple CPLs over shared essence).
    CreateMulti {
        /// Compositions manifest (JSON array): each entry names title, j2k_dir,
        /// and optional audio/subtitle/subtitle_language/kind
        #[arg(long)]
        compositions: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// DCP standard (smpte|interop)
        #[arg(long, default_value = "smpte")]
        standard: String,
        /// DCP frame rate
        #[arg(long, default_value = "24")]
        frame_rate: u32,
        /// Force 4K resolution (default 2K)
        #[arg(long)]
        fourk: bool,
        /// Picture container: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full
        #[arg(long)]
        container: Option<String>,
        /// Default subtitle language code for entries that omit one
        #[arg(long, default_value = "en")]
        subtitle_language: String,
        /// Default content type abbrev (FTR, TLR, ...) for entries that omit kind
        #[arg(long)]
        content_type: Option<String>,
        /// Encrypt the DCP
        #[arg(long)]
        encrypt: bool,
        /// Where to write the content keys (required with --encrypt)
        #[arg(long, required_if_eq("encrypt", "true"))]
        key_out: Option<String>,
    },
    /// Encode images to JPEG 2000
    Encode {
        /// Input image directory
        #[arg(short, long)]
        input: String,
        /// Output J2K directory
        #[arg(short, long)]
        output: String,
        /// Target bitrate (Mbps)
        #[arg(long, default_value = "250")]
        bandwidth: u32,
    },
    /// Full pipeline: video → J2K → DCP (streaming, no intermediate files)
    Pipeline {
        /// Input video file (or image/J2K directory)
        #[arg(short, long)]
        input: String,
        /// DCP title
        #[arg(short, long)]
        title: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// Audio WAV file
        #[arg(long)]
        audio: Option<String>,
        /// Compression ratio (default: 10)
        #[arg(long, default_value = "10")]
        ratio: f64,
        /// Frame rate (default: 24)
        #[arg(long, default_value = "24")]
        fps: u32,
        /// Force the ffmpeg decode range: full or legal. Corrects wrong or missing
        /// source range metadata.
        #[arg(long, value_parser = ["full", "legal"])]
        input_range: Option<String>,
        /// Split into reels at the source video's chapter marks (ffprobe).
        #[arg(long)]
        split_chapters: bool,
    },
    /// Transcode video to image sequence
    Transcode {
        /// Input video file
        #[arg(short, long)]
        input: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// Image format: tiff (default), dpx, exr, png
        #[arg(long, default_value = "tiff")]
        format: String,
        /// Bit depth: 16 (default), 10, or 8
        #[arg(long, default_value = "16")]
        bit_depth: u32,
    },
    /// Re-encode an existing DCP's picture essence to a lower bandwidth
    TranscodeDcp {
        /// Input DCP directory
        #[arg(short, long)]
        input: String,
        /// Output DCP directory (must differ from input)
        #[arg(short, long)]
        output: String,
        /// Target picture bandwidth in Mbit/s
        #[arg(long)]
        video_bit_rate: u32,
        /// Optional target width (with --height, rescales the picture)
        #[arg(long)]
        width: Option<u32>,
        /// Optional target height (with --width, rescales the picture)
        #[arg(long)]
        height: Option<u32>,
        /// KDM XML to decrypt an encrypted source (needs --recipient-key)
        #[arg(long)]
        kdm: Option<String>,
        /// Recipient RSA private key (PEM) matching --kdm
        #[arg(long)]
        recipient_key: Option<String>,
        /// dcpwizard KEYS.json, an alternative key source to --kdm
        #[arg(long)]
        keys: Option<String>,
    },
    /// Decrypt an encrypted DCP into a cleartext DCP with the same structure
    Decrypt {
        /// Input (encrypted) DCP directory
        #[arg(short, long)]
        input: String,
        /// Output DCP directory (must differ from input)
        #[arg(short, long)]
        output: String,
        /// KDM XML (needs --recipient-key)
        #[arg(long)]
        kdm: Option<String>,
        /// Recipient RSA private key (PEM) matching --kdm
        #[arg(long)]
        recipient_key: Option<String>,
        /// dcpwizard KEYS.json, an alternative key source to --kdm
        #[arg(long)]
        keys: Option<String>,
    },
    /// Verify an existing DCP
    Verify {
        /// DCP directory
        dcp_dir: String,
        /// Skip asset hash verification
        #[arg(long)]
        no_hash_check: bool,
        /// Skip picture bitstream checks (faster)
        #[arg(long)]
        no_picture_check: bool,
        /// Require strict SMPTE Bv2.1 compliance
        #[arg(long)]
        strict: bool,
        /// Write report to file (.txt or .html)
        #[arg(short, long)]
        output: Option<String>,
        /// Quiet mode (exit code only, no output)
        #[arg(short, long)]
        quiet: bool,
    },
    /// Show DCP metadata
    Info {
        /// DCP directory
        dcp_dir: String,
    },
    /// Generate KDM for encrypted DCP
    Kdm {
        /// CPL ID
        #[arg(long)]
        cpl_id: String,
        /// Content title
        #[arg(long)]
        content_title: String,
        /// Recipient certificate file
        #[arg(long)]
        cert: String,
        /// Signer leaf certificate file
        #[arg(long)]
        signer_cert: String,
        /// Signer private key file
        #[arg(long)]
        signer_key: String,
        /// Signer CA certificate above the leaf (repeatable: intermediate(s) then root)
        #[arg(long)]
        signer_chain: Vec<String>,
        /// Output KDM file
        #[arg(short, long)]
        output: String,
        /// Valid from (ISO 8601 or "now"). Overrides a --template start.
        #[arg(short = 'f', long)]
        valid_from: Option<String>,
        /// Valid to (ISO 8601 or relative duration). Overrides a --template end.
        #[arg(short = 't', long)]
        valid_to: Option<String>,
        /// Named validity template to expand the window from (kdm-template)
        #[arg(long)]
        template: Option<String>,
        /// Validity templates file (default: XDG data dir)
        #[arg(long)]
        templates_file: Option<String>,
        /// KDM history log file (default: XDG data dir); every KDM is recorded
        #[arg(long)]
        history_file: Option<String>,
        /// Email the KDM to this address (repeatable). Requires --smtp-config
        #[arg(long = "email-to")]
        email_to: Vec<String>,
        /// SMTP config TOML for sending the KDM by email
        #[arg(long)]
        smtp_config: Option<String>,
        /// DCP keys file (KEYS.json from `create --encrypt`) whose content keys
        /// this KDM should carry. Required to unlock an encrypted DCP.
        #[arg(long)]
        keys: Option<String>,
        /// KDM format: smpte (default) or interop (legacy, needs real-gear validation)
        #[arg(long, default_value = "smpte")]
        format: String,
        /// AnnotationText override (default: "<title> KDM for <recipient>")
        #[arg(long)]
        annotation: Option<String>,
    },
    /// Re-wrap a DKDM to a new recipient
    KdmRewrap {
        /// Source DKDM file
        #[arg(long)]
        dkdm: String,
        /// DKDM recipient's private key (decrypts the source key blocks)
        #[arg(long)]
        dkdm_key: String,
        /// New recipient certificate file
        #[arg(long)]
        cert: String,
        /// Signer leaf certificate file
        #[arg(long)]
        signer_cert: String,
        /// Signer private key file
        #[arg(long)]
        signer_key: String,
        /// Signer CA certificate above the leaf (repeatable: intermediate(s) then root)
        #[arg(long)]
        signer_chain: Vec<String>,
        /// Valid from: ISO 8601 or "now"; empty preserves the DKDM window
        #[arg(short = 'f', long, default_value = "")]
        valid_from: String,
        /// Valid to: ISO 8601 or relative duration; empty preserves the DKDM window
        #[arg(short = 't', long, default_value = "")]
        valid_to: String,
        /// Output KDM file
        #[arg(short, long)]
        output: String,
    },
    /// Copy DCP to drive
    Copy {
        /// DCP directory
        #[arg(long)]
        src: String,
        /// Destination drive/directory
        #[arg(long)]
        dst: String,
    },
    /// Format a delivery drive as ext2/ext3 (cinema hard-drive delivery).
    /// Refuses any mounted target; requires --yes.
    FormatDrive {
        /// Target block device (or regular file with --image)
        target: String,
        /// Filesystem: ext2 or ext3
        #[arg(long, default_value = "ext2")]
        fs: String,
        /// Volume label
        #[arg(long)]
        label: Option<String>,
        /// Confirm the erase (required)
        #[arg(long)]
        yes: bool,
        /// Format a regular file instead of a block device (tests/loopback)
        #[arg(long)]
        image: bool,
    },
    /// Report a drive's filesystem type and label without modifying it.
    CheckDrive {
        /// Target block device or image file
        target: String,
    },
    /// Measure audio loudness
    Loudness {
        /// Audio file
        audio_file: String,
    },
    /// Equal-power crossfade join of two WAVs (dom#374)
    Crossfade {
        /// First (leading) WAV
        #[arg(long)]
        a: String,
        /// Second (trailing) WAV
        #[arg(long)]
        b: String,
        /// Output WAV
        #[arg(short, long)]
        output: String,
        /// Overlap length in seconds
        #[arg(long, default_value = "1.0")]
        overlap: f64,
    },
    /// Decode a mid-side channel pair to L/R in a WAV (dom#3020)
    MidSideDecode {
        /// Input WAV
        #[arg(short, long)]
        input: String,
        /// Output WAV
        #[arg(short, long)]
        output: String,
        /// Mid channel index (0-based); becomes left
        #[arg(long, default_value = "0")]
        mid: usize,
        /// Side channel index (0-based); becomes right
        #[arg(long, default_value = "1")]
        side: usize,
    },
    /// Generate QC report
    Report {
        /// DCP directory
        #[arg(long)]
        dcp: String,
        /// Output HTML file
        #[arg(short, long)]
        output: String,
    },
    /// Start REST API server
    Serve {
        /// Listen address (host:port)
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        bind: String,
    },
    /// Watch directory for auto-DCP creation
    Watch {
        /// Directory to watch
        dir: String,
        /// POST a JSON notification to this URL when a new DCP is detected
        #[arg(long)]
        webhook_url: Option<String>,
    },
    /// Export a DCP picture MXF to a delivery format via ffmpeg
    Export {
        /// Input picture MXF
        #[arg(long)]
        input: String,
        /// Output file (or directory for image-sequence)
        #[arg(short, long)]
        output: String,
        /// Format: prores, h264, h265, dnxhr, image-sequence
        #[arg(long, default_value = "h264")]
        format: String,
        /// Quality CRF for h264/h265 (lower is better; default 18)
        #[arg(long, default_value = "18")]
        crf: u32,
        /// Optional sound MXF to mux into the output
        #[arg(long)]
        audio: Option<String>,
    },
    /// Generate shell completion
    Completion {
        /// Shell (bash|zsh|fish)
        #[arg(default_value = "bash")]
        shell: String,
    },
    /// Start job queue daemon
    Daemon,
    /// Manage job queue
    Batch {
        #[command(subcommand)]
        action: BatchAction,
    },
    /// Convert SRT subtitles to DCP XML (SMPTE Timed Text)
    SubtitleConvert {
        /// Input SRT file
        #[arg(short, long)]
        input: String,
        /// Output XML file
        #[arg(short, long)]
        output: String,
        /// Language code (e.g. "en", "fr", "de")
        #[arg(short, long, default_value = "en")]
        language: String,
        /// Frame rate for timecode conversion (24, 25, 30, 48)
        #[arg(long, default_value = "24")]
        fps: u32,
        /// Bottom-line position as a percentage up from the bottom of the screen
        #[arg(long, default_value = "8.0")]
        vposition: f64,
    },
    /// Extract timed text from a DCP or subtitle asset to SRT or plain text
    SubtitleExtract {
        /// Input DCP directory, or a subtitle asset (XML or timed-text MXF)
        #[arg(short, long)]
        input: String,
        /// Output file; .srt keeps timing, .txt is text only
        #[arg(short, long)]
        output: String,
    },
    /// Edit a standalone subtitle file: list cues, shift timing, or change a
    /// cue's text/timing, writing SRT back out (dom#828, dom#2071). It edits
    /// source subtitle files, never subtitles inside a finished DCP.
    SubtitleEdit {
        /// Input subtitle file (SRT/ASS/PAC/MKS/FCPXML/interop XML)
        #[arg(short, long)]
        input: String,
        /// Output SRT path (required for edits; omit with --list)
        #[arg(short, long)]
        output: Option<String>,
        /// List cues and exit without writing output
        #[arg(long)]
        list: bool,
        /// Shift every cue by this many milliseconds (may be negative)
        #[arg(long, allow_hyphen_values = true)]
        shift_ms: Option<i64>,
        /// 1-based cue index to edit with --text / --set-start-ms / --set-end-ms
        #[arg(long)]
        index: Option<usize>,
        /// New text for the --index cue
        #[arg(long)]
        text: Option<String>,
        /// New start time (ms) for the --index cue (with --set-end-ms)
        #[arg(long)]
        set_start_ms: Option<u64>,
        /// New end time (ms) for the --index cue (with --set-start-ms)
        #[arg(long)]
        set_end_ms: Option<u64>,
        /// Timecode rate for frame-based inputs (interop/PAC), default 24
        #[arg(long, default_value_t = 24)]
        fps: u32,
    },
    /// Burn subtitles into video
    #[command(alias = "burn-in")]
    Burnin {
        /// Input video file
        #[arg(short, long)]
        input: String,
        /// Subtitle file (SRT, ASS, or SMPTE XML)
        #[arg(short, long)]
        subtitles: String,
        /// Output video file
        #[arg(short, long)]
        output: String,
        /// Font size for burn-in (default: 24)
        #[arg(long, default_value = "24")]
        font_size: u32,
    },
    /// Convert video to a target DCI container (scale/crop/letterbox)
    Convert {
        /// Input video file
        #[arg(short, long)]
        input: String,
        /// Output video file
        #[arg(short, long)]
        output: String,
        /// Target container: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full
        #[arg(short, long)]
        target: String,
        /// Method: letterbox, crop, or scale
        #[arg(short, long, default_value = "letterbox")]
        method: String,
    },

    /// Create DCDM (Digital Cinema Distribution Master) X'Y'Z' sequence
    Dcdm {
        /// Input image sequence directory
        #[arg(short, long)]
        input: String,

        /// Output DCDM TIFF directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3, aces, logc)
        #[arg(short, long, default_value = "rec709")]
        colour_space: String,

        /// Optional 3D LUT for colour transform
        #[arg(long)]
        lut: Option<String>,
    },

    /// Convert colour space of images/video
    Colour {
        /// Input file or directory
        #[arg(short, long)]
        input: String,

        /// Output file or directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3, xyz, rec2020, aces, acescg, logc)
        #[arg(short, long)]
        source: String,

        /// Target colour space (rec709, p3, rec2020, xyz for DCDM, p3-d65 mastering)
        #[arg(short, long)]
        target: String,

        /// Optional 3D LUT file for custom transform
        #[arg(long)]
        lut: Option<String>,
    },

    /// Conform an EDL/xmeml timeline: parse, or (with --media-dir) resolve every
    /// reel to media and write a reel/asset plan + conform manifest
    Conform {
        /// Input timeline file (EDL, AAF, FCP XML, OTIO)
        #[arg(short, long)]
        input: String,

        /// Media directory: resolve each reel to a file here and assemble a plan
        #[arg(long)]
        media_dir: Option<String>,

        /// Output directory for the reel plan + conform manifest (with --media-dir)
        #[arg(short, long)]
        output: Option<String>,

        /// Output the parsed timeline as JSON (parse-only mode)
        #[arg(long)]
        json: bool,
    },

    /// Ingest camera raw media
    Ingest {
        /// Camera card/media directory
        #[arg(short, long)]
        source: String,

        /// Output directory
        #[arg(short, long)]
        output: String,

        /// Output format (dpx, tiff, exr, prores)
        #[arg(short, long, default_value = "dpx")]
        format: String,

        /// Colour space (ACES, Rec.709, P3, LogC)
        #[arg(short, long, default_value = "ACES")]
        colour_space: String,

        /// 3D LUT (.cube) applied during transcode via ffmpeg lut3d
        #[arg(long)]
        lut: Option<String>,
    },

    /// Extract a frame from video/MXF as image
    #[command(name = "frame-extract")]
    FrameExtract {
        /// Input video/MXF file
        #[arg(short, long)]
        input: String,

        /// Frame number to extract
        #[arg(short, long, default_value = "0")]
        frame: u32,

        /// Output image file (png, jpg, tiff)
        #[arg(short, long)]
        output: String,
    },

    /// Inject Dolby Vision RPU into HEVC stream
    #[command(name = "dv-inject")]
    DvInject {
        /// Input HEVC file
        #[arg(short, long)]
        input: String,

        /// RPU file (.bin)
        #[arg(short, long)]
        rpu: String,

        /// Output file
        #[arg(short, long)]
        output: String,
    },

    /// Inject HDR10 static metadata
    #[command(name = "hdr10-inject")]
    Hdr10Inject {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Output video file
        #[arg(short, long)]
        output: String,

        /// Max content light level (MaxCLL)
        #[arg(long, default_value = "1000")]
        max_cll: u16,

        /// Max frame average light level (MaxFALL)
        #[arg(long, default_value = "400")]
        max_fall: u16,
    },

    /// Burn a visible watermark into a video/image file
    Watermark {
        /// Input video/image file
        #[arg(short, long)]
        input: String,

        /// Output video/image file
        #[arg(short, long)]
        output: String,

        /// Watermark payload (distributor ID, serial, etc.) rendered visibly
        #[arg(short, long)]
        payload: String,
    },

    /// Generate or inspect X.509 certificates for DCP encryption
    #[command(alias = "cert")]
    Certificate {
        #[command(subcommand)]
        action: CertAction,
    },

    /// Generate KDMs for multiple recipients in one pass
    #[command(name = "kdm-batch")]
    KdmBatch {
        /// CPL ID
        #[arg(long)]
        cpl_id: String,
        /// Content title
        #[arg(long)]
        content_title: String,
        /// Recipient certificate file (repeatable, one KDM generated per cert)
        #[arg(long = "cert")]
        certs: Vec<String>,
        /// Directory of recipient certificates; every *.pem/*.crt/*.cer in it
        /// gets a KDM. Combined with any --cert values.
        #[arg(long)]
        cert_dir: Option<String>,
        /// Cinema in the db: generate a KDM for every screen (repeatable)
        #[arg(long = "cinema")]
        cinemas: Vec<String>,
        /// Single screen selector "cinema/screen" from the db (repeatable)
        #[arg(long = "screen")]
        screens: Vec<String>,
        /// Cinema db file (default: XDG data dir)
        #[arg(long)]
        db: Option<String>,
        /// Signer leaf certificate file
        #[arg(long)]
        signer_cert: String,
        /// Signer private key file
        #[arg(long)]
        signer_key: String,
        /// Signer CA certificate above the leaf (repeatable)
        #[arg(long)]
        signer_chain: Vec<String>,
        /// Output directory for generated KDMs
        #[arg(short, long)]
        output_dir: String,
        /// Valid from (ISO 8601 or "now"). Overrides a --template start.
        #[arg(short = 'f', long)]
        valid_from: Option<String>,
        /// Valid to (ISO 8601 or relative duration). Overrides a --template end.
        #[arg(short = 't', long)]
        valid_to: Option<String>,
        /// Named validity template to expand the window from
        #[arg(long)]
        template: Option<String>,
        /// Validity templates file (default: XDG data dir)
        #[arg(long)]
        templates_file: Option<String>,
        /// KDM history log file (default: XDG data dir); every KDM is recorded
        #[arg(long)]
        history_file: Option<String>,
        /// Email each cinema its KDMs zipped (one email per cinema). Extra
        /// address(es) to add to every email (repeatable)
        #[arg(long = "email-to")]
        email_to: Vec<String>,
        /// SMTP config TOML for sending KDMs by email
        #[arg(long)]
        smtp_config: Option<String>,
        /// Ignore cinema contact emails; send only to --email-to (dom#2515)
        #[arg(long)]
        email_only_additional: bool,
        /// DCP keys file (KEYS.json from `create --encrypt`) whose content keys
        /// every generated KDM should carry.
        #[arg(long)]
        keys: Option<String>,
        /// KDM format: smpte (default) or interop (legacy, needs real-gear validation)
        #[arg(long, default_value = "smpte")]
        format: String,
    },

    /// Manage the cinema/screen database
    Cinema {
        /// Cinema db file (default: XDG data dir)
        #[arg(long, global = true)]
        db: Option<String>,
        #[command(subcommand)]
        action: CinemaAction,
    },

    /// Show the KDM generation history log (dom#1014)
    #[command(name = "kdm-history")]
    KdmHistory {
        /// History log file (default: XDG data dir)
        #[arg(long)]
        history_file: Option<String>,
        /// Filter by content title substring
        #[arg(long)]
        title: Option<String>,
        /// Filter by recipient subject or serial substring
        #[arg(long)]
        recipient: Option<String>,
        /// Only records at or after this date/prefix (e.g. "2026-07")
        #[arg(long)]
        since: Option<String>,
        /// Only records at or before this date/prefix
        #[arg(long)]
        until: Option<String>,
    },

    /// Manage named KDM validity templates (dom#2424)
    #[command(name = "kdm-template")]
    KdmTemplate {
        /// Templates file (default: XDG data dir)
        #[arg(long, global = true)]
        templates_file: Option<String>,
        #[command(subcommand)]
        action: TemplateAction,
    },

    /// Download a projector/server certificate by vendor + serial (dom#2705)
    #[command(name = "cert-fetch")]
    CertFetch {
        /// Vendor: dolby/doremi, qube (anonymous); christie, gdc, barco
        /// (need --user/--password). Others must be obtained from the vendor.
        #[arg(long)]
        vendor: String,
        /// Server serial number
        #[arg(long)]
        serial: String,
        /// Device type (qube only, e.g. QXPD)
        #[arg(long = "type")]
        device_type: Option<String>,
        /// Vendor account user (christie/gdc/barco)
        #[arg(long)]
        user: Option<String>,
        /// Vendor account password (christie/gdc/barco); never logged
        #[arg(long)]
        password: Option<String>,
        /// Output PEM file for the downloaded certificate
        #[arg(short, long)]
        output: String,
    },

    /// Package a trailer (ratings card + countdown leader + content)
    Trailer {
        /// Trailer content video file
        #[arg(short, long)]
        content: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// Trailer title (rendered on the ratings card)
        #[arg(long, default_value = "")]
        title: String,
        /// Rating text (e.g. "PG-13", "15")
        #[arg(long, default_value = "")]
        rating: String,
        /// Rating system: mpaa, bbfc, fsk, custom
        #[arg(long, default_value = "mpaa")]
        rating_system: String,
        /// Band colour: green, red, yellow
        #[arg(long, default_value = "green")]
        band: String,
        /// Countdown leader length in seconds
        #[arg(long, default_value = "8")]
        countdown: u32,
        /// Frame rate
        #[arg(long, default_value = "24")]
        fps: u32,
    },

    /// Generate DCP markers for a composition
    Markers {
        /// Composition length in frames
        #[arg(short, long)]
        frames: u64,
        /// Place a marker: LABEL=timecode (repeatable). LABEL is one of FFOC,
        /// LFOC, FFTC, LFTC, FFOI, LFOI, FFEC, LFEC, FFMC, LFMC; timecode is a
        /// frame number or HH:MM:SS:FF. Given markers replace the FFOC/LFOC
        /// default set.
        #[arg(long = "marker")]
        markers: Vec<String>,
        /// Frame rate for HH:MM:SS:FF timecodes (default 24)
        #[arg(long, default_value = "24")]
        fps: u32,
        /// Emit an XML MarkerList instead of a plain list
        #[arg(long)]
        xml: bool,
    },

    /// Check accessibility compliance of a DCP
    Accessibility {
        /// DCP directory
        dcp_dir: String,
        /// Standard: cvaa, eaa, aoda, ofcom
        #[arg(short, long, default_value = "cvaa")]
        standard: String,
    },

    /// Send a webhook notification (HTTP POST via curl)
    Webhook {
        /// Target URL
        #[arg(short, long)]
        url: String,
        /// Event type
        #[arg(long, default_value = "ping")]
        event: String,
        /// Job ID
        #[arg(long, default_value = "")]
        job_id: String,
        /// Shared secret (sent as X-Webhook-Secret)
        #[arg(long, default_value = "")]
        secret: String,
        /// JSON payload (defaults to a test ping body)
        #[arg(long, default_value = "")]
        payload: String,
    },

    /// Content version / delivery history tracker (SQLite)
    Version {
        #[command(subcommand)]
        action: VersionAction,
    },

    /// OV/VF version dashboard and distribution tracking
    Dashboard {
        #[command(subcommand)]
        action: DashboardAction,
    },
}

#[derive(Subcommand)]
enum VersionAction {
    /// Record a delivery
    Record {
        /// Tracker database file
        #[arg(long, default_value = "deliveries.db")]
        db: String,
        /// Package UUID
        #[arg(long)]
        package_uuid: String,
        /// Title
        #[arg(long, default_value = "")]
        title: String,
        /// Version label (e.g. OV, VF)
        #[arg(long, default_value = "")]
        version: String,
        /// Destination
        #[arg(long, default_value = "")]
        destination: String,
        /// Delivery method (e.g. hard_drive, satellite)
        #[arg(long, default_value = "")]
        method: String,
        /// Mark as verified
        #[arg(long)]
        verified: bool,
    },
    /// List recorded deliveries
    List {
        /// Tracker database file
        #[arg(long, default_value = "deliveries.db")]
        db: String,
        /// Filter by package UUID
        #[arg(long)]
        package_uuid: Option<String>,
        /// Filter by destination
        #[arg(long)]
        destination: Option<String>,
    },
    /// Export delivery history (format by extension: .json or .csv)
    Export {
        /// Tracker database file
        #[arg(long, default_value = "deliveries.db")]
        db: String,
        /// Output file (.json or .csv)
        #[arg(short, long)]
        output: String,
    },
}

#[derive(Subcommand)]
enum DashboardAction {
    /// Register a DCP version (OV or VF)
    Register {
        /// Version UUID
        #[arg(long)]
        uuid: String,
        /// Title
        #[arg(long)]
        title: String,
        /// Version type: OV or VF
        #[arg(long, default_value = "OV")]
        version_type: String,
        /// Territory (ISO 3166-1 alpha-2)
        #[arg(long, default_value = "")]
        territory: String,
        /// Language (RFC 5646)
        #[arg(long, default_value = "")]
        language: String,
        /// Standard: SMPTE or Interop
        #[arg(long, default_value = "SMPTE")]
        standard: String,
        /// DCP path
        #[arg(long, default_value = "")]
        dcp_path: String,
        /// Status: draft, released, archived
        #[arg(long, default_value = "draft")]
        status: String,
        /// KDM recipient theatre (repeatable)
        #[arg(long = "kdm-recipient")]
        kdm_recipients: Vec<String>,
    },
    /// List registered versions
    List {
        /// Filter by territory
        #[arg(long)]
        territory: Option<String>,
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
    },
    /// Update a version's status
    Status {
        /// Version UUID
        #[arg(long)]
        uuid: String,
        /// New status
        #[arg(long)]
        status: String,
    },
    /// Export the distribution matrix as CSV
    Matrix {
        /// Output CSV file
        #[arg(short, long)]
        output: String,
    },
    /// Start the dashboard HTTP server
    Serve {
        /// Listen port
        #[arg(short, long, default_value = "9090")]
        port: u32,
        /// Bind address
        #[arg(short, long, default_value = "127.0.0.1")]
        bind: String,
    },
}

#[derive(Subcommand)]
enum CertAction {
    /// Generate a full certificate chain (root → intermediate → signer)
    Chain {
        /// Organization name for the certificates
        #[arg(long)]
        organization: String,
        /// Output directory for generated certificates
        #[arg(short, long)]
        output: String,
    },
    /// Generate a single certificate
    Generate {
        /// Certificate type: root, intermediate, leaf, signer
        #[arg(short = 't', long, default_value = "signer")]
        cert_type: String,
        /// Common Name (CN)
        #[arg(long)]
        cn: String,
        /// Organization
        #[arg(long, default_value = "")]
        organization: String,
        /// Output certificate file
        #[arg(long)]
        output_cert: String,
        /// Output private key file
        #[arg(long)]
        output_key: String,
        /// Issuer certificate (required for non-root)
        #[arg(long)]
        issuer_cert: Option<String>,
        /// Issuer private key (required for non-root)
        #[arg(long)]
        issuer_key: Option<String>,
        /// Key size in bits
        #[arg(long, default_value = "2048")]
        key_bits: u32,
        /// Validity in days
        #[arg(long, default_value = "3650")]
        validity_days: u32,
    },
    /// Inspect a certificate file and show its details
    Inspect {
        /// Path to PEM certificate file
        cert_file: String,
    },
}

#[derive(Subcommand)]
enum CinemaAction {
    /// Add a cinema
    Add {
        /// Cinema name
        #[arg(long)]
        name: String,
        /// Contact email (repeatable)
        #[arg(long = "email")]
        emails: Vec<String>,
        /// Free-text notes
        #[arg(long, default_value = "")]
        notes: String,
    },
    /// List cinemas and their screens
    List,
    /// Remove a cinema
    Remove {
        #[arg(long)]
        name: String,
    },
    /// Add a screen with its recipient certificate
    AddScreen {
        /// Cinema name
        #[arg(long)]
        cinema: String,
        /// Screen name
        #[arg(long)]
        name: String,
        /// Recipient certificate (PEM/CRT file)
        #[arg(long)]
        cert: String,
        /// Embed the certificate PEM in the db instead of storing the path
        #[arg(long)]
        inline: bool,
    },
    /// Remove a screen from a cinema
    RemoveScreen {
        #[arg(long)]
        cinema: String,
        #[arg(long)]
        name: String,
    },
    /// Search cinemas/screens by name or certificate serial/thumbprint (dom#2707)
    Search {
        /// Query substring
        query: String,
    },
    /// Import a facility from an FLM-x file (dom#239)
    ImportFlm {
        /// FLM XML file
        file: String,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// Add a validity template
    Add {
        /// Template name
        #[arg(long)]
        name: String,
        /// Offset from now to the start (e.g. "0 days", "2 days")
        #[arg(long, default_value = "")]
        start_offset: String,
        /// Window length (e.g. "1 week", "180 days")
        #[arg(long)]
        duration: String,
        /// UTC offset for emitted timestamps (e.g. "+02:00"); empty = UTC
        #[arg(long, default_value = "")]
        tz_offset: String,
    },
    /// List validity templates
    List,
    /// Remove a validity template
    Remove {
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum BatchAction {
    /// List all jobs
    List,
    /// Submit a new job
    Add {
        /// Job type (create-dcp|verify-dcp|export-dcp|import-video|encode-j2k|wrap-mxf|copy-to-drive)
        #[arg(short = 'T', long)]
        r#type: String,
        /// Job parameters (JSON string)
        #[arg(short, long)]
        params: String,
    },
    /// Cancel a job
    Cancel {
        /// Job ID to cancel
        id: String,
    },
}

fn parse_colour_space(s: &str) -> postkit::colour::ColourSpace {
    match s.to_lowercase().as_str() {
        "rec709" | "bt709" => postkit::colour::ColourSpace::Rec709,
        "p3" | "dcip3" | "dci-p3" => postkit::colour::ColourSpace::P3,
        "xyz" | "ciexyz" => postkit::colour::ColourSpace::Xyz,
        "rec2020" | "bt2020" => postkit::colour::ColourSpace::Rec2020,
        "aces" => postkit::colour::ColourSpace::Aces,
        "acescg" => postkit::colour::ColourSpace::AcesCg,
        "logc" | "arrilogc" => postkit::colour::ColourSpace::LogC,
        _ => {
            tracing::warn!("Unknown colour space '{s}', defaulting to Rec709");
            postkit::colour::ColourSpace::Rec709
        }
    }
}

/// Map a `colour --target` string to a dcdm-module target (X'Y'Z' DCDM or P3-D65
/// mastering). Returns None for ffmpeg colorspace targets (rec709/p3/rec2020).
fn parse_dcdm_target(s: &str) -> Option<postkit::dcdm::DcdmTarget> {
    match s.to_lowercase().as_str() {
        "xyz" | "ciexyz" => Some(postkit::dcdm::DcdmTarget::Xyz),
        "p3-d65" | "p3d65" => Some(postkit::dcdm::DcdmTarget::P3D65),
        _ => None,
    }
}

// ── create-time helpers (container dims, reel splits, input range) ───────────

/// Resolve container dimensions from a preset name or a custom WxH.
///
/// `dims` (e.g. "1920x1080") wins over a `preset`; both absent yields (0,0)
/// meaning "use the full-container resolution default". Custom dims are validated
/// against the 2K/4K bounds.
fn resolve_container(
    preset: Option<&str>,
    dims: Option<&str>,
    is_4k: bool,
) -> Result<(u32, u32), String> {
    if let Some(spec) = dims {
        let (w, h) = spec
            .split_once(['x', 'X'])
            .ok_or_else(|| format!("--container-dims '{spec}' must be WxH (e.g. 1920x1080)"))?;
        let w: u32 = w
            .trim()
            .parse()
            .map_err(|_| format!("invalid width in --container-dims '{spec}'"))?;
        let h: u32 = h
            .trim()
            .parse()
            .map_err(|_| format!("invalid height in --container-dims '{spec}'"))?;
        dcpwizard_core::dcp::validate_container_dims(w, h, is_4k)?;
        return Ok((w, h));
    }
    match preset {
        Some("2k-scope") => Ok((2048, 858)),
        Some("2k-flat") => Ok((1998, 1080)),
        Some("2k-full") => Ok((2048, 1080)),
        Some("4k-scope") => Ok((4096, 1716)),
        Some("4k-flat") => Ok((3996, 2160)),
        Some("4k-full") => Ok((4096, 2160)),
        Some(value) => Err(format!("Unknown container: {value}")),
        None => Ok((0, 0)),
    }
}

/// ffprobe chapter boundaries for `video` at `fps`, or a loud error.
fn video_chapter_boundaries(video: &Path, fps: u32) -> Result<Vec<u64>, String> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_chapters"])
        .arg(video)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err("ffprobe failed to read chapters".into());
    }
    let json = String::from_utf8_lossy(&out.stdout);
    dcpwizard_core::reel::parse_chapter_starts(&json, fps)
}

/// Resolve reel-split boundaries from --split-at timecodes or --split-chapters.
fn resolve_reel_splits(
    split_at: Option<&str>,
    split_chapters: bool,
    chapter_video: Option<&Path>,
    fps: u32,
) -> Result<Vec<u64>, String> {
    if let Some(spec) = split_at {
        let mut frames = Vec::new();
        for tc in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            frames.push(dcpwizard_core::reel::parse_timecode(tc, fps)?);
        }
        if frames.is_empty() {
            return Err("--split-at needs at least one timecode".into());
        }
        return Ok(frames);
    }
    if split_chapters {
        let video = chapter_video
            .ok_or("--split-chapters needs a video input to read chapter marks from")?;
        return video_chapter_boundaries(video, fps);
    }
    Ok(Vec::new())
}

/// Re-encode `video` to a lossless intermediate that forces the given decode
/// `range` (full|legal), so the downstream ffmpeg raw-RGB decode is correct even
/// when the source range metadata is wrong or missing. Returns the intermediate path.
fn normalize_input_range(video: &Path, range: &str, out_dir: &Path) -> Result<PathBuf, String> {
    // ffmpeg's scale in_range names: full<->pc, legal<->tv/mpeg
    let in_range = if range == "full" { "full" } else { "tv" };
    let out = out_dir.join("range_corrected.mkv");
    let status = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(video)
        .arg("-vf")
        .arg(format!("scale=in_range={in_range}:out_range=full"))
        .args(["-c:v", "ffv1", "-level", "3", "-pix_fmt", "gbrp16le", "-an"])
        .arg(&out)
        .status()
        .map_err(|e| format!("failed to run ffmpeg for input-range correction: {e}"))?;
    if !status.success() {
        return Err("ffmpeg input-range correction failed".into());
    }
    Ok(out)
}

/// Build the combined sign-language sound track (ISDCF Doc 13): VP9-pack the
/// sign video onto channel 15 over the main audio. Returns the combined WAV and
/// the leading main-audio channel count for the SLVS MCA config.
fn build_sign_language_audio(
    slvs_video: &str,
    main_audio: Option<&Path>,
    min_frames: u64,
    fps: u32,
    output_dir: &Path,
) -> Result<(PathBuf, u32), String> {
    let combined = output_dir.join("slvs_sound.wav");
    let main_channels = dcpwizard_core::sign_language::build_slvs_sound(
        &PathBuf::from(slvs_video),
        main_audio,
        min_frames,
        fps,
        &combined,
    )?;
    Ok((combined, main_channels))
}

/// Create-time audio processing (W5): filename channel routing when `audio` is a
/// directory (dom#2134), then stereo->5.1 upmix (dom#921/#1080), then loudness
/// normalization (dom#1382). Intermediates go under `work_dir` (a scratch dir).
/// Runs before sign-language packing and any pull-up.
fn prepare_create_audio(
    audio: Option<PathBuf>,
    upmix: Option<&str>,
    loudness_target: Option<&str>,
    true_peak_ceiling: Option<f64>,
    work_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(mut path) = audio else {
        return Ok(None);
    };

    if path.is_dir() {
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let routed = work_dir.join("routed.wav");
        path = dcpwizard_core::audio_route::route_directory(&path, &routed)?;
        tracing::info!("Routed channel WAVs from the input directory by filename");
    }

    if let Some(v) = upmix {
        let variant = match v {
            "a" | "A" => postkit::upmix::Upmixer::A,
            "b" | "B" => postkit::upmix::Upmixer::B,
            other => return Err(format!("unknown upmix variant '{other}' (use a or b)")),
        };
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("upmix.wav");
        postkit::upmix::upmix_wav(variant, &path, &out).map_err(|e| e.to_string())?;
        tracing::info!("Upmixed stereo to 5.1 (variant {v})");
        path = out;
    }

    if let Some(spec) = loudness_target {
        let target = dcpwizard_core::loudness::parse_loudness_target(spec)?;
        let ceiling =
            true_peak_ceiling.unwrap_or(dcpwizard_core::loudness::DEFAULT_TRUE_PEAK_CEILING_DBTP);
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("loudness.wav");
        let plan = dcpwizard_core::loudness::adjust_loudness(&path, &out, target, ceiling)
            .map_err(|e| e.to_string())?;
        tracing::info!(
            "Loudness adjusted {:.1} -> {:.1} dB (gain {:+.2} dB, peak {:.2} dBTP)",
            plan.measured_db,
            plan.target_db,
            plan.gain_db,
            plan.resulting_true_peak_dbtp,
        );
        path = out;
    }

    Ok(Some(path))
}

/// Validate the DCI HDR Addendum flag combo and the raised per-codestream cap.
/// The picture MXF is wrapped with TransferCharacteristic=ST 2084 / P3-D65
/// primaries in create_dcp; this only rejects an unusable request up front.
fn validate_hdr_dci(
    hdr_to_dci_lut: &Option<String>,
    hdr_already_pq: bool,
    frame_rate: Option<u32>,
    video_bit_rate: Option<u32>,
) {
    use dcpwizard_core::hdr;
    if hdr_to_dci_lut.is_none() && !hdr_already_pq {
        tracing::error!(
            "--hdr-dci needs the source path to PQ: pass --hdr-to-dci-lut or --hdr-already-pq"
        );
        std::process::exit(1);
    }
    let rate = frame_rate.unwrap_or(24);
    let cap = hdr::hdr_codestream_byte_cap(rate);
    if let Some(mbps) = video_bit_rate
        && mbps > hdr::HDR_MAX_MBPS
    {
        tracing::error!(
            "--hdr-dci caps the codestream at {cap} bytes/frame ({} Mbit/s at {rate} fps); requested {mbps} Mbit/s exceeds it",
            hdr::HDR_MAX_MBPS
        );
        std::process::exit(1);
    }
}

// ── KDM distribution helpers ────────────────────────────────────────────────

/// resolve a validity window: a --template supplies the base window, explicit
/// --valid-from/--valid-to override it, and the fallback is now .. +2 weeks.
fn resolve_window(
    valid_from: Option<String>,
    valid_to: Option<String>,
    template: Option<String>,
    templates_file: Option<String>,
) -> Result<(String, String), String> {
    let (mut vf, mut vt) = ("now".to_string(), "2 weeks".to_string());
    if let Some(name) = template {
        let path = templates_file
            .map(PathBuf::from)
            .unwrap_or_else(dcpwizard_core::store::default_templates_path);
        let store = dcpwizard_core::kdm_template::TemplateStore::load(&path)?;
        let t = store
            .get(&name)
            .ok_or_else(|| format!("template '{name}' not found"))?;
        let (f, t2) = t.expand()?;
        vf = f;
        vt = t2;
    }
    if let Some(f) = valid_from {
        vf = f;
    }
    if let Some(t) = valid_to {
        vt = t;
    }
    Ok((vf, vt))
}

fn history_path(history_file: Option<String>) -> PathBuf {
    history_file
        .map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::store::default_history_path)
}

fn cinema_db_path(db: Option<String>) -> PathBuf {
    db.map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::store::default_db_path)
}

fn templates_db_path(templates_file: Option<String>) -> PathBuf {
    templates_file
        .map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::store::default_templates_path)
}

fn send_kdm_email(
    cfg_path: &str,
    cinema: &str,
    title: &str,
    to: &[String],
    files: &[PathBuf],
) -> Result<(), String> {
    if to.is_empty() {
        return Err("no email recipients: pass --email-to".to_string());
    }
    let cfg = dcpwizard_core::email::SmtpConfig::load(Path::new(cfg_path))?;
    dcpwizard_core::email::send_kdms(&cfg, cinema, title, to, files)
}

fn sanitize_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn xml_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("xml"))
        .collect();
    v.sort();
    v
}

/// a set of recipients that share one delivery email (one cinema, or the loose
/// --cert/--cert-dir group with an empty name).
struct BatchGroup {
    name: String,
    emails: Vec<String>,
    cert_paths: Vec<PathBuf>,
}

struct KdmBatchArgs {
    cpl_id: String,
    content_title: String,
    certs: Vec<String>,
    cert_dir: Option<String>,
    cinemas: Vec<String>,
    screens: Vec<String>,
    db: Option<String>,
    signer_cert: String,
    signer_key: String,
    signer_chain: Vec<String>,
    output_dir: String,
    valid_from: Option<String>,
    valid_to: Option<String>,
    template: Option<String>,
    templates_file: Option<String>,
    history_file: Option<String>,
    email_to: Vec<String>,
    smtp_config: Option<String>,
    email_only_additional: bool,
    keys: Option<String>,
    format: String,
}

fn run_kdm_batch(a: KdmBatchArgs) -> i32 {
    let format = match dcpwizard_core::kdm::parse_format(&a.format) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let (valid_from, valid_to) =
        match resolve_window(a.valid_from, a.valid_to, a.template, a.templates_file) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        };
    let content_keys = match a.keys {
        Some(path) => match dcpwizard_core::kdm::load_content_keys(&PathBuf::from(path), &a.cpl_id)
        {
            Ok(k) => k,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        },
        None => Vec::new(),
    };

    // materialized inline certs live here for the whole batch.
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("cannot create temp dir: {e}");
            return 1;
        }
    };

    // loose certs from --cert / --cert-dir go into an unnamed group.
    let mut loose: Vec<String> = a.certs;
    if let Some(dir) = a.cert_dir {
        match dcpwizard_core::kdm::certs_in_dir(&PathBuf::from(&dir)) {
            Ok(found) => loose.extend(found),
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        }
    }

    let mut groups: Vec<BatchGroup> = Vec::new();
    if !loose.is_empty() {
        groups.push(BatchGroup {
            name: String::new(),
            emails: Vec::new(),
            cert_paths: loose.into_iter().map(PathBuf::from).collect(),
        });
    }

    // db-resolved cinema/screen recipients, grouped by cinema.
    if !a.cinemas.is_empty() || !a.screens.is_empty() {
        let db = match dcpwizard_core::cinema::CinemaDb::load(&cinema_db_path(a.db)) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        };
        let recips = match db.resolve(&a.cinemas, &a.screens, tmp.path()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        };
        for r in recips {
            match groups.iter_mut().find(|g| g.name == r.cinema) {
                Some(g) => g.cert_paths.push(r.cert_path),
                None => groups.push(BatchGroup {
                    name: r.cinema,
                    emails: r.emails,
                    cert_paths: vec![r.cert_path],
                }),
            }
        }
    }

    if groups.iter().all(|g| g.cert_paths.is_empty()) {
        tracing::error!("No recipients (use --cert, --cert-dir, --cinema or --screen)");
        return 1;
    }

    let history = Some(history_path(a.history_file));
    let signer_cert = PathBuf::from(&a.signer_cert);
    let signer_key = PathBuf::from(&a.signer_key);
    let signer_chain: Vec<PathBuf> = a.signer_chain.iter().map(PathBuf::from).collect();
    let output_root = PathBuf::from(&a.output_dir);

    // without email: one flat batch into output_dir (preserves prior behaviour).
    if a.smtp_config.is_none() {
        let all: Vec<PathBuf> = groups.into_iter().flat_map(|g| g.cert_paths).collect();
        return dcpwizard_core::kdm::generate_kdm_batch(
            a.cpl_id,
            a.content_title,
            all,
            signer_cert,
            signer_key,
            signer_chain,
            valid_from,
            valid_to,
            content_keys,
            output_root,
            format,
            None,
            history,
        );
    }

    // with email: one email per group (dom#2516), each with that group's KDMs
    // zipped. multiple groups get their own subdir so files don't collide.
    let cfg_path = a.smtp_config.unwrap();
    let multi = groups.len() > 1;
    let mut failures = 0;
    for g in &groups {
        let out_dir = if multi {
            let sub = if g.name.is_empty() {
                "additional".to_string()
            } else {
                sanitize_dir_name(&g.name)
            };
            output_root.join(sub)
        } else {
            output_root.clone()
        };
        let code = dcpwizard_core::kdm::generate_kdm_batch(
            a.cpl_id.clone(),
            a.content_title.clone(),
            g.cert_paths.clone(),
            signer_cert.clone(),
            signer_key.clone(),
            signer_chain.clone(),
            valid_from.clone(),
            valid_to.clone(),
            content_keys.clone(),
            out_dir.clone(),
            format,
            None,
            history.clone(),
        );
        if code != 0 {
            failures += 1;
            continue;
        }
        // recipients: cinema contacts (unless only-additional) plus --email-to.
        let mut to: Vec<String> = if a.email_only_additional {
            Vec::new()
        } else {
            g.emails.clone()
        };
        for e in &a.email_to {
            if !to.contains(e) {
                to.push(e.clone());
            }
        }
        let files = xml_files_in(&out_dir);
        match send_kdm_email(&cfg_path, &g.name, &a.content_title, &to, &files) {
            Ok(()) => tracing::info!(
                "emailed {} KDM(s) for {}",
                files.len(),
                if g.name.is_empty() {
                    "additional recipients"
                } else {
                    &g.name
                }
            ),
            Err(e) => {
                tracing::error!("{e}");
                failures += 1;
            }
        }
    }
    if failures == 0 { 0 } else { 1 }
}

fn run_cinema(db: Option<String>, action: CinemaAction) -> i32 {
    let path = cinema_db_path(db);
    let mut store = match dcpwizard_core::cinema::CinemaDb::load(&path) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    use dcpwizard_core::cinema::CertSource;
    let mutated: Result<bool, String> = match action {
        CinemaAction::Add {
            name,
            emails,
            notes,
        } => store.add_cinema(&name, emails, notes).map(|_| true),
        CinemaAction::Remove { name } => store.remove_cinema(&name).map(|_| true),
        CinemaAction::AddScreen {
            cinema,
            name,
            cert,
            inline,
        } => {
            let src = if inline {
                match std::fs::read_to_string(&cert) {
                    Ok(pem) => CertSource::Inline(pem),
                    Err(e) => {
                        tracing::error!("cannot read {cert}: {e}");
                        return 1;
                    }
                }
            } else {
                CertSource::Path(PathBuf::from(&cert))
            };
            store.add_screen(&cinema, &name, src).map(|_| true)
        }
        CinemaAction::RemoveScreen { cinema, name } => {
            store.remove_screen(&cinema, &name).map(|_| true)
        }
        CinemaAction::ImportFlm { file } => match store.import_flm(&PathBuf::from(&file)) {
            Ok(summary) => {
                println!("imported {summary}");
                Ok(true)
            }
            Err(e) => Err(e),
        },
        CinemaAction::List => {
            for c in &store.cinemas {
                println!("{} [{}]", c.name, c.emails.join(", "));
                for s in &c.screens {
                    println!("  - {} (serial {})", s.name, s.cert_serial);
                }
            }
            Ok(false)
        }
        CinemaAction::Search { query } => {
            let hits = store.search(&query);
            if hits.is_empty() {
                println!("no matches for '{query}'");
            }
            for (cinema, screen) in hits {
                if screen.is_empty() {
                    println!("{cinema}");
                } else {
                    println!("{cinema} / {screen}");
                }
            }
            Ok(false)
        }
    };
    match mutated {
        Ok(true) => {
            if let Err(e) = store.save(&path) {
                tracing::error!("{e}");
                return 1;
            }
            0
        }
        Ok(false) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

fn run_kdm_history(
    history_file: Option<String>,
    title: Option<String>,
    recipient: Option<String>,
    since: Option<String>,
    until: Option<String>,
) -> i32 {
    let path = history_path(history_file);
    let all = match dcpwizard_core::kdm_log::read_all(&path) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let recs = dcpwizard_core::kdm_log::filter(
        all,
        title.as_deref(),
        recipient.as_deref(),
        since.as_deref(),
        until.as_deref(),
    );
    if recs.is_empty() {
        println!("no history records");
    }
    for r in recs {
        println!(
            "{}  {}  {}  serial={}  {}..{}  {}",
            r.timestamp,
            r.format,
            r.content_title,
            r.recipient_serial,
            r.valid_from,
            r.valid_to,
            r.output_path
        );
    }
    0
}

fn run_kdm_template(templates_file: Option<String>, action: TemplateAction) -> i32 {
    let path = templates_db_path(templates_file);
    let mut store = match dcpwizard_core::kdm_template::TemplateStore::load(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    use dcpwizard_core::kdm_template::Template;
    let mutated: Result<bool, String> = match action {
        TemplateAction::Add {
            name,
            start_offset,
            duration,
            tz_offset,
        } => {
            let t = Template {
                name,
                start_offset,
                duration,
                tz_offset,
            };
            // validate the window parses before persisting
            match t.expand() {
                Ok(_) => store.add(t).map(|_| true),
                Err(e) => Err(e),
            }
        }
        TemplateAction::Remove { name } => store.remove(&name).map(|_| true),
        TemplateAction::List => {
            for t in &store.templates {
                let tz = if t.tz_offset.is_empty() {
                    "UTC"
                } else {
                    &t.tz_offset
                };
                println!(
                    "{}: start +[{}] duration {} ({})",
                    t.name,
                    if t.start_offset.is_empty() {
                        "now"
                    } else {
                        &t.start_offset
                    },
                    t.duration,
                    tz
                );
            }
            Ok(false)
        }
    };
    match mutated {
        Ok(true) => {
            if let Err(e) = store.save(&path) {
                tracing::error!("{e}");
                return 1;
            }
            0
        }
        Ok(false) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

/// Resolve a parsed timeline against a media dir into a reel plan, write the
/// plan + conform manifest, and print the assembled reels. Per-reel encode/wrap
/// into a DCP is the remaining step; the plan is the executable hand-off.
fn run_conform_assembly(
    input: &str,
    timeline: &postkit::conform::Timeline,
    media_dir: &str,
    output: Option<&str>,
) -> i32 {
    let media = PathBuf::from(media_dir);
    let out = PathBuf::from(output.unwrap_or("conform_out"));
    let plan = match dcpwizard_core::conform::build_reel_plan(timeline, &media) {
        Ok(p) => p,
        Err(missing) => {
            for m in &missing {
                tracing::error!("unresolved reel (no matching media in {media_dir}): {m}");
            }
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&out) {
        tracing::error!("cannot create output dir: {e}");
        return 1;
    }
    // postkit conform writes the assembled timeline manifest
    let opts = postkit::conform::ConformOptions {
        timeline_file: PathBuf::from(input),
        media_dir: media,
        output_dir: out.clone(),
        ..Default::default()
    };
    if postkit::conform::conform(&opts) != 0 {
        tracing::error!("conform assembly failed");
        return 1;
    }
    // keep the reel plan as an artifact next to the manifest
    let plan_path = out.join("conform_plan.json");
    let plan_json = serde_json::to_string_pretty(&plan).unwrap_or_default();
    if let Err(e) = std::fs::write(&plan_path, plan_json) {
        tracing::error!("cannot write reel plan: {e}");
        return 1;
    }
    println!(
        "Conforming {} reel(s) from \"{}\" -> {}",
        plan.reels.len(),
        plan.title,
        out.display()
    );
    for r in &plan.reels {
        println!(
            "  {} [{}] {} ({}..{})",
            r.reel_name,
            r.track_type,
            r.media_path.display(),
            r.source_in,
            r.source_out
        );
    }

    // drive the plan to a finished multi-reel DCP (per-reel encode + wrap + assembly)
    dcpwizard_core::conform::assemble_dcp(&plan, &out)
}

/// Encode the packaged trailer mp4 to J2K and build a DCP (ContentKind=trailer)
/// in `<output_dir>/dcp`, reusing the same grok encode + create_dcp path as
/// `create --video`. The mp4 stays in place as the intermediate.
fn trailer_to_dcp(mp4: &Path, output_dir: &Path, fps_arg: u32) -> i32 {
    use postkit::grok_encoder::{self, CompressParams, EncodeProgress};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    if let Err(e) = dcpwizard_core::probe::ensure_video_decodable(mp4) {
        tracing::error!("{e}");
        return 1;
    }
    let info = dcpwizard_core::probe::probe_video(mp4);
    let (width, height, total_frames) = info
        .as_ref()
        .map(|v| (v.width, v.height, v.total_frames))
        .unwrap_or((1920, 1080, 0));
    let fps = if fps_arg > 0 { fps_arg } else { 24 };

    let j2k_dir = output_dir.join("j2k");
    if let Err(e) = std::fs::create_dir_all(&j2k_dir) {
        tracing::error!("Failed to create j2k dir: {e}");
        return 1;
    }
    let params = CompressParams {
        compression_ratio: 10.0,
        frame_rate: fps as u16,
        apply_xyz_transform: true,
        ..CompressParams::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let result = grok_encoder::encode_video_pipeline(
        mp4,
        &j2k_dir,
        &params,
        total_frames as u64,
        width,
        height,
        &cancel,
        |_p: EncodeProgress| {},
    );
    if !result.success {
        tracing::error!("Trailer encode failed: {}", result.error);
        return 1;
    }

    // demux audio if the packaged trailer carries any (card/leader are silent)
    let audio_path = {
        let wav = output_dir.join("audio_demux.wav");
        let demux = std::process::Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(mp4)
            .args(["-vn", "-acodec", "pcm_s24le", "-ar", "48000"])
            .arg(&wav)
            .output();
        match demux {
            Ok(o) if o.status.success() => Some(wav),
            _ => None,
        }
    };

    let dcp_dir = output_dir.join("dcp");
    let config = dcpwizard_core::dcp::DcpConfig {
        title: mp4
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Trailer")
            .to_string(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Trailer,
        frame_rate_num: fps,
        frame_rate_den: 1,
        // declare the CPL container at the encoded essence size
        container_width: width,
        container_height: height,
        output_dir: dcp_dir.clone(),
        j2k_dir: Some(j2k_dir),
        audio_path,
        subtitle_language: "en".to_string(),
        ..Default::default()
    };
    let code = dcpwizard_core::dcp::create_dcp(&config);
    if code == 0 {
        tracing::info!("Trailer DCP created: {}", dcp_dir.display());
        0
    } else {
        tracing::error!("Trailer DCP creation failed");
        1
    }
}

fn run_cert_fetch(
    vendor: String,
    serial: String,
    device_type: Option<String>,
    user: Option<String>,
    password: Option<String>,
    output: String,
) -> i32 {
    let v = match dcpwizard_core::cert_fetch::parse_vendor(&vendor) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let creds = match (user, password) {
        (Some(user), Some(password)) => {
            Some(dcpwizard_core::cert_fetch::Credentials { user, password })
        }
        (None, None) => None,
        _ => {
            tracing::error!("pass both --user and --password, or neither");
            return 1;
        }
    };
    match dcpwizard_core::cert_fetch::fetch(
        v,
        &serial,
        device_type.as_deref(),
        creds.as_ref(),
        &PathBuf::from(&output),
    ) {
        Ok(summary) => {
            println!("downloaded {summary} -> {output}");
            0
        }
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

fn main() {
    // Windows debug builds overflow the default 1MB stack due to large clap
    // derive enum (102 args across 34 subcommands). Spawn with 8MB stack.
    let thread = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn main thread");
    thread.join().unwrap();
}

fn run() {
    // User-friendly panic handler
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unexpected error".to_string()
        };
        let location = info
            .location()
            .map(|l| format!(" ({}:{})", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!("\nerror: dcpwizard crashed: {payload}{location}");
        eprintln!(
            "This is a bug. Please report it at https://github.com/PostPerfection/dcpwizard/issues"
        );
        eprintln!("Include the command you ran and any input files if possible.");
        if std::env::var("RUST_BACKTRACE").is_ok() {
            eprintln!(
                "\nBacktrace:\n{:?}",
                std::backtrace::Backtrace::force_capture()
            );
        } else {
            eprintln!("Set RUST_BACKTRACE=1 for a detailed backtrace.");
        }
    }));

    let cli = Cli::parse();

    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    postkit::grok_encoder::initialize(0);

    let code = match cli.command {
        Commands::Create {
            title,
            video,
            audio,
            audio_input_order,
            hdr_to_dci_lut,
            allow_generic_hdr_tonemap,
            hdr_dci,
            hdr_already_pq,
            sign_language_video,
            sign_language_lang,
            subtitle,
            versions,
            subtitle_language,
            subtitle_qol,
            output,
            standard,
            encrypt,
            key_out,
            content_type,
            frame_rate,
            twok,
            fourk,
            container,
            threads,
            video_bit_rate,
            reel_length,
            profile,
            right_eye,
            atmos,
            hi_channel,
            vi_channel,
            pad_head,
            pad_tail,
            pad_color,
            container_dims,
            split_at,
            split_chapters,
            input_range,
            audio_qol,
            signer_opts,
        } => {
            let CreateAudioQol {
                loudness_target,
                true_peak_ceiling,
                upmix,
                start_at,
                resume,
                shutdown_when_done,
            } = *audio_qol;
            // fail loud on shutdown up front, before the long encode, so the
            // user is not left with a finished DCP and no power-off.
            if shutdown_when_done
                && let Err(e) = dcpwizard_core::encode_qol::resolve_shutdown_command()
            {
                tracing::error!("{e}");
                std::process::exit(1);
            }
            // same for the signer: an unusable key or certificate must fail
            // before the encode, not after it.
            let package_signer = create_package_signer(&signer_opts);
            if let Some(signer) = package_signer.as_ref()
                && let Err(e) = signer.check_usable()
            {
                tracing::error!("{e}");
                std::process::exit(1);
            }
            // scheduled start: block until the wall-clock time before any work.
            if let Some(spec) = start_at.as_deref() {
                match dcpwizard_core::encode_qol::parse_start_at(
                    spec,
                    dcpwizard_core::encode_qol::now_local(),
                ) {
                    Ok(target) => {
                        tracing::info!("Scheduled start: waiting until {target}");
                        dcpwizard_core::encode_qol::wait_until(target);
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            }
            let video_path = PathBuf::from(&video);
            let output_dir = PathBuf::from(&output);
            let std_val = if standard == "interop" {
                dcpwizard_core::Standard::Interop
            } else {
                dcpwizard_core::Standard::Smpte
            };
            let audio_input_order = match audio_input_order.as_str() {
                "dcp" => dcpwizard_core::mxf_wrap::AudioInputOrder::Canonical51,
                "lrc-ls-rs-lfe" => dcpwizard_core::mxf_wrap::AudioInputOrder::LrcLsRsLfe,
                value => {
                    tracing::error!("Unknown audio input order: {value}");
                    return;
                }
            };

            // parse the multi-version manifest up front so a bad manifest fails
            // before any encoding
            let versions_specs = match versions.as_deref() {
                Some(path) => match dcpwizard_core::versions::load_versions(&PathBuf::from(path)) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::error!("{e}");
                        return;
                    }
                },
                None => None,
            };

            // Resolve delivery profile and apply its presets as defaults; explicit
            // flags still win.
            let profile = match profile.as_deref() {
                Some(name) => match dcpwizard_core::profiles::get_profile(name) {
                    Some(p) => {
                        tracing::info!("Using profile '{}': {}", p.name, p.description);
                        Some(p)
                    }
                    None => {
                        let names: Vec<String> = dcpwizard_core::profiles::all_profiles()
                            .into_iter()
                            .map(|p| p.name)
                            .collect();
                        tracing::error!(
                            "Unknown profile '{name}'. Available: {}",
                            names.join(", ")
                        );
                        std::process::exit(1);
                    }
                },
                None => None,
            };
            let fourk = fourk
                || (!twok
                    && profile
                        .as_ref()
                        .map(|p| p.resolution_width >= 4096)
                        .unwrap_or(false));
            let frame_rate = frame_rate.or_else(|| profile.as_ref().map(|p| p.frame_rate));
            let video_bit_rate =
                video_bit_rate.or_else(|| profile.as_ref().map(|p| p.bitrate_mbps));

            // DCI HDR Addendum: validate the flag combo + raised codestream cap.
            // The ST 2084 / P3-D65 ULs are written onto the picture MXF in create_dcp.
            if hdr_dci {
                validate_hdr_dci(&hdr_to_dci_lut, hdr_already_pq, frame_rate, video_bit_rate);
            }

            // Detect if input is a video file (not a J2K directory)
            let is_video_file = video_path.is_file()
                && video_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        matches!(
                            e.to_lowercase().as_str(),
                            "mp4"
                                | "mov"
                                | "mkv"
                                | "avi"
                                | "mxf"
                                | "ts"
                                | "m2ts"
                                | "mpg"
                                | "mpeg"
                                | "webm"
                                | "apv"
                        )
                    })
                    .unwrap_or(false);

            let CreateSubtitleOpts {
                subtitle_halign,
                subtitle_valign,
                subtitle_vposition,
                subtitle_zposition,
                subtitle_rtl,
                subtitle_wrap,
                subtitle_font,
                subtitle_no_subset,
                ccap,
                ccap_language,
            } = *subtitle_qol;
            let subtitle_rtl_mode = match subtitle_rtl.as_str() {
                "on" => dcpwizard_core::subtitle::RtlMode::On,
                "off" => dcpwizard_core::subtitle::RtlMode::Off,
                _ => dcpwizard_core::subtitle::RtlMode::Auto,
            };
            let subtitle_opts = dcpwizard_core::subtitle::SubtitleOptions {
                halign: subtitle_halign,
                valign: subtitle_valign,
                vposition: subtitle_vposition,
                zposition: subtitle_zposition,
                rtl: subtitle_rtl_mode,
                wrap_cols: subtitle_wrap,
                font_path: subtitle_font.map(PathBuf::from),
                no_subset: subtitle_no_subset,
            };

            let code = if is_video_file {
                // Full pipeline: video → J2K encode → MXF wrap → DCP
                use postkit::grok_encoder::{self, CompressParams, EncodeProgress};
                use std::sync::Arc;
                use std::sync::atomic::AtomicBool;

                // fail loud if ffmpeg cannot decode the source codec (e.g. APV on
                // an older ffmpeg); the whole pipeline decodes through ffmpeg
                if let Err(e) = dcpwizard_core::probe::ensure_video_decodable(&video_path) {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }

                let _ = std::fs::create_dir_all(&output_dir);
                let j2k_dir = output_dir.join("j2k");
                let _ = std::fs::create_dir_all(&j2k_dir);

                // optional decode-range correction: re-decode the source at a
                // forced range into a lossless intermediate the encode reads from
                let range_src = if let Some(range) = input_range.as_deref() {
                    match normalize_input_range(&video_path, range, &output_dir) {
                        Ok(p) => {
                            tracing::info!("Forcing {range}-range decode of the source");
                            p
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            return;
                        }
                    }
                } else {
                    video_path.clone()
                };

                let mut encode_video_path = range_src.clone();
                // the hdr-lut branch outputs x'y'z' already; every other source is
                // display rgb and needs grok's dcdm transform at encode time
                let mut content_already_xyz = false;
                let hdr_type = dcpwizard_core::dolby_vision::detect_hdr_type(&range_src);
                if hdr_type != postkit::dolby_vision::HdrType::Sdr {
                    let converted = output_dir.join("hdr_to_dci_source.mov");
                    if let Some(lut) = hdr_to_dci_lut.as_ref() {
                        let lut = PathBuf::from(lut);
                        if !lut.is_file() {
                            tracing::error!("HDR-to-DCI LUT not found: {}", lut.display());
                            return;
                        }
                        let opts = postkit::colour::ColourConvertOptions {
                            input: range_src.clone(),
                            output: converted.clone(),
                            source_space: postkit::colour::ColourSpace::Rec2020,
                            target_space: postkit::colour::ColourSpace::Xyz,
                            lut_path: Some(lut),
                        };
                        if let Err(e) = postkit::colour::convert_colour(&opts) {
                            tracing::error!("HDR-to-DCI LUT conversion failed: {e}");
                            return;
                        }
                        content_already_xyz = true;
                    } else if allow_generic_hdr_tonemap {
                        tracing::warn!(
                            "Using generic FFmpeg HDR tone mapping. It is not suitable as a default delivery transform."
                        );
                        if dcpwizard_core::dolby_vision::convert_hdr(
                            &range_src,
                            postkit::dolby_vision::HdrType::Sdr,
                            &converted,
                        ) != 0
                        {
                            return;
                        }
                    } else {
                        tracing::error!(
                            "HDR source requires --hdr-to-dci-lut. Use --allow-generic-hdr-tonemap only for an explicitly accepted generic transform."
                        );
                        return;
                    }
                    encode_video_path = converted;
                }

                tracing::info!("Detected video file input — using grok encoder");

                // Probe video for frame rate and resolution
                let video_info = dcpwizard_core::probe::probe_video(&encode_video_path);
                match dcpwizard_core::probe::video_has_alpha(&encode_video_path) {
                    Ok(true) => {
                        tracing::error!(
                            "Input video has alpha. Composite it over an opaque background before creating a DCP."
                        );
                        std::process::exit(1);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::error!("Cannot determine whether the input has alpha: {error}");
                        std::process::exit(1);
                    }
                }
                let (source_fps, source_needs_audio_pull_up) = video_info
                    .as_ref()
                    .map(|v| dcpwizard_core::hfr::source_rate_to_dcp(v.fps_num, v.fps_den))
                    .unwrap_or((24, false));
                let fps = frame_rate.unwrap_or(source_fps);
                let needs_audio_pull_up = source_needs_audio_pull_up && fps == 24;
                let (mut width, mut height, total_frames) = video_info
                    .as_ref()
                    .map(|v| (v.width, v.height, v.total_frames))
                    .unwrap_or((2048, 1080, 0));

                // reject an illegal fps/resolution combo before the encode runs
                if let Err(e) = dcpwizard_core::hfr::validate_fps_resolution(
                    fps,
                    fourk,
                    std_val == dcpwizard_core::Standard::Smpte,
                ) {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }

                // Apply resolution override
                if fourk {
                    width = 4096;
                    height = 2160;
                } else if twok {
                    width = 2048;
                    height = 1080;
                }

                if let Some(ref info) = video_info {
                    tracing::info!(
                        "Input: {}x{} @ {}/{} fps, ~{} frames",
                        info.width,
                        info.height,
                        info.fps_num,
                        info.fps_den,
                        info.total_frames,
                    );
                }

                // Compute compression ratio from bitrate if specified
                let compression_ratio = match video_bit_rate {
                    Some(mbps) => {
                        dcpwizard_core::encode::bandwidth_to_ratio(width, height, fps, mbps)
                    }
                    None => 10.0,
                };

                let _num_threads = threads.unwrap_or(0); // reserved for future use

                let params = CompressParams {
                    compression_ratio,
                    frame_rate: fps as u16,
                    apply_xyz_transform: !content_already_xyz,
                    ..CompressParams::default()
                };

                let cancel = Arc::new(AtomicBool::new(false));
                let cancel_clone = cancel.clone();
                let _ = ctrlc::set_handler(move || {
                    cancel_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                });

                // persist encode identity so an interrupted run can --resume the
                // J2K frames on disk (dom#344). --resume verifies the params match
                // before reusing them.
                let encode_state = dcpwizard_core::encode_qol::EncodeState {
                    source: video_path.to_string_lossy().to_string(),
                    total_frames: total_frames as u64,
                    fps,
                    width,
                    height,
                    bitrate_mbps: video_bit_rate.unwrap_or(0),
                };
                if resume && let Err(e) = encode_state.check_resumable(&output_dir) {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
                if let Err(e) = encode_state.save(&output_dir) {
                    tracing::warn!("could not save resume state: {e}");
                }

                let encode_start = std::time::Instant::now();
                let result = grok_encoder::encode_video_pipeline_resumable(
                    &encode_video_path,
                    &j2k_dir,
                    &params,
                    total_frames as u64,
                    width,
                    height,
                    &cancel,
                    resume,
                    |p: EncodeProgress| {
                        let percent = if p.total_frames > 0 {
                            (p.frames_encoded as f64 / p.total_frames as f64) * 100.0
                        } else {
                            0.0
                        };
                        // ETA from average fps since the encode started (dom#502):
                        // steadier than the instantaneous rate.
                        let elapsed = encode_start.elapsed().as_secs_f64();
                        let avg_fps = if elapsed > 0.0 {
                            p.frames_encoded as f64 / elapsed
                        } else {
                            0.0
                        };
                        let eta = dcpwizard_core::encode_qol::eta_seconds(
                            p.frames_encoded,
                            p.total_frames,
                            avg_fps,
                        )
                        .map(dcpwizard_core::encode_qol::format_eta)
                        .unwrap_or_else(|| "--:--".to_string());
                        eprint!(
                            "\r[encode] {}/{} frames ({:.0}%) {:.1} fps  avg {:.1}  eta {}   ",
                            p.frames_encoded, p.total_frames, percent, p.fps, avg_fps, eta
                        );
                    },
                );
                eprintln!();

                if !result.success {
                    tracing::error!("Encode failed: {}", result.error);
                    std::process::exit(1);
                }
                tracing::info!("Encoded {} frames", result.frames_encoded);

                // Stereoscopic: encode the right eye into its own dir at the same
                // settings (main input is the left eye).
                let right_eye_dir = if let Some(ref re) = right_eye {
                    let re_path = PathBuf::from(re);
                    let j2k_right = output_dir.join("j2k_right");
                    let _ = std::fs::create_dir_all(&j2k_right);
                    tracing::info!("Encoding right eye: {}", re_path.display());
                    let re_result = grok_encoder::encode_video_pipeline(
                        &re_path,
                        &j2k_right,
                        &params,
                        total_frames as u64,
                        width,
                        height,
                        &cancel,
                        |_p: EncodeProgress| {},
                    );
                    if !re_result.success {
                        tracing::error!("Right-eye encode failed: {}", re_result.error);
                        std::process::exit(1);
                    }
                    Some(j2k_right)
                } else {
                    None
                };

                // Auto-demux audio from video if --audio not provided
                let raw_audio = if let Some(a) = audio {
                    Some(PathBuf::from(a))
                } else {
                    let wav_out = output_dir.join("audio_demux.wav");
                    let demux = std::process::Command::new("ffmpeg")
                        .arg("-y")
                        .arg("-i")
                        .arg(&video_path)
                        .arg("-vn")
                        .arg("-acodec")
                        .arg("pcm_s24le")
                        .arg("-ar")
                        .arg("48000")
                        .arg(&wav_out)
                        .output();
                    match demux {
                        Ok(o) if o.status.success() => {
                            tracing::info!("Demuxed audio: {}", wav_out.display());
                            Some(wav_out)
                        }
                        Ok(_) => {
                            tracing::warn!("No audio stream found in input (or demux failed)");
                            None
                        }
                        Err(e) => {
                            tracing::warn!("ffmpeg not available for audio demux: {e}");
                            None
                        }
                    }
                };
                // W5 audio processing: filename channel routing (a --audio
                // directory), stereo->5.1 upmix, then loudness normalization.
                let audio_path = match prepare_create_audio(
                    raw_audio,
                    upmix.as_deref(),
                    loudness_target.as_deref(),
                    true_peak_ceiling,
                    &output_dir.join("audio_work"),
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("{e}");
                        return;
                    }
                };

                let audio_path = if needs_audio_pull_up {
                    audio_path.map(|input| {
                        let output = output_dir.join("audio_pullup.wav");
                        let result = std::process::Command::new("ffmpeg")
                            .arg("-y")
                            .arg("-i")
                            .arg(&input)
                            .arg("-af")
                            .arg("asetrate=48048,aresample=48000")
                            .arg("-c:a")
                            .arg("pcm_s24le")
                            .arg(&output)
                            .output();
                        match result {
                            Ok(result) if result.status.success() => {
                                tracing::info!("Applied 23.976-to-24 audio pull-up");
                                output
                            }
                            Ok(result) => {
                                tracing::error!(
                                    "23.976-to-24 audio pull-up failed: {}",
                                    String::from_utf8_lossy(&result.stderr)
                                );
                                std::process::exit(1);
                            }
                            Err(error) => {
                                tracing::error!("failed to run ffmpeg for audio pull-up: {error}");
                                std::process::exit(1);
                            }
                        }
                    })
                } else {
                    audio_path
                };

                // sign-language video (ISDCF Doc 13): pack VP9 onto channel 15,
                // overriding the sound track with the combined 16-channel WAV
                let (audio_path, sl_main_channels) = if let Some(slv) = sign_language_video.as_ref()
                {
                    match build_sign_language_audio(
                        slv,
                        audio_path.as_deref(),
                        total_frames as u64,
                        fps,
                        &output_dir,
                    ) {
                        Ok((wav, ch)) => (Some(wav), Some(ch)),
                        Err(e) => {
                            tracing::error!("{e}");
                            return;
                        }
                    }
                } else {
                    (audio_path, None)
                };

                let resolution = if fourk {
                    dcpwizard_core::Resolution::FourK
                } else {
                    dcpwizard_core::Resolution::TwoK
                };
                let ct = content_type
                    .as_deref()
                    .and_then(dcpwizard_core::ContentType::from_abbrev)
                    .unwrap_or_default();

                let (container_width, container_height) =
                    match resolve_container(container.as_deref(), container_dims.as_deref(), fourk)
                    {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::error!("{e}");
                            return;
                        }
                    };

                // reel-split boundaries from --split-at / --split-chapters
                let reel_split_frames = match resolve_reel_splits(
                    split_at.as_deref(),
                    split_chapters,
                    Some(&video_path),
                    fps,
                ) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("{e}");
                        return;
                    }
                };

                let config = dcpwizard_core::dcp::DcpConfig {
                    title,
                    standard: std_val,
                    encrypt,
                    key_out: key_out.map(PathBuf::from),
                    output_dir: output_dir.clone(),
                    frame_rate_num: fps,
                    frame_rate_den: 1,
                    resolution,
                    content_type: ct,
                    container_width,
                    container_height,
                    max_bitrate_mbps: video_bit_rate.unwrap_or(0),
                    j2k_dir: Some(j2k_dir.clone()),
                    audio_path: audio_path.clone(),
                    audio_input_order,
                    subtitle_path: subtitle.clone().map(PathBuf::from),
                    subtitle_language: subtitle_language.clone(),
                    subtitle_opts: subtitle_opts.clone(),
                    ccap_path: ccap.clone().map(PathBuf::from),
                    ccap_language: ccap_language.clone(),
                    reel_length_minutes: reel_length.unwrap_or(0),
                    right_eye_dir: right_eye_dir.clone(),
                    atmos_path: atmos.clone().map(PathBuf::from),
                    hi_channel,
                    vi_channel,
                    stereo_3d: right_eye_dir.is_some(),
                    pad_head: pad_head.clone(),
                    pad_tail: pad_tail.clone(),
                    pad_color: pad_color.clone(),
                    reel_split_frames,
                    sign_language_lang: sign_language_lang.clone(),
                    sign_language_main_channels: sl_main_channels,
                    hdr_dci,
                    signer: package_signer.clone(),
                };
                let code = match versions_specs.as_ref() {
                    Some(v) => dcpwizard_core::versions::create_versioned_dcp(&config, v),
                    None => dcpwizard_core::dcp::create_dcp(&config),
                };

                // Clean up intermediate files
                let _ = std::fs::remove_dir_all(&j2k_dir);
                let _ = std::fs::remove_dir_all(output_dir.join("audio_work"));
                dcpwizard_core::encode_qol::EncodeState::clear(&output_dir);
                if let Some(ref d) = right_eye_dir {
                    let _ = std::fs::remove_dir_all(d);
                }
                if let Some(ref wav) = audio_path
                    && matches!(
                        wav.file_name().and_then(|f| f.to_str()),
                        Some("audio_demux.wav" | "audio_pullup.wav")
                    )
                {
                    let _ = std::fs::remove_file(wav);
                }
                let _ = std::fs::remove_file(output_dir.join("range_corrected.mkv"));
                code
            } else {
                // Input is a J2K directory or image sequence
                let resolution = if fourk {
                    dcpwizard_core::Resolution::FourK
                } else {
                    dcpwizard_core::Resolution::TwoK
                };
                let ct = content_type
                    .as_deref()
                    .and_then(dcpwizard_core::ContentType::from_abbrev)
                    .unwrap_or_default();

                if input_range.is_some() {
                    tracing::error!(
                        "--input-range applies to a video input; a J2K/image sequence carries no decode range"
                    );
                    return;
                }

                let (container_width, container_height) =
                    match resolve_container(container.as_deref(), container_dims.as_deref(), fourk)
                    {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::error!("{e}");
                            return;
                        }
                    };

                let fps = frame_rate.unwrap_or(24);
                let reel_split_frames =
                    match resolve_reel_splits(split_at.as_deref(), split_chapters, None, fps) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::error!("{e}");
                            return;
                        }
                    };

                // W5 audio processing: filename channel routing (a --audio
                // directory), stereo->5.1 upmix, then loudness normalization.
                let work_dir = output_dir.join("audio_work");
                let prepared_audio = match prepare_create_audio(
                    audio.map(PathBuf::from),
                    upmix.as_deref(),
                    loudness_target.as_deref(),
                    true_peak_ceiling,
                    &work_dir,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("{e}");
                        return;
                    }
                };

                // sign-language video (ISDCF Doc 13): pack VP9 onto channel 15.
                // Cover at least the J2K frame count so the sound spans the picture.
                let (audio_path, sl_main_channels) = if let Some(slv) = sign_language_video.as_ref()
                {
                    let frames = std::fs::read_dir(&video_path)
                        .map(|rd| {
                            rd.filter_map(|e| e.ok())
                                .filter(|e| e.path().is_file())
                                .count()
                        })
                        .unwrap_or(0) as u64;
                    match build_sign_language_audio(
                        slv,
                        prepared_audio.as_deref(),
                        frames,
                        fps,
                        &output_dir,
                    ) {
                        Ok((wav, ch)) => (Some(wav), Some(ch)),
                        Err(e) => {
                            tracing::error!("{e}");
                            return;
                        }
                    }
                } else {
                    (prepared_audio, None)
                };

                let config = dcpwizard_core::dcp::DcpConfig {
                    title,
                    standard: std_val,
                    encrypt,
                    key_out: key_out.map(PathBuf::from),
                    output_dir: output_dir.clone(),
                    frame_rate_num: fps,
                    frame_rate_den: 1,
                    resolution,
                    content_type: ct,
                    container_width,
                    container_height,
                    max_bitrate_mbps: video_bit_rate.unwrap_or(0),
                    j2k_dir: Some(video_path),
                    audio_path,
                    audio_input_order,
                    subtitle_path: subtitle.map(PathBuf::from),
                    subtitle_language,
                    subtitle_opts,
                    ccap_path: ccap.map(PathBuf::from),
                    ccap_language,
                    reel_length_minutes: reel_length.unwrap_or(0),
                    stereo_3d: right_eye.is_some(),
                    right_eye_dir: right_eye.map(PathBuf::from),
                    atmos_path: atmos.map(PathBuf::from),
                    hi_channel,
                    vi_channel,
                    pad_head,
                    pad_tail,
                    pad_color,
                    reel_split_frames,
                    sign_language_lang,
                    sign_language_main_channels: sl_main_channels,
                    hdr_dci,
                    signer: package_signer,
                };
                let code = match versions_specs.as_ref() {
                    Some(v) => dcpwizard_core::versions::create_versioned_dcp(&config, v),
                    None => dcpwizard_core::dcp::create_dcp(&config),
                };
                let _ = std::fs::remove_dir_all(&work_dir);
                code
            };

            // shutdown on completion (dom#1394): opt-in, only after a clean run.
            // resolve_shutdown_command already failed loud up front if missing.
            if shutdown_when_done && code == 0 {
                tracing::info!("Encode complete; powering off (--shutdown-when-done)");
                if let Err(e) = dcpwizard_core::encode_qol::run_shutdown() {
                    tracing::error!("{e}");
                    1
                } else {
                    code
                }
            } else {
                code
            }
        }

        Commands::Encode {
            input,
            output,
            bandwidth,
        } => {
            let config = dcpwizard_core::encode::EncodeConfig {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                bandwidth_mbps: bandwidth,
                ..Default::default()
            };
            dcpwizard_core::encode::encode_j2k(&config)
        }

        Commands::Pipeline {
            input,
            title,
            output,
            audio,
            ratio,
            fps,
            input_range,
            split_chapters,
        } => {
            use postkit::encode::{StreamEncodeOptions, stream_encode_subprocess};
            use std::sync::Arc;
            use std::sync::atomic::AtomicBool;

            let input_path = PathBuf::from(&input);
            let output_dir = PathBuf::from(&output);

            if !input_path.exists() {
                tracing::error!("Input not found: {input}");
                std::process::exit(1);
            }

            let _ = std::fs::create_dir_all(&output_dir);
            let j2k_dir = output_dir.join("j2k");
            let _ = std::fs::create_dir_all(&j2k_dir);

            tracing::info!("Pipeline (subprocess Grok): {} -> {}", input, output);

            // reel-split boundaries from the source's chapter marks
            let reel_split_frames = if split_chapters {
                match video_chapter_boundaries(&input_path, fps) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            } else {
                Vec::new()
            };

            // optional decode-range correction into a lossless intermediate
            let encode_input = if let Some(range) = input_range.as_deref() {
                match normalize_input_range(&input_path, range, &output_dir) {
                    Ok(p) => {
                        tracing::info!("Forcing {range}-range decode of the source");
                        p
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            } else {
                input_path.clone()
            };

            let grk_bin = std::env::var("GRK_COMPRESS_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_default();
                    PathBuf::from(home).join("bin/grok/bin/grk_compress")
                });

            let opts = StreamEncodeOptions {
                input: encode_input.clone(),
                output_dir: j2k_dir.clone(),
                compression_ratio: ratio,
                num_resolutions: 6,
                codeblock_size: 32,
                progression: "CPRL".to_string(),
                fps,
                compressor_path: grk_bin,
                lib_dir: None,
            };

            let cancel = Arc::new(AtomicBool::new(false));

            // Handle Ctrl+C
            let cancel_clone = cancel.clone();
            let _ = ctrlc::set_handler(move || {
                cancel_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            });

            let result = stream_encode_subprocess(&opts, &cancel, |p| {
                let percent = if p.total_frames > 0 {
                    (p.frame as f64 / p.total_frames as f64) * 100.0
                } else {
                    0.0
                };
                eprint!(
                    "\r[encode] {}/{} frames ({:.0}%) {:.1} fps   ",
                    p.frame, p.total_frames, percent, p.fps
                );
            });
            eprintln!();

            if !result.success {
                tracing::error!("Encode failed: {}", result.error);
                1
            } else {
                tracing::info!("Encoded {} frames", result.frames_encoded);

                // Auto-demux audio from video if --audio not provided
                let audio_path = if let Some(a) = audio {
                    Some(PathBuf::from(a))
                } else {
                    let wav_out = output_dir.join("audio_demux.wav");
                    let demux = std::process::Command::new("ffmpeg")
                        .arg("-y")
                        .arg("-i")
                        .arg(&input_path)
                        .arg("-vn")
                        .arg("-acodec")
                        .arg("pcm_s24le")
                        .arg("-ar")
                        .arg("48000")
                        .arg(&wav_out)
                        .output();
                    match demux {
                        Ok(o) if o.status.success() => {
                            tracing::info!("Demuxed audio: {}", wav_out.display());
                            Some(wav_out)
                        }
                        Ok(_) => {
                            tracing::warn!("No audio stream in input (or demux failed)");
                            None
                        }
                        Err(e) => {
                            tracing::warn!("ffmpeg not available for audio demux: {e}");
                            None
                        }
                    }
                };

                // Package
                let config = dcpwizard_core::dcp::DcpConfig {
                    title,
                    standard: dcpwizard_core::Standard::Smpte,
                    output_dir: output_dir.clone(),
                    frame_rate_num: fps,
                    frame_rate_den: 1,
                    j2k_dir: Some(j2k_dir.clone()),
                    audio_path: audio_path.clone(),
                    audio_input_order: dcpwizard_core::mxf_wrap::AudioInputOrder::Canonical51,
                    reel_split_frames: reel_split_frames.clone(),
                    ..Default::default()
                };
                let code = dcpwizard_core::dcp::create_dcp(&config);

                // Clean up intermediate files
                let _ = std::fs::remove_dir_all(&j2k_dir);
                if let Some(ref wav) = audio_path
                    && wav.file_name().and_then(|f| f.to_str()) == Some("audio_demux.wav")
                {
                    let _ = std::fs::remove_file(wav);
                }
                let _ = std::fs::remove_file(output_dir.join("range_corrected.mkv"));
                code
            }
        }

        Commands::Transcode {
            input,
            output,
            format,
            bit_depth,
        } => {
            // 8/10/16-bit packed RGB pixel formats ffmpeg understands for these codecs
            let pixel_format = match bit_depth {
                8 => "rgb24",
                10 | 16 => "rgb48le",
                other => {
                    tracing::error!("unsupported bit depth {other}; use 8, 10 or 16");
                    std::process::exit(1);
                }
            };
            let config = dcpwizard_core::transcode::TranscodeConfig {
                input_file: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                image_format: format,
                pixel_format: pixel_format.to_string(),
                ..Default::default()
            };
            dcpwizard_core::transcode::transcode_to_sequence(&config)
        }

        Commands::TranscodeDcp {
            input,
            output,
            video_bit_rate,
            width,
            height,
            kdm,
            recipient_key,
            keys,
        } => {
            let config = dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                target_bitrate_mbps: video_bit_rate,
                target_width: width.unwrap_or(0),
                target_height: height.unwrap_or(0),
                kdm: kdm.map(PathBuf::from),
                recipient_key: recipient_key.map(PathBuf::from),
                keys: keys.map(PathBuf::from),
            };
            dcpwizard_core::j2k_transcode::transcode_dcp(&config)
        }

        Commands::Decrypt {
            input,
            output,
            kdm,
            recipient_key,
            keys,
        } => {
            let config = dcpwizard_core::decrypt::DcpDecryptConfig {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                kdm: kdm.map(PathBuf::from),
                recipient_key: recipient_key.map(PathBuf::from),
                keys: keys.map(PathBuf::from),
            };
            dcpwizard_core::decrypt::decrypt_dcp(&config)
        }

        Commands::Verify {
            dcp_dir,
            no_hash_check,
            no_picture_check,
            strict,
            output,
            quiet,
        } => {
            let result = dcpwizard_core::verify::verify_dcp_with_options(
                &PathBuf::from(&dcp_dir),
                &dcpwizard_core::verify::VerifyCliOptions {
                    skip_hash_check: no_hash_check,
                    skip_picture_check: no_picture_check,
                    strict,
                },
            );

            if let Some(ref out_path) = output
                && let Err(e) =
                    dcpwizard_core::verify::write_verify_report(&result, Path::new(out_path))
            {
                tracing::error!("Failed to write report: {e}");
                std::process::exit(1);
            }

            if !quiet {
                if result.valid {
                    tracing::info!("DCP verification PASSED");
                } else {
                    for e in &result.errors {
                        tracing::error!("{e}");
                    }
                }
                for w in &result.warnings {
                    tracing::warn!("{w}");
                }
                for i in &result.info {
                    tracing::info!("{i}");
                }
            }

            if result.valid { 0 } else { 1 }
        }

        Commands::Info { dcp_dir } => {
            match dcpwizard_core::info::inspect_dcp(&PathBuf::from(dcp_dir)) {
                Ok(info) => {
                    tracing::info!("Title: {}", info.title);
                    tracing::info!("Standard: {}", info.standard);
                    tracing::info!("Frame rate: {}", info.frame_rate);
                    tracing::info!("Duration: {} frames", info.duration_frames);
                    tracing::info!("Reels: {}", info.reel_count);
                    tracing::info!("Encrypted: {}", if info.encrypted { "yes" } else { "no" });
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::Kdm {
            cpl_id,
            content_title,
            cert,
            signer_cert,
            signer_key,
            signer_chain,
            output,
            valid_from,
            valid_to,
            template,
            templates_file,
            history_file,
            email_to,
            smtp_config,
            keys,
            format,
            annotation,
        } => {
            let format = match dcpwizard_core::kdm::parse_format(&format) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let (valid_from, valid_to) =
                match resolve_window(valid_from, valid_to, template, templates_file) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            let content_keys = match keys {
                Some(path) => {
                    match dcpwizard_core::kdm::load_content_keys(&PathBuf::from(path), &cpl_id) {
                        Ok(k) => k,
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                }
                None => Vec::new(),
            };
            let out_path = PathBuf::from(&output);
            let title = content_title.clone();
            let code = dcpwizard_core::kdm::generate_kdm(
                cpl_id,
                content_title,
                PathBuf::from(cert),
                PathBuf::from(signer_cert),
                PathBuf::from(signer_key),
                signer_chain.into_iter().map(PathBuf::from).collect(),
                valid_from,
                valid_to,
                content_keys,
                out_path.clone(),
                format,
                annotation,
                Some(history_path(history_file)),
            );
            if code == 0 {
                if let Some(cfg_path) = smtp_config {
                    match send_kdm_email(&cfg_path, "", &title, &email_to, &[out_path]) {
                        Ok(()) => 0,
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                    }
                } else {
                    0
                }
            } else {
                code
            }
        }

        Commands::KdmRewrap {
            dkdm,
            dkdm_key,
            cert,
            signer_cert,
            signer_key,
            signer_chain,
            valid_from,
            valid_to,
            output,
        } => dcpwizard_core::kdm::rewrap_dkdm(
            PathBuf::from(dkdm),
            PathBuf::from(dkdm_key),
            PathBuf::from(cert),
            PathBuf::from(signer_cert),
            PathBuf::from(signer_key),
            signer_chain.into_iter().map(PathBuf::from).collect(),
            valid_from,
            valid_to,
            PathBuf::from(output),
        ),

        Commands::Copy { src, dst } => {
            dcpwizard_core::copy_drive::copy_to_drive(&PathBuf::from(src), &PathBuf::from(dst))
        }

        Commands::FormatDrive {
            target,
            fs,
            label,
            yes,
            image,
        } => {
            let fs = match dcpwizard_core::disk::ExtFs::parse(&fs) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("{e}");
                    return;
                }
            };
            match dcpwizard_core::disk::format_drive(
                &PathBuf::from(&target),
                fs,
                label.as_deref(),
                yes,
                image,
            ) {
                Ok(()) => {
                    tracing::info!("Formatted {target} as {fs:?}");
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::CheckDrive { target } => {
            match dcpwizard_core::disk::check_drive(&PathBuf::from(&target)) {
                Ok(info) => {
                    tracing::info!(
                        "{target}: fs={} label={}",
                        info.fstype.as_deref().unwrap_or("unknown"),
                        info.label.as_deref().unwrap_or("(none)")
                    );
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::Loudness { audio_file } => {
            let result = dcpwizard_core::loudness::measure_loudness(&PathBuf::from(audio_file));
            if result.success {
                tracing::info!("Integrated: {:.1} LUFS", result.integrated_lufs);
                tracing::info!("True Peak: {:.1} dBTP", result.true_peak_dbtp);
                tracing::info!("LRA: {:.1} LU", result.range_lu);
                0
            } else {
                tracing::error!("{}", result.error);
                1
            }
        }

        Commands::Crossfade {
            a,
            b,
            output,
            overlap,
        } => match postkit::crossfade::crossfade_join_wav(
            &PathBuf::from(a),
            &PathBuf::from(b),
            &PathBuf::from(&output),
            overlap,
        ) {
            Ok(()) => {
                tracing::info!("Wrote crossfade join: {output}");
                0
            }
            Err(e) => {
                tracing::error!("crossfade failed: {e}");
                1
            }
        },

        Commands::MidSideDecode {
            input,
            output,
            mid,
            side,
        } => match postkit::mid_side::decode_mid_side_wav(
            &PathBuf::from(input),
            &PathBuf::from(&output),
            mid,
            side,
        ) {
            Ok(()) => {
                tracing::info!("Wrote mid-side decoded WAV: {output}");
                0
            }
            Err(e) => {
                tracing::error!("mid-side decode failed: {e}");
                1
            }
        },

        Commands::Report { dcp, output } => {
            dcpwizard_core::report::generate_report(&PathBuf::from(dcp), &PathBuf::from(output))
        }

        Commands::Serve { bind } => dcpwizard_core::rest_api::start_rest_api(&bind),

        Commands::Watch { dir, webhook_url } => {
            let webhook = webhook_url.map(|url| postkit::webhook::WebhookConfig {
                url,
                ..Default::default()
            });
            dcpwizard_core::watch::watch_directory(
                &PathBuf::from(dir),
                std::time::Duration::from_secs(5),
                &|| false,
                |p| {
                    tracing::info!("New DCP detected: {}", p.display());
                    if let Some(ref cfg) = webhook {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                        let evt = postkit::webhook::WebhookEvent {
                            event_type: "dcp.detected".into(),
                            job_id: name.to_string(),
                            payload_json: postkit::webhook::build_job_completed_payload(
                                name, p, 0.0,
                            ),
                            timestamp: String::new(),
                        };
                        let res = postkit::webhook::send_webhook(cfg, &evt);
                        if !res.success {
                            tracing::warn!("webhook delivery failed: {}", res.error);
                        }
                    }
                },
            );
            0
        }

        Commands::Export {
            input,
            output,
            format,
            crf,
            audio,
        } => {
            use dcpwizard_core::export::{ExportConfig, ExportFormat, export_dcp};
            let fmt = match format.to_lowercase().as_str() {
                "prores" => ExportFormat::ProRes,
                "h264" | "x264" | "avc" => ExportFormat::H264,
                "h265" | "hevc" | "x265" => ExportFormat::H265,
                "dnxhr" | "dnxhd" => ExportFormat::DnxHr,
                "image-sequence" | "images" | "png" => ExportFormat::ImageSequence,
                other => {
                    tracing::error!(
                        "unknown export format '{other}'; use prores, h264, h265, dnxhr or image-sequence"
                    );
                    std::process::exit(1);
                }
            };
            let config = ExportConfig {
                input_mxf: PathBuf::from(input),
                output_path: PathBuf::from(output),
                format: fmt,
                quality_crf: crf,
                audio_mxf: audio.map(PathBuf::from),
            };
            export_dcp(&config)
        }

        Commands::Completion { shell } => {
            print!(
                "{}",
                dcpwizard_core::shell_completion::generate_completion(&shell, "dcpwizard")
            );
            0
        }

        Commands::Daemon => {
            let addr = dcpwizard_core::job_queue::daemon_addr();
            println!("Starting dcpwizard daemon on {addr}...");
            let queue = dcpwizard_core::job_queue::JobQueue::new();
            dcpwizard_core::job_queue::start_daemon_ipc(&queue)
        }

        Commands::SubtitleConvert {
            input,
            output,
            language,
            fps,
            vposition,
        } => {
            let input_path = PathBuf::from(&input);
            let output_path = PathBuf::from(&output);
            if !input_path.exists() {
                tracing::error!("Input file not found: {input}");
                std::process::exit(1);
            }
            match dcpwizard_core::subtitle::convert_srt_to_dcp_xml(
                &input_path,
                &output_path,
                &language,
                fps,
                vposition,
            ) {
                Ok(()) => {
                    tracing::info!(
                        "Converted {} -> {} (lang={}, fps={}, vposition={})",
                        input,
                        output,
                        language,
                        fps,
                        vposition
                    );
                    0
                }
                Err(e) => {
                    tracing::error!("Subtitle conversion failed: {e}");
                    1
                }
            }
        }

        Commands::SubtitleExtract { input, output } => {
            let input_path = PathBuf::from(&input);
            let output_path = PathBuf::from(&output);
            if !input_path.exists() {
                tracing::error!("Input not found: {input}");
                std::process::exit(1);
            }
            match dcpwizard_core::subtitle_extract::extract(&input_path, &output_path) {
                Ok(()) => {
                    tracing::info!("Extracted subtitles -> {output}");
                    0
                }
                Err(e) => {
                    tracing::error!("Subtitle extraction failed: {e}");
                    1
                }
            }
        }
        Commands::SubtitleEdit {
            input,
            output,
            list,
            shift_ms,
            index,
            text,
            set_start_ms,
            set_end_ms,
            fps,
        } => {
            use dcpwizard_core::subtitle_edit as se;
            let input_path = PathBuf::from(&input);
            let mut cues = match se::load(&input_path, fps) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to load subtitles: {e}");
                    std::process::exit(1);
                }
            };
            if list {
                for (i, c) in cues.iter().enumerate() {
                    println!("{}", se::summary_line(i + 1, c));
                }
                0
            } else {
                if let Some(delta) = shift_ms {
                    se::shift_all(&mut cues, delta);
                }
                if let Some(idx) = index {
                    if let Some(t) = text.as_deref()
                        && let Err(e) = se::set_text(&mut cues, idx, t)
                    {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                    match (set_start_ms, set_end_ms) {
                        (Some(s), Some(e)) => {
                            if let Err(err) = se::set_timing(&mut cues, idx, s, e) {
                                tracing::error!("{err}");
                                std::process::exit(1);
                            }
                        }
                        (None, None) => {}
                        _ => {
                            tracing::error!(
                                "--set-start-ms and --set-end-ms must be given together"
                            );
                            std::process::exit(1);
                        }
                    }
                } else if text.is_some() || set_start_ms.is_some() || set_end_ms.is_some() {
                    tracing::error!("--text / --set-start-ms / --set-end-ms need --index");
                    std::process::exit(1);
                }
                let Some(out) = output else {
                    tracing::error!("--output is required to write edits (use --list to inspect)");
                    std::process::exit(1);
                };
                match std::fs::write(&out, se::format_srt(&cues)) {
                    Ok(()) => {
                        tracing::info!("Wrote {} cues -> {out}", cues.len());
                        0
                    }
                    Err(e) => {
                        tracing::error!("Failed to write {out}: {e}");
                        1
                    }
                }
            }
        }

        Commands::Burnin {
            input,
            subtitles,
            output,
            font_size,
        } => {
            let opts = dcpwizard_core::burnin::BurninOptions {
                input: PathBuf::from(&input),
                output: PathBuf::from(&output),
                subtitle_file: Some(PathBuf::from(&subtitles)),
                font_size,
                position: "bottom".to_string(),
                ..Default::default()
            };
            match dcpwizard_core::burnin::burnin(&opts) {
                Ok(()) => {
                    tracing::info!("Burned subtitles into: {output}");
                    0
                }
                Err(e) => {
                    tracing::error!("Burn-in failed: {e}");
                    1
                }
            }
        }

        Commands::Convert {
            input,
            output,
            target,
            method,
        } => {
            // Parse target resolution
            let (tw, th) = match target.to_lowercase().as_str() {
                "2k-scope" => (2048, 858),
                "2k-flat" => (1998, 1080),
                "2k-full" => (2048, 1080),
                "4k-scope" => (4096, 1716),
                "4k-flat" => (3996, 2160),
                "4k-full" => (4096, 2160),
                _ => {
                    tracing::error!(
                        "Unknown target: {target}. Use: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full"
                    );
                    std::process::exit(1);
                }
            };

            let vf = match method.to_lowercase().as_str() {
                "letterbox" => format!(
                    "scale={tw}:{th}:force_original_aspect_ratio=decrease,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2"
                ),
                "crop" => {
                    format!("scale={tw}:{th}:force_original_aspect_ratio=increase,crop={tw}:{th}")
                }
                "scale" => format!("scale={tw}:{th}"),
                _ => {
                    tracing::error!("Unknown method: {method}. Use: letterbox, crop, or scale");
                    std::process::exit(1);
                }
            };

            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-i")
                .arg(&input)
                .arg("-vf")
                .arg(&vf)
                .arg("-c:a")
                .arg("copy")
                .arg(&output)
                .status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!("Converted to {target} ({method}): {output} ({}x{})", tw, th);
                    0
                }
                Ok(s) => {
                    tracing::error!("ffmpeg exited with code {}", s.code().unwrap_or(-1));
                    1
                }
                Err(e) => {
                    tracing::error!("Failed to run ffmpeg: {e}");
                    1
                }
            }
        }

        Commands::Dcdm {
            input,
            output,
            colour_space,
            lut,
        } => {
            let cs = parse_colour_space(&colour_space);
            let opts = postkit::dcdm::DcdmOptions {
                input_dir: std::path::PathBuf::from(&input),
                output_dir: std::path::PathBuf::from(&output),
                encoding: postkit::dcdm::DcdmColourEncoding::Xyz12Bit,
                width: 0,
                height: 0,
                fps_num: 24,
                fps_den: 1,
                colour_space: format!("{cs:?}"),
                target: postkit::dcdm::DcdmTarget::Xyz,
                lut_path: lut.map(std::path::PathBuf::from).unwrap_or_default(),
            };
            let result = postkit::dcdm::create_dcdm(&opts);
            if result.success {
                tracing::info!("DCDM created: {} frames written", result.frames_written);
                0
            } else {
                tracing::error!("DCDM creation failed: {}", result.error);
                1
            }
        }

        Commands::Colour {
            input,
            output,
            source,
            target,
            lut,
        } => {
            // X'Y'Z' (DCDM) and P3-D65 are dcdm-module transforms, not ffmpeg
            // colorspace-filter targets; route them through the real
            // Rec.709/P3/Rec.2020 transform (fails loud on an unsupported source).
            if let Some(dcdm_target) = parse_dcdm_target(&target) {
                let opts = postkit::dcdm::DcdmOptions {
                    input_dir: std::path::PathBuf::from(&input),
                    output_dir: std::path::PathBuf::from(&output),
                    encoding: postkit::dcdm::DcdmColourEncoding::Xyz12Bit,
                    width: 0,
                    height: 0,
                    fps_num: 24,
                    fps_den: 1,
                    colour_space: source.clone(),
                    target: dcdm_target,
                    lut_path: lut.map(std::path::PathBuf::from).unwrap_or_default(),
                };
                let result = postkit::dcdm::create_dcdm(&opts);
                if result.success {
                    tracing::info!(
                        "Colour converted {source} -> {target}: {} frames written",
                        result.frames_written
                    );
                    0
                } else {
                    tracing::error!("Colour conversion failed: {}", result.error);
                    1
                }
            } else {
                let opts = postkit::colour::ColourConvertOptions {
                    input: std::path::PathBuf::from(&input),
                    output: std::path::PathBuf::from(&output),
                    source_space: parse_colour_space(&source),
                    target_space: parse_colour_space(&target),
                    lut_path: lut.map(std::path::PathBuf::from),
                };
                match postkit::colour::convert_colour(&opts) {
                    Ok(()) => {
                        tracing::info!("Colour converted {source} -> {target}: {output}");
                        0
                    }
                    Err(e) => {
                        tracing::error!("Colour conversion failed: {e}");
                        1
                    }
                }
            }
        }

        Commands::Conform {
            input,
            media_dir,
            output,
            json,
        } => match postkit::conform::parse_timeline(std::path::Path::new(&input)) {
            Err(e) => {
                tracing::error!("Timeline parse failed: {e}");
                1
            }
            Ok(timeline) => {
                if let Some(media_dir) = media_dir {
                    run_conform_assembly(&input, &timeline, &media_dir, output.as_deref())
                } else if json {
                    println!("{}", serde_json::to_string_pretty(&timeline).unwrap());
                    0
                } else {
                    println!("Timeline: {}", timeline.title);
                    println!("Format: {:?}", timeline.format);
                    println!("Frame rate: {}", timeline.frame_rate);
                    println!("Events: {}", timeline.events.len());
                    for (i, evt) in timeline.events.iter().enumerate() {
                        println!("  [{i}] {} -> {}", evt.source_in, evt.source_out);
                    }
                    0
                }
            }
        },

        Commands::Ingest {
            source,
            output,
            format,
            colour_space,
            lut,
        } => {
            if let Some(ref l) = lut
                && !std::path::Path::new(l).is_file()
            {
                tracing::error!("LUT file not found: {l}");
                std::process::exit(1);
            }
            let opts = postkit::ingest::IngestOptions {
                source: std::path::PathBuf::from(&source),
                output_dir: std::path::PathBuf::from(&output),
                output_format: format,
                colour_space,
                debayer_quality: 3,
                apply_lut: lut.is_some(),
                lut_path: lut.map(std::path::PathBuf::from).unwrap_or_default(),
                gpu_device: -1,
            };
            postkit::ingest::ingest(&opts)
        }

        Commands::FrameExtract {
            input,
            frame,
            output,
        } => postkit::preview::extract_frame(
            std::path::Path::new(&input),
            frame,
            std::path::Path::new(&output),
        ),

        Commands::DvInject { input, rpu, output } => {
            let opts = postkit::dolby_vision::DolbyVisionOptions {
                input: std::path::PathBuf::from(&input),
                rpu_file: std::path::PathBuf::from(&rpu),
                profile: postkit::dolby_vision::DolbyVisionProfile::Profile8,
                output: std::path::PathBuf::from(&output),
                embed_rpu: true,
            };
            postkit::dolby_vision::inject_dolby_vision(&opts)
        }

        Commands::Hdr10Inject {
            input,
            output,
            max_cll,
            max_fall,
        } => {
            let opts = postkit::dolby_vision::HdrMetadataOptions {
                input: std::path::PathBuf::from(&input),
                hdr_type: postkit::dolby_vision::HdrType::Hdr10,
                hdr10: postkit::dolby_vision::Hdr10Metadata {
                    display_primaries_rx: 13250,
                    display_primaries_ry: 34500,
                    display_primaries_gx: 7500,
                    display_primaries_gy: 3000,
                    display_primaries_bx: 34000,
                    display_primaries_by: 16000,
                    white_point_x: 15635,
                    white_point_y: 16450,
                    max_luminance: 10000000,
                    min_luminance: 1,
                    max_cll,
                    max_fall,
                },
                dolby_vision_xml: std::path::PathBuf::new(),
                output: std::path::PathBuf::from(&output),
            };
            postkit::dolby_vision::inject_hdr10_metadata(&opts)
        }

        Commands::Watermark {
            input,
            output,
            payload,
        } => {
            match dcpwizard_core::watermark::embed_watermark(
                PathBuf::from(&input),
                PathBuf::from(&output),
                &payload,
            ) {
                Ok(()) => {
                    tracing::info!("Visible watermark burned into: {output}");
                    0
                }
                Err(e) => {
                    tracing::error!("Watermark failed: {e}");
                    1
                }
            }
        }

        Commands::Certificate { action } => match action {
            CertAction::Chain {
                organization,
                output,
            } => {
                let output_dir = PathBuf::from(&output);
                dcpwizard_core::certificate::generate_chain(&organization, &output_dir)
            }
            CertAction::Generate {
                cert_type,
                cn,
                organization,
                output_cert,
                output_key,
                issuer_cert,
                issuer_key,
                key_bits,
                validity_days,
            } => {
                let ct = match cert_type.to_lowercase().as_str() {
                    "root" => dcpwizard_core::certificate::CertType::Root,
                    "intermediate" => dcpwizard_core::certificate::CertType::Intermediate,
                    "leaf" => dcpwizard_core::certificate::CertType::Leaf,
                    _ => dcpwizard_core::certificate::CertType::Signer,
                };
                let opts = dcpwizard_core::certificate::CertOptions {
                    cert_type: ct,
                    common_name: cn,
                    organization,
                    output_cert: PathBuf::from(&output_cert),
                    output_key: PathBuf::from(&output_key),
                    issuer_cert: issuer_cert.map(PathBuf::from).unwrap_or_default(),
                    issuer_key: issuer_key.map(PathBuf::from).unwrap_or_default(),
                    key_bits,
                    validity_days,
                    ..Default::default()
                };
                dcpwizard_core::certificate::generate_certificate(&opts)
            }
            CertAction::Inspect { cert_file } => {
                let info = dcpwizard_core::certificate::read_certificate(Path::new(&cert_file));
                println!("Subject CN:  {}", info.subject_cn);
                println!("Issuer CN:   {}", info.issuer_cn);
                println!("Serial:      {}", info.serial);
                println!("Not Before:  {}", info.not_before);
                println!("Not After:   {}", info.not_after);
                println!("Key Size:    {} bits", info.key_bits);
                println!("Is CA:       {}", info.is_ca);
                println!("Expired:     {}", info.is_expired);
                println!("Thumbprint:  {}", info.thumbprint_sha1);
                0
            }
        },

        Commands::Batch { action } => {
            use dcpwizard_core::job_queue::{IpcRequest, IpcResponse, send_ipc_request};

            match action {
                BatchAction::List => match send_ipc_request(&IpcRequest::List) {
                    Ok(IpcResponse::Jobs(jobs)) => {
                        if jobs.is_empty() {
                            println!("No jobs in queue");
                        } else {
                            println!(
                                "{:<38} {:<12} {:<10} {:<14} Message",
                                "ID", "State", "Progress", "Type"
                            );
                            for j in &jobs {
                                println!(
                                    "{:<38} {:?} {:<10}% {:?} {}",
                                    j.id, j.state, j.progress_percent, j.job_type, j.message
                                );
                            }
                        }
                        0
                    }
                    Ok(IpcResponse::Error(e)) => {
                        tracing::error!("{e}");
                        1
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        1
                    }
                    _ => 1,
                },
                BatchAction::Add { r#type, params } => {
                    if !dcpwizard_core::job_queue::is_daemon_running() {
                        tracing::error!("Daemon is not running. Start it with: dcpwizard daemon");
                        std::process::exit(1);
                    }
                    let job_type = match r#type.as_str() {
                        "create-dcp" => dcpwizard_core::job_queue::JobType::CreateDcp,
                        "verify-dcp" => dcpwizard_core::job_queue::JobType::VerifyDcp,
                        "export-dcp" => dcpwizard_core::job_queue::JobType::ExportDcp,
                        "import-video" => dcpwizard_core::job_queue::JobType::ImportVideo,
                        "encode-j2k" => dcpwizard_core::job_queue::JobType::EncodeJ2k,
                        "wrap-mxf" => dcpwizard_core::job_queue::JobType::WrapMxf,
                        "copy-to-drive" => dcpwizard_core::job_queue::JobType::CopyToDrive,
                        other => {
                            tracing::error!("Unknown job type: {other}");
                            std::process::exit(1);
                        }
                    };
                    match send_ipc_request(&IpcRequest::Submit { job_type, params }) {
                        Ok(IpcResponse::Submitted { id }) => {
                            println!("Submitted job {id}");
                            0
                        }
                        Ok(IpcResponse::Error(e)) => {
                            tracing::error!("{e}");
                            1
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                        _ => 1,
                    }
                }
                BatchAction::Cancel { id } => {
                    match send_ipc_request(&IpcRequest::Cancel { id: id.clone() }) {
                        Ok(IpcResponse::Cancelled(true)) => {
                            println!("Cancelled job {id}");
                            0
                        }
                        Ok(IpcResponse::Cancelled(false)) => {
                            println!("Could not cancel job {id}");
                            1
                        }
                        Ok(IpcResponse::Error(e)) => {
                            tracing::error!("{e}");
                            1
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                        _ => 1,
                    }
                }
            }
        }

        Commands::KdmBatch {
            cpl_id,
            content_title,
            certs,
            cert_dir,
            cinemas,
            screens,
            db,
            signer_cert,
            signer_key,
            signer_chain,
            output_dir,
            valid_from,
            valid_to,
            template,
            templates_file,
            history_file,
            email_to,
            smtp_config,
            email_only_additional,
            keys,
            format,
        } => run_kdm_batch(KdmBatchArgs {
            cpl_id,
            content_title,
            certs,
            cert_dir,
            cinemas,
            screens,
            db,
            signer_cert,
            signer_key,
            signer_chain,
            output_dir,
            valid_from,
            valid_to,
            template,
            templates_file,
            history_file,
            email_to,
            smtp_config,
            email_only_additional,
            keys,
            format,
        }),

        Commands::Cinema { db, action } => run_cinema(db, action),

        Commands::KdmHistory {
            history_file,
            title,
            recipient,
            since,
            until,
        } => run_kdm_history(history_file, title, recipient, since, until),

        Commands::KdmTemplate {
            templates_file,
            action,
        } => run_kdm_template(templates_file, action),

        Commands::CertFetch {
            vendor,
            serial,
            device_type,
            user,
            password,
            output,
        } => run_cert_fetch(vendor, serial, device_type, user, password, output),

        Commands::Trailer {
            content,
            output,
            title,
            rating,
            rating_system,
            band,
            countdown,
            fps,
        } => {
            let opts = postkit::trailer::TrailerOptions {
                content_dir: PathBuf::from(&content),
                audio_file: PathBuf::new(),
                output_dir: PathBuf::from(&output),
                title,
                rating,
                rating_system: match rating_system.to_lowercase().as_str() {
                    "bbfc" => postkit::trailer::RatingSystem::Bbfc,
                    "fsk" => postkit::trailer::RatingSystem::Fsk,
                    "custom" => postkit::trailer::RatingSystem::Custom,
                    _ => postkit::trailer::RatingSystem::Mpaa,
                },
                band: match band.to_lowercase().as_str() {
                    "red" => postkit::trailer::TrailerBand::Red,
                    "yellow" => postkit::trailer::TrailerBand::Yellow,
                    _ => postkit::trailer::TrailerBand::Green,
                },
                countdown_seconds: countdown,
                fps_num: fps,
                fps_den: 1,
            };
            let result = postkit::trailer::package_trailer(&opts);
            if !result.success {
                tracing::error!("Trailer packaging failed: {}", result.error);
                1
            } else {
                tracing::info!(
                    "Trailer packaged: {} ({})",
                    result.output_dir.display(),
                    result.output_file.display()
                );
                // route the packaged mp4 through the encode + create path so the
                // deliverable is a real DCP, not just an mp4.
                trailer_to_dcp(&result.output_file, &result.output_dir, fps)
            }
        }

        Commands::Markers {
            frames,
            markers,
            fps,
            xml,
        } => {
            let entries = if markers.is_empty() {
                dcpwizard_core::markers::default_markers(frames)
            } else {
                let mut out = Vec::with_capacity(markers.len());
                for arg in &markers {
                    match dcpwizard_core::markers::parse_marker_arg(arg, fps, frames) {
                        Ok(e) => out.push(e),
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                }
                out
            };
            if xml {
                println!("{}", dcpwizard_core::markers::markers_to_xml(&entries));
            } else if entries.is_empty() {
                println!("No markers (composition length is 0 frames)");
            } else {
                for m in &entries {
                    println!("{}\t{}", m.marker.label(), m.frame);
                }
            }
            0
        }

        Commands::Accessibility { dcp_dir, standard } => {
            let std_val = match standard.to_lowercase().as_str() {
                "eaa" => postkit::accessibility::AccessibilityStandard::Eaa,
                "aoda" => postkit::accessibility::AccessibilityStandard::Aoda,
                "ofcom" => postkit::accessibility::AccessibilityStandard::Ofcom,
                _ => postkit::accessibility::AccessibilityStandard::Cvaa,
            };
            let result =
                dcpwizard_core::accessibility::check_accessibility(Path::new(&dcp_dir), std_val);
            println!("Standard:  {:?}", result.standard);
            println!("Compliant: {}", result.compliant);
            println!("Errors:    {}", result.errors);
            println!("Warnings:  {}", result.warnings);
            for f in &result.findings {
                println!(
                    "  [{:?}] {} ({:?}): {}",
                    f.severity, f.rule_id, f.track_type, f.description
                );
            }
            if result.compliant { 0 } else { 1 }
        }

        Commands::Webhook {
            url,
            event,
            job_id,
            secret,
            payload,
        } => {
            let config = postkit::webhook::WebhookConfig {
                url,
                secret,
                ..Default::default()
            };
            let result = if payload.is_empty() && event == "ping" {
                postkit::webhook::test_webhook(&config)
            } else {
                let evt = postkit::webhook::WebhookEvent {
                    event_type: event,
                    job_id,
                    payload_json: payload,
                    timestamp: String::new(),
                };
                postkit::webhook::send_webhook(&config, &evt)
            };
            if result.success {
                tracing::info!(
                    "Webhook delivered (HTTP {}, {} attempt(s))",
                    result.http_status,
                    result.attempts
                );
                0
            } else {
                tracing::error!(
                    "Webhook failed after {} attempt(s): {}",
                    result.attempts,
                    result.error
                );
                1
            }
        }

        Commands::Version { action } => match action {
            VersionAction::Record {
                db,
                package_uuid,
                title,
                version,
                destination,
                method,
                verified,
            } => {
                let mut tracker = dcpwizard_core::version_tracker::VersionTracker::new();
                if !tracker.open(Path::new(&db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                let record = dcpwizard_core::version_tracker::DeliveryRecord {
                    package_uuid,
                    title,
                    version,
                    destination,
                    delivery_method: method,
                    timestamp: dcpwizard_core::version_tracker::now_iso(),
                    verified,
                };
                if tracker.record(&record) {
                    println!("Recorded delivery of {}", record.package_uuid);
                    0
                } else {
                    tracing::error!("Failed to record delivery");
                    1
                }
            }
            VersionAction::List {
                db,
                package_uuid,
                destination,
            } => {
                let mut tracker = dcpwizard_core::version_tracker::VersionTracker::new();
                if !tracker.open(Path::new(&db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                let query = dcpwizard_core::version_tracker::VersionQuery {
                    package_uuid,
                    destination,
                    ..Default::default()
                };
                let records = tracker.query(&query);
                if records.is_empty() {
                    println!("No deliveries recorded");
                } else {
                    for r in &records {
                        println!(
                            "{}  {}  {}  -> {}  ({}, verified={})",
                            r.timestamp,
                            r.package_uuid,
                            r.title,
                            r.destination,
                            r.delivery_method,
                            r.verified
                        );
                    }
                }
                0
            }
            VersionAction::Export { db, output } => {
                let mut tracker = dcpwizard_core::version_tracker::VersionTracker::new();
                if !tracker.open(Path::new(&db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                let out = PathBuf::from(&output);
                let ok = if output.to_lowercase().ends_with(".csv") {
                    tracker.export_csv(&out)
                } else {
                    tracker.export_json(&out)
                };
                if ok {
                    println!("Exported delivery history to {output}");
                    0
                } else {
                    tracing::error!("Failed to export delivery history");
                    1
                }
            }
        },

        Commands::Dashboard { action } => {
            // register/list/status/matrix operate on postkit's default database;
            // ensure its schema exists first.
            let db_path = dcpwizard_core::dashboard::default_db_path();
            if dcpwizard_core::dashboard::init_database(&db_path) != 0 {
                tracing::error!("Failed to initialise dashboard database");
                std::process::exit(1);
            }
            match action {
                DashboardAction::Register {
                    uuid,
                    title,
                    version_type,
                    territory,
                    language,
                    standard,
                    dcp_path,
                    status,
                    kdm_recipients,
                } => {
                    let entry = dcpwizard_core::dashboard::VersionEntry {
                        uuid,
                        title,
                        version_type,
                        territory,
                        language,
                        standard,
                        dcp_path: PathBuf::from(dcp_path),
                        ov_uuid: String::new(),
                        created_date: dcpwizard_core::version_tracker::now_iso(),
                        status,
                        kdm_recipients,
                    };
                    if dcpwizard_core::dashboard::register_version(&entry) == 0 {
                        println!("Registered version {}", entry.uuid);
                        0
                    } else {
                        1
                    }
                }
                DashboardAction::List { territory, status } => {
                    let versions = dcpwizard_core::dashboard::list_versions(
                        territory.as_deref(),
                        status.as_deref(),
                    );
                    if versions.is_empty() {
                        println!("No versions registered");
                    } else {
                        for v in &versions {
                            println!(
                                "{}  {}  {}  {}  [{}]",
                                v.uuid, v.title, v.version_type, v.territory, v.status
                            );
                        }
                    }
                    0
                }
                DashboardAction::Status { uuid, status } => {
                    if dcpwizard_core::dashboard::update_status(&uuid, &status) == 0 {
                        println!("Updated {uuid} -> {status}");
                        0
                    } else {
                        tracing::error!("Failed to update status (unknown UUID?)");
                        1
                    }
                }
                DashboardAction::Matrix { output } => {
                    if dcpwizard_core::dashboard::export_distribution_matrix(Path::new(&output))
                        == 0
                    {
                        println!("Exported distribution matrix to {output}");
                        0
                    } else {
                        tracing::error!("Failed to export distribution matrix");
                        1
                    }
                }
                DashboardAction::Serve { port, bind } => {
                    let opts = dcpwizard_core::dashboard::DashboardOptions {
                        database_path: db_path,
                        http_port: port,
                        bind_address: bind,
                    };
                    dcpwizard_core::dashboard::serve_dashboard(&opts)
                }
            }
        }

        Commands::IngestPackage { dir } => {
            let code = dcpwizard_core::ingest_package::ingest_package(&PathBuf::from(&dir));
            if code == 0 {
                println!("Repackaged {dir} (regenerated ASSETMAP and PKL)");
            }
            code
        }

        Commands::Combine {
            inputs,
            output,
            separate_pkls,
            sort,
            annotation,
        } => {
            let config = dcpwizard_core::combine::CombineConfig {
                inputs: inputs.iter().map(PathBuf::from).collect(),
                output_dir: PathBuf::from(&output),
                separate_pkls,
                sort,
                annotation,
            };
            let code = dcpwizard_core::combine::combine(&config);
            if code == 0 {
                println!("Combined into {output}");
            }
            code
        }

        Commands::CreateVf {
            ov,
            output,
            title,
            replace_picture,
            replace_sound,
            replace_subtitle,
            add_subtitle,
            replace_ccap,
            add_ccap,
            subtitle_language,
        } => {
            // Parse REEL=PATH into a per-reel map. picture/sound/subtitle/ccap share
            // reels; --add-* and --replace-* both set the track.
            #[derive(Clone, Copy)]
            enum Track {
                Picture,
                Sound,
                Subtitle,
                Ccap,
            }
            let mut reels: std::collections::BTreeMap<u32, dcpwizard_core::vf::ReplacementReel> =
                std::collections::BTreeMap::new();
            let mut parse_ok = true;
            for (specs, track) in [
                (&replace_picture, Track::Picture),
                (&replace_sound, Track::Sound),
                (&replace_subtitle, Track::Subtitle),
                (&add_subtitle, Track::Subtitle),
                (&replace_ccap, Track::Ccap),
                (&add_ccap, Track::Ccap),
            ] {
                for spec in specs {
                    let Some((reel_str, path)) = spec.split_once('=') else {
                        tracing::error!("bad --replace spec '{spec}', expected REEL=PATH");
                        parse_ok = false;
                        continue;
                    };
                    let Ok(reel_number) = reel_str.trim().parse::<u32>() else {
                        tracing::error!("bad reel number in '{spec}'");
                        parse_ok = false;
                        continue;
                    };
                    let entry =
                        reels
                            .entry(reel_number)
                            .or_insert(dcpwizard_core::vf::ReplacementReel {
                                reel_number,
                                ..Default::default()
                            });
                    let p = Some(PathBuf::from(path.trim()));
                    match track {
                        Track::Picture => entry.picture = p,
                        Track::Sound => entry.sound = p,
                        Track::Subtitle => entry.subtitle = p,
                        Track::Ccap => entry.ccap = p,
                    }
                }
            }

            if !parse_ok {
                1
            } else {
                let config = dcpwizard_core::vf::VfConfig {
                    ov_dir: PathBuf::from(&ov),
                    vf_dir: PathBuf::from(&output),
                    title,
                    subtitle_language,
                    replacement_reels: reels.into_values().collect(),
                };
                let code = dcpwizard_core::vf::create_vf(&config);
                if code == 0 {
                    println!("Created VF DCP at {output}");
                }
                code
            }
        }

        Commands::Assemble {
            input,
            output,
            title,
        } => {
            let config = dcpwizard_core::assemble::AssembleConfig {
                inputs: input.iter().map(PathBuf::from).collect(),
                output_dir: PathBuf::from(&output),
                title,
            };
            let code = dcpwizard_core::assemble::assemble(&config);
            if code == 0 {
                println!("Assembled OV at {output}");
            }
            code
        }

        Commands::Edit {
            input,
            output,
            title,
            annotation,
            content_kind,
            issuer,
        } => {
            let config = dcpwizard_core::edit::EditConfig {
                input: PathBuf::from(&input),
                output: output.as_ref().map(PathBuf::from),
                title,
                annotation,
                content_kind,
                issuer,
            };
            let code = dcpwizard_core::edit::edit_dcp(&config);
            if code == 0 {
                println!(
                    "Edited DCP CPL metadata ({})",
                    output.as_deref().unwrap_or(&input)
                );
            }
            code
        }

        Commands::CreateMulti {
            compositions,
            output,
            standard,
            frame_rate,
            fourk,
            container,
            subtitle_language,
            content_type,
            encrypt,
            key_out,
        } => {
            let comps =
                match dcpwizard_core::multi_cpl::load_compositions(&PathBuf::from(&compositions)) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("{e}");
                        return;
                    }
                };
            let std_val = if standard == "interop" {
                dcpwizard_core::Standard::Interop
            } else {
                dcpwizard_core::Standard::Smpte
            };
            let resolution = if fourk {
                dcpwizard_core::Resolution::FourK
            } else {
                dcpwizard_core::Resolution::TwoK
            };
            let (container_width, container_height) = match container.as_deref() {
                Some("2k-scope") => (2048, 858),
                Some("2k-flat") => (1998, 1080),
                Some("2k-full") => (2048, 1080),
                Some("4k-scope") => (4096, 1716),
                Some("4k-flat") => (3996, 2160),
                Some("4k-full") => (4096, 2160),
                Some(value) => {
                    tracing::error!("Unknown container: {value}");
                    return;
                }
                None => (0, 0),
            };
            let ct = content_type
                .as_deref()
                .and_then(dcpwizard_core::ContentType::from_abbrev)
                .unwrap_or_default();
            let config = dcpwizard_core::dcp::DcpConfig {
                title: String::new(),
                standard: std_val,
                resolution,
                content_type: ct,
                frame_rate_num: frame_rate,
                frame_rate_den: 1,
                encrypt,
                key_out: key_out.map(PathBuf::from),
                container_width,
                container_height,
                output_dir: PathBuf::from(&output),
                subtitle_language,
                ..Default::default()
            };
            let code = dcpwizard_core::multi_cpl::create_multi_composition(&config, &comps);
            if code == 0 {
                println!("Created multi-composition DCP at {output}");
            }
            code
        }
    };

    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_target_selects_dcdm_target() {
        assert_eq!(
            parse_dcdm_target("xyz"),
            Some(postkit::dcdm::DcdmTarget::Xyz)
        );
        assert_eq!(
            parse_dcdm_target("p3-d65"),
            Some(postkit::dcdm::DcdmTarget::P3D65)
        );
        assert_eq!(
            parse_dcdm_target("p3d65"),
            Some(postkit::dcdm::DcdmTarget::P3D65)
        );
        // ffmpeg colorspace targets are not dcdm-module targets
        assert_eq!(parse_dcdm_target("rec709"), None);
        assert_eq!(parse_dcdm_target("p3"), None);
    }
}
