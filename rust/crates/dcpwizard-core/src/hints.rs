//! Advisory findings gathered before the encode.
//!
//! A hint is not a refusal: everything here builds and packages. It says the
//! result is likely to be wrong on a cinema screen, so the front ends print it
//! and let the build go on.

use std::path::Path;

use crate::preflight::{CreatePlan, PictureKind};
use crate::{ContentType, Standard};

/// One advisory finding, ready to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub text: String,
}

/// Sound peaking above this clips on some playback chains.
const LOUD_TRUE_PEAK_DBTP: f64 = -3.0;
/// A sound track packaged with fewer channels than this can trouble a projector.
const FEWEST_PACKAGED_CHANNELS: u32 = 6;
/// The channel counts a distributor expects a DCP sound track to carry.
const EXPECTED_PACKAGED_CHANNELS: [u32; 2] = [8, 16];

/// The two container ratios every projector masks to.
const FLAT_RATIO: f64 = 1.85;
const SCOPE_RATIO: f64 = 2.39;
/// Content narrower than this inside a Scope container is pillar-boxed.
const NARROWEST_SCOPE_CONTENT_RATIO: f64 = 1.90;
/// The ratios content is matched against when deciding what shape it is.
const NAMED_RATIOS: [f64; 6] = [1.33, 1.66, 1.78, FLAT_RATIO, 1.90, SCOPE_RATIO];
/// How close an aspect has to sit to a named ratio to count as that ratio.
const RATIO_TOLERANCE: f64 = 0.005;

/// A few projectors stumble on a track at or above this.
const HIGH_VIDEO_BIT_RATE_MBPS: u32 = 245;
/// How far the sound's pitch may move before it is audible, as the ratio
/// between the DCP's frame rate and the source's.
const LARGEST_SOUND_SPEED_CHANGE: f64 = 25.5 / 24.0;

/// Rates not every projector plays, each with the rate to fall back to. None is
/// a rate with no safer neighbour to name.
const AWKWARD_FRAME_RATES: [(u32, Option<u32>); 5] = [
    (25, Some(24)),
    (30, None),
    (48, Some(24)),
    (50, Some(25)),
    (60, Some(30)),
];

/// A first subtitle earlier than this is easy to miss.
const FIRST_CUE_SECONDS: f64 = 4.0;
/// A cue shorter than this is hard to read.
const SHORTEST_CUE_FRAMES: f64 = 15.0;
/// Two cues closer than this read as one flicker.
const SMALLEST_CUE_GAP_FRAMES: f64 = 2.0;
const MOST_CUE_LINES: usize = 3;
/// Line lengths, in characters: the length to aim for, and the one past which
/// the text will not fit at all.
const ADVISED_LINE_CHARACTERS: usize = 52;
const MOST_LINE_CHARACTERS: usize = 79;
/// A caption line is held to the narrower limit a caption reader draws.
const MOST_CAPTION_LINE_CHARACTERS: usize = 32;

const MILLISECONDS_PER_SECOND: f64 = 1000.0;
const SECONDS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;

/// What the hints are decided from. Kept apart from the probing so the rules can
/// be driven directly.
#[derive(Debug, Clone, Default)]
pub struct HintFacts {
    pub standard: Standard,
    pub content_type: ContentType,
    pub fps: u32,
    /// The frame rate the source runs at, when the picture is a video.
    pub source_fps: Option<f64>,
    /// Whether the job conforms the source by playing it faster with the sound
    /// pulled up to match, which is what a 23.976 source at 24 fps gets.
    pub conforms_with_pull_up: bool,
    pub four_k: bool,
    pub stereo_3d: bool,
    pub video_bit_rate_mbps: u32,
    /// The active area a container declares, when one was named.
    pub container: Option<(u32, u32)>,
    /// The size the picture itself lands at inside the encode raster.
    pub content: Option<(u32, u32)>,
    /// Channels the sound track is packaged with, after any map, upmix and the
    /// wrap's own padding.
    pub packaged_channels: Option<u32>,
    pub upmix: bool,
    /// Whether the composition carries sound at all, measured or not.
    pub has_audio: bool,
    pub audio_language: Option<String>,
    /// The measured level of each sound file the loudness pass could read.
    pub audio: Vec<AudioLevel>,
    pub markers: Vec<MarkerPlacement>,
    pub picture_frames: u64,
    /// Subtitles the audience reads on screen, packaged or burnt in.
    pub subtitles: Vec<SubtitleCues>,
    pub captions: Vec<SubtitleCues>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioLevel {
    pub file: String,
    pub true_peak_dbtp: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerPlacement {
    pub label: String,
    pub frame: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCues {
    pub file: String,
    pub cues: Vec<HintCue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HintCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub lines: Vec<String>,
}

/// Every hint the job raises, probing the source for the numbers it needs.
pub fn gather_hints(plan: &CreatePlan) -> Vec<Hint> {
    hints_from(&probe_hint_facts(plan))
}

fn hints_from(facts: &HintFacts) -> Vec<Hint> {
    let mut hints = Vec::new();
    hints.extend(interop_hint(facts));
    hints.extend(sound_channel_hints(facts));
    hints.extend(upmix_hint(facts));
    hints.extend(container_hints(facts));
    hints.extend(video_bit_rate_hint(facts));
    hints.extend(frame_rate_hint(facts));
    hints.extend(four_k_stereo_hint(facts));
    hints.extend(source_rate_hint(facts));
    hints.extend(pull_up_hint(facts));
    hints.extend(audio_level_hint(facts));
    hints.extend(audio_language_hint(facts));
    hints.extend(marker_hints(facts));
    hints.extend(subtitle_hints(facts));
    hints.extend(caption_hints(facts));
    hints
}

fn interop_hint(facts: &HintFacts) -> Option<Hint> {
    (facts.standard == Standard::Interop).then(|| Hint {
        text: "This is an Interop DCP. Build a SMPTE one unless something in the chain \
               needs Interop."
            .to_string(),
    })
}

fn sound_channel_hints(facts: &HintFacts) -> Vec<Hint> {
    let Some(channels) = facts.packaged_channels else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    if channels < FEWEST_PACKAGED_CHANNELS {
        hints.push(Hint {
            text: format!(
                "The sound track is packaged with {channels} channels. Fewer than \
                 {FEWEST_PACKAGED_CHANNELS} can trouble a projector, and the channels the \
                 content does not fill cost only silence. Pass --audio-channels \
                 {FEWEST_PACKAGED_CHANNELS} (or {}) to fill the rest with silence.",
                EXPECTED_PACKAGED_CHANNELS[1]
            ),
        });
    }
    if !EXPECTED_PACKAGED_CHANNELS.contains(&channels) {
        hints.push(Hint {
            text: format!(
                "The sound track is packaged with {channels} channels, not \
                 {} or {}. Some distributors raise a QC error on any other count.",
                EXPECTED_PACKAGED_CHANNELS[0], EXPECTED_PACKAGED_CHANNELS[1]
            ),
        });
    }
    hints
}

fn upmix_hint(facts: &HintFacts) -> Option<Hint> {
    facts.upmix.then(|| Hint {
        text: "The stereo-to-5.1 upmix is in use. It is experimental, so listen to the \
               finished DCP in a cinema before it goes out."
            .to_string(),
    })
}

fn container_hints(facts: &HintFacts) -> Vec<Hint> {
    let Some(container) = facts.container else {
        return Vec::new();
    };
    let container_ratio = aspect_of(container);
    let mut hints = Vec::new();
    if let Some(content_ratio) = facts.content.map(aspect_of) {
        if is_ratio(container_ratio, FLAT_RATIO) && nearest_ratio(content_ratio) == SCOPE_RATIO {
            hints.push(Hint {
                text: format!(
                    "The picture is Scope ({SCOPE_RATIO}:1) inside a Flat ({FLAT_RATIO}:1) \
                     container, so it will be letter-boxed. A Scope container fits it."
                ),
            });
        }
        if is_ratio(container_ratio, SCOPE_RATIO) && content_ratio < NARROWEST_SCOPE_CONTENT_RATIO {
            hints.push(Hint {
                text: format!(
                    "The picture is narrower than {NARROWEST_SCOPE_CONTENT_RATIO:.2}:1 inside a \
                     Scope ({SCOPE_RATIO}:1) container, so it will be pillar-boxed. Give the \
                     container the ratio the picture has."
                ),
            });
        }
    }
    if !is_ratio(container_ratio, FLAT_RATIO) && !is_ratio(container_ratio, SCOPE_RATIO) {
        hints.push(Hint {
            text: format!(
                "The container is {container_ratio:.2}:1, which is neither Flat \
                 ({FLAT_RATIO}:1) nor Scope ({SCOPE_RATIO}:1). An unusual ratio can trouble \
                 a projector."
            ),
        });
    }
    hints
}

fn video_bit_rate_hint(facts: &HintFacts) -> Option<Hint> {
    (facts.video_bit_rate_mbps >= HIGH_VIDEO_BIT_RATE_MBPS).then(|| Hint {
        text: format!(
            "The video bit rate is {} Mbps. A few projectors stumble on a very high rate, \
             and around 200 Mbps looks the same.",
            facts.video_bit_rate_mbps
        ),
    })
}

fn frame_rate_hint(facts: &HintFacts) -> Option<Hint> {
    let (rate, fallback) = AWKWARD_FRAME_RATES
        .into_iter()
        .find(|(rate, _)| *rate == facts.fps)?;
    let advice = match fallback {
        Some(fallback) => format!("{fallback} fps is the rate to fall back to."),
        None => "Expect compatibility problems on some of them.".to_string(),
    };
    let interop = if rate == 25 && facts.standard == Standard::Interop {
        " Interop at 25 fps is worse still: use the SMPTE standard."
    } else {
        ""
    };
    Some(Hint {
        text: format!("The DCP is {rate} fps, which not every projector plays. {advice}{interop}"),
    })
}

fn four_k_stereo_hint(facts: &HintFacts) -> Option<Hint> {
    (facts.four_k && facts.stereo_3d).then(|| Hint {
        text: "4K 3D plays on very few projectors. Package it at 2K unless you know the \
               projector it goes to takes 4K 3D."
            .to_string(),
    })
}

fn source_rate_hint(facts: &HintFacts) -> Option<Hint> {
    let source_fps = facts.source_fps.filter(|fps| *fps > 0.0)?;
    let dcp_fps = f64::from(facts.fps);
    let change = (dcp_fps / source_fps).max(source_fps / dcp_fps);
    (change > LARGEST_SOUND_SPEED_CHANGE).then(|| Hint {
        text: format!(
            "The source runs at {source_fps:.3} fps and the DCP at {} fps, so the sound plays \
             back at a noticeably wrong pitch. Pick a DCP rate closer to the source.",
            facts.fps
        ),
    })
}

fn pull_up_hint(facts: &HintFacts) -> Option<Hint> {
    facts.conforms_with_pull_up.then(|| Hint {
        text: format!(
            "The 23.976 source will play at {} fps, 0.1% faster, and the sound is pulled up by \
             the same amount to stay in sync. No frame is duplicated or dropped.",
            facts.fps
        ),
    })
}

fn audio_level_hint(facts: &HintFacts) -> Option<Hint> {
    let loud = facts
        .audio
        .iter()
        .find(|level| level.true_peak_dbtp > LOUD_TRUE_PEAK_DBTP)?;
    Some(Hint {
        text: format!(
            "The audio level is very high ({:.1} dBTP in {}). Reduce the gain.",
            loud.true_peak_dbtp, loud.file
        ),
    })
}

fn audio_language_hint(facts: &HintFacts) -> Option<Hint> {
    let named = facts
        .audio_language
        .as_deref()
        .map(str::trim)
        .is_some_and(|language| !language.is_empty());
    (facts.has_audio && !named).then(|| Hint {
        text: "The sound has no language set. Set one unless it has no spoken parts.".to_string(),
    })
}

fn marker_hints(facts: &HintFacts) -> Vec<Hint> {
    let mut hints = Vec::new();
    let placed = |label: &str| facts.markers.iter().any(|marker| marker.label == label);
    if facts.standard == Standard::Smpte
        && facts.content_type == ContentType::Feature
        && !(placed("FFEC") && placed("FFMC"))
    {
        hints.push(Hint {
            text: "This is a SMPTE feature carrying no FFEC and FFMC markers. Distributors \
                   expect the first frame of end credits and the first frame of moving \
                   credits."
                .to_string(),
        });
    }
    if facts.picture_frames > 0
        && let Some(late) = facts
            .markers
            .iter()
            .find(|marker| marker.frame >= facts.picture_frames)
    {
        hints.push(Hint {
            text: format!(
                "The marker {} sits at frame {}, at or past the picture's {} frames, so it \
                 will be ignored.",
                late.label, late.frame, facts.picture_frames
            ),
        });
    }
    hints
}

/// A cue with what the rules need around it.
struct CueInContext<'a> {
    cue: &'a HintCue,
    previous_end_ms: Option<u64>,
    is_first: bool,
    fps: u32,
    interop: bool,
}

/// How a rule words itself, given the file it found the fault in and the time of
/// the first cue that showed it.
type SayHint = fn(&str, &str) -> String;

/// One advisory rule over a cue: what counts as an offence, and what to say
/// about the first cue that offends.
struct CueRule {
    offends: fn(&CueInContext) -> bool,
    say: SayHint,
}

const SUBTITLE_RULES: [CueRule; 4] = [
    CueRule {
        offends: |context| {
            context.is_first && context.cue.start_ms < seconds_to_milliseconds(FIRST_CUE_SECONDS)
        },
        say: |file, at| {
            format!(
                "The first subtitle in {file} starts at {at}. Put it at least {FIRST_CUE_SECONDS:.0} seconds in, or it is easy to miss."
            )
        },
    },
    CueRule {
        offends: |context| {
            context.cue.end_ms.saturating_sub(context.cue.start_ms)
                < frames_to_milliseconds(SHORTEST_CUE_FRAMES, context.fps)
        },
        say: |file, at| {
            format!(
                "A subtitle in {file} at {at} lasts less than {SHORTEST_CUE_FRAMES:.0} frames. Make every subtitle at least that long."
            )
        },
    },
    CueRule {
        offends: |context| match context.previous_end_ms {
            Some(previous_end_ms) => {
                context.cue.start_ms
                    < previous_end_ms + frames_to_milliseconds(SMALLEST_CUE_GAP_FRAMES, context.fps)
            }
            None => false,
        },
        say: |file, at| {
            format!(
                "A subtitle in {file} at {at} starts less than {SMALLEST_CUE_GAP_FRAMES:.0} frames after the one before it ends. Leave at least that gap."
            )
        },
    },
    CueRule {
        offends: |context| context.cue.lines.len() > MOST_CUE_LINES,
        say: |file, at| {
            format!(
                "A subtitle in {file} at {at} has more than {MOST_CUE_LINES} lines. Use no more than {MOST_CUE_LINES}."
            )
        },
    },
];

const CAPTION_RULES: [CueRule; 3] = [
    CueRule {
        offends: |context| {
            context
                .cue
                .lines
                .iter()
                .any(|line| line.chars().count() > MOST_CAPTION_LINE_CHARACTERS)
        },
        say: |file, at| {
            format!(
                "A caption line in {file} at {at} is longer than {MOST_CAPTION_LINE_CHARACTERS} characters. Keep caption lines to {MOST_CAPTION_LINE_CHARACTERS} at most."
            )
        },
    },
    CueRule {
        offends: |context| context.cue.lines.len() > MOST_CUE_LINES,
        say: |file, at| {
            format!(
                "A caption in {file} at {at} has more than {MOST_CUE_LINES} lines, so it will be truncated."
            )
        },
    },
    CueRule {
        offends: |context| match context.previous_end_ms {
            Some(previous_end_ms) => context.interop && context.cue.start_ms < previous_end_ms,
            None => false,
        },
        say: |file, at| {
            format!(
                "Captions in {file} overlap at {at}, which an Interop DCP does not allow. Use the SMPTE standard."
            )
        },
    },
];

fn subtitle_hints(facts: &HintFacts) -> Vec<Hint> {
    let mut hints: Vec<Hint> = SUBTITLE_RULES
        .iter()
        .filter_map(|rule| first_offence(facts, &facts.subtitles, rule))
        .collect();
    hints.extend(line_length_hint(facts));
    hints
}

fn caption_hints(facts: &HintFacts) -> Vec<Hint> {
    CAPTION_RULES
        .iter()
        .filter_map(|rule| first_offence(facts, &facts.captions, rule))
        .collect()
}

/// The first cue in reading order that breaks a rule, said once for the whole
/// job rather than once per cue.
fn first_offence(facts: &HintFacts, files: &[SubtitleCues], rule: &CueRule) -> Option<Hint> {
    for subtitle in files {
        let mut previous_end_ms = None;
        for (index, cue) in subtitle.cues.iter().enumerate() {
            let context = CueInContext {
                cue,
                previous_end_ms,
                is_first: index == 0,
                fps: facts.fps,
                interop: facts.standard == Standard::Interop,
            };
            if (rule.offends)(&context) {
                return Some(Hint {
                    text: (rule.say)(&subtitle.file, &format_cue_time(cue.start_ms)),
                });
            }
            previous_end_ms = Some(cue.end_ms);
        }
    }
    None
}

/// A line past the hard limit is the same fault as one past the advised length,
/// said more strongly, so only the stronger hint is raised.
fn line_length_hint(facts: &HintFacts) -> Option<Hint> {
    let limits: [(usize, SayHint); 2] = [
        (MOST_LINE_CHARACTERS, |file, at| {
            format!(
                "A subtitle line in {file} at {at} is longer than {MOST_LINE_CHARACTERS} characters. Cut it to {MOST_LINE_CHARACTERS} at most."
            )
        }),
        (ADVISED_LINE_CHARACTERS, |file, at| {
            format!(
                "A subtitle line in {file} at {at} is longer than {ADVISED_LINE_CHARACTERS} characters. Keep lines to {ADVISED_LINE_CHARACTERS} where you can."
            )
        }),
    ];
    for (characters, say) in limits {
        for subtitle in &facts.subtitles {
            let offender = subtitle.cues.iter().find(|cue| {
                cue.lines
                    .iter()
                    .any(|line| line.chars().count() > characters)
            });
            if let Some(cue) = offender {
                return Some(Hint {
                    text: say(&subtitle.file, &format_cue_time(cue.start_ms)),
                });
            }
        }
    }
    None
}

/// The ratio nearest `aspect` among the shapes content is described by.
fn nearest_ratio(aspect: f64) -> f64 {
    NAMED_RATIOS
        .into_iter()
        .min_by(|left, right| (left - aspect).abs().total_cmp(&(right - aspect).abs()))
        .unwrap_or(aspect)
}

fn is_ratio(aspect: f64, ratio: f64) -> bool {
    (aspect - ratio).abs() <= RATIO_TOLERANCE
}

fn aspect_of((width, height): (u32, u32)) -> f64 {
    f64::from(width) / f64::from(height.max(1))
}

const fn seconds_to_milliseconds(seconds: f64) -> u64 {
    (seconds * MILLISECONDS_PER_SECOND) as u64
}

fn frames_to_milliseconds(frames: f64, fps: u32) -> u64 {
    (frames / f64::from(fps.max(1)) * MILLISECONDS_PER_SECOND).round() as u64
}

fn format_cue_time(milliseconds: u64) -> String {
    let total_seconds = milliseconds / MILLISECONDS_PER_SECOND as u64;
    let hours = total_seconds / (SECONDS_PER_MINUTE * MINUTES_PER_HOUR);
    let minutes = total_seconds / SECONDS_PER_MINUTE % MINUTES_PER_HOUR;
    let seconds = total_seconds % SECONDS_PER_MINUTE;
    format!(
        "{hours:02}:{minutes:02}:{seconds:02}.{:03}",
        milliseconds % MILLISECONDS_PER_SECOND as u64
    )
}

fn probe_hint_facts(plan: &CreatePlan) -> HintFacts {
    let planned = crate::preflight::plan_picture(plan).ok().flatten();
    let source = plan.source.as_ref();
    let audio = plan
        .packaged_wav()
        .into_iter()
        .filter_map(|path| {
            // a WAV the loudness pass cannot read is the encode's problem to report
            let measured = crate::loudness::measure_loudness(path);
            measured.success.then(|| AudioLevel {
                file: short_name(path),
                true_peak_dbtp: measured.true_peak_dbtp,
            })
        })
        .collect();

    HintFacts {
        standard: plan.standard,
        content_type: plan.content_type,
        fps: plan.fps,
        source_fps: (plan.picture_kind == PictureKind::Video)
            .then_some(source)
            .flatten()
            .map(|info| f64::from(info.fps_num) / f64::from(info.fps_den.max(1))),
        conforms_with_pull_up: (plan.picture_kind == PictureKind::Video)
            .then_some(source)
            .flatten()
            .is_some_and(|info| {
                crate::hfr::conform_source_to_dcp(info.fps_num, info.fps_den, plan.fps)
                    .audio_pull_up
            }),
        four_k: plan.four_k,
        stereo_3d: plan.right_eye.is_some(),
        video_bit_rate_mbps: plan.video_bit_rate_mbps,
        container: plan.geometry.container,
        content: planned.map(|picture| picture.content),
        packaged_channels: packaged_channels(plan),
        upmix: plan.upmix,
        has_audio: plan.audio.is_some() || source.is_some_and(|info| info.has_audio),
        audio_language: plan.audio_language.clone(),
        audio,
        markers: placed_markers(plan),
        picture_frames: crate::preflight::planned_picture_frames(plan).unwrap_or(0),
        subtitles: read_cue_files(
            [plan.subtitle.as_deref(), plan.burn_subtitle.as_deref()],
            plan.fps,
        ),
        captions: read_cue_files([plan.ccap.as_deref(), None], plan.fps),
    }
}

/// How many channels the sound track lands with: what the map or the upmix
/// leaves, filled to the count the job asked for, or widened by the wrap's own
/// 5.1 padding when it asked for none.
fn packaged_channels(plan: &CreatePlan) -> Option<u32> {
    let channels = crate::preflight::content_channels(plan)?;
    Some(match plan.audio_channels {
        Some(count) => count,
        // the wrap widens a 5.1 source to the 16-channel DCP layout on its own
        None if channels == crate::mxf_wrap::CANONICAL_51_CHANNELS => {
            crate::mxf_wrap::DEFAULT_PACKAGED_51_CHANNELS
        }
        None => channels,
    })
}

/// The markers the job places, dropping any spec that does not parse: the
/// packager reports those itself.
fn placed_markers(plan: &CreatePlan) -> Vec<MarkerPlacement> {
    plan.markers
        .iter()
        .filter_map(|spec| {
            let (label, offset) = spec.split_once('=')?;
            let marker = crate::markers::Marker::from_label(label)?;
            let frame = crate::markers::parse_frame_offset(offset, plan.fps).ok()?;
            Some(MarkerPlacement {
                label: marker.label().to_string(),
                frame,
            })
        })
        .collect()
}

/// Read every cue file the audience sees. A file that does not parse is the
/// preflight's refusal to report, so it is skipped here.
fn read_cue_files(paths: [Option<&Path>; 2], fps: u32) -> Vec<SubtitleCues> {
    paths
        .into_iter()
        .flatten()
        .filter_map(|path| {
            let cues = crate::subtitle::load_styled_cues(path, fps).ok()?;
            Some(SubtitleCues {
                file: short_name(path),
                cues: cues
                    .into_iter()
                    .map(|cue| HintCue {
                        start_ms: cue.start_ms,
                        end_ms: cue.end_ms,
                        lines: cue
                            .plain_text()
                            .lines()
                            .map(|line| line.trim().to_string())
                            .filter(|line| !line.is_empty())
                            .collect(),
                    })
                    .collect(),
            })
        })
        .collect()
}

fn short_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: u32 = 24;

    fn cue(start_ms: u64, end_ms: u64, lines: &[&str]) -> HintCue {
        HintCue {
            start_ms,
            end_ms,
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }

    /// A job with nothing to say about it, so a test names only what it sets.
    /// The content type is a short: a feature carries its own marker hint.
    fn facts() -> HintFacts {
        HintFacts {
            fps: FPS,
            content_type: ContentType::Short,
            ..Default::default()
        }
    }

    fn with_cues(cues: Vec<HintCue>) -> HintFacts {
        HintFacts {
            subtitles: vec![SubtitleCues {
                file: "subs.srt".to_string(),
                cues,
            }],
            ..facts()
        }
    }

    fn texts(facts: &HintFacts) -> Vec<String> {
        hints_from(facts)
            .into_iter()
            .map(|hint| hint.text)
            .collect()
    }

    fn mentions(facts: &HintFacts, needle: &str) -> bool {
        texts(facts).iter().any(|text| text.contains(needle))
    }

    #[test]
    fn an_interop_package_is_hinted_and_a_smpte_one_is_not() {
        let interop = HintFacts {
            standard: Standard::Interop,
            ..facts()
        };
        assert!(mentions(&interop, "Interop DCP"), "{:?}", texts(&interop));
        assert!(!mentions(&facts(), "Interop DCP"));
    }

    /// A 6-channel package raises the QC-count hint and nothing else, and both
    /// channel hints fire together on a stereo one.
    #[test]
    fn both_channel_hints_can_fire_and_a_16_channel_track_raises_neither() {
        let stereo = HintFacts {
            packaged_channels: Some(2),
            ..facts()
        };
        assert!(mentions(&stereo, "Fewer than 6"), "{:?}", texts(&stereo));
        // the hint names the flag that fixes it
        assert!(
            mentions(&stereo, "--audio-channels 6 (or 16)"),
            "{:?}",
            texts(&stereo)
        );
        assert!(mentions(&stereo, "not 8 or 16"), "{:?}", texts(&stereo));

        let six = HintFacts {
            packaged_channels: Some(6),
            ..facts()
        };
        assert!(!mentions(&six, "Fewer than 6"), "{:?}", texts(&six));
        assert!(mentions(&six, "not 8 or 16"), "{:?}", texts(&six));

        let sixteen = HintFacts {
            packaged_channels: Some(16),
            ..facts()
        };
        assert_eq!(hints_from(&sixteen), vec![]);
    }

    #[test]
    fn the_upmix_is_hinted_only_when_it_is_used() {
        let upmixed = HintFacts {
            upmix: true,
            ..facts()
        };
        assert!(mentions(&upmixed, "upmix is in use"));
        assert!(!mentions(&facts(), "upmix is in use"));
    }

    #[test]
    fn scope_content_in_a_flat_container_is_hinted_and_flat_content_is_not() {
        let letter_boxed = HintFacts {
            container: Some((1998, 1080)),
            content: Some((1998, 836)),
            ..facts()
        };
        assert!(
            mentions(&letter_boxed, "letter-boxed"),
            "{:?}",
            texts(&letter_boxed)
        );

        let fitting = HintFacts {
            content: Some((1998, 1080)),
            ..letter_boxed.clone()
        };
        assert!(!mentions(&fitting, "letter-boxed"), "{:?}", texts(&fitting));
    }

    #[test]
    fn narrow_content_in_a_scope_container_is_hinted_and_190_content_is_not() {
        let pillar_boxed = HintFacts {
            container: Some((2048, 858)),
            content: Some((1526, 858)),
            ..facts()
        };
        assert!(
            mentions(&pillar_boxed, "pillar-boxed"),
            "{:?}",
            texts(&pillar_boxed)
        );

        let wide = HintFacts {
            content: Some((1631, 858)),
            ..pillar_boxed.clone()
        };
        assert!(!mentions(&wide, "pillar-boxed"), "{:?}", texts(&wide));
    }

    #[test]
    fn an_unusual_container_is_hinted_and_flat_and_scope_are_not() {
        let full = HintFacts {
            container: Some((2048, 1080)),
            ..facts()
        };
        assert!(mentions(&full, "neither Flat"), "{:?}", texts(&full));

        for container in [(1998, 1080), (2048, 858), (3996, 2160), (4096, 1716)] {
            let named = HintFacts {
                container: Some(container),
                ..facts()
            };
            assert!(!mentions(&named, "neither Flat"), "{container:?}");
        }
    }

    #[test]
    fn a_very_high_bit_rate_is_hinted_and_one_below_the_line_is_not() {
        let high = HintFacts {
            video_bit_rate_mbps: 245,
            ..facts()
        };
        assert!(mentions(&high, "245 Mbps"), "{:?}", texts(&high));

        let allowed = HintFacts {
            video_bit_rate_mbps: 244,
            ..facts()
        };
        assert!(!mentions(&allowed, "Mbps"), "{:?}", texts(&allowed));
    }

    #[test]
    fn every_awkward_frame_rate_is_hinted_and_24_is_not() {
        for (rate, fallback) in AWKWARD_FRAME_RATES {
            let job = HintFacts {
                fps: rate,
                ..facts()
            };
            assert!(
                mentions(&job, &format!("The DCP is {rate} fps")),
                "{:?}",
                texts(&job)
            );
            if let Some(fallback) = fallback {
                assert!(mentions(&job, &format!("{fallback} fps is the rate")));
            }
        }
        let common = HintFacts { fps: 24, ..facts() };
        assert!(!mentions(&common, "not every projector plays"));
    }

    #[test]
    fn interop_at_25_fps_adds_the_standard_advice() {
        let interop = HintFacts {
            fps: 25,
            standard: Standard::Interop,
            ..facts()
        };
        assert!(mentions(&interop, "Interop at 25 fps is worse"));

        let smpte = HintFacts { fps: 25, ..facts() };
        assert!(!mentions(&smpte, "Interop at 25 fps is worse"));
    }

    #[test]
    fn four_k_3d_is_hinted_and_four_k_on_its_own_is_not() {
        let stereo = HintFacts {
            four_k: true,
            stereo_3d: true,
            ..facts()
        };
        assert!(mentions(&stereo, "4K 3D"), "{:?}", texts(&stereo));

        let flat = HintFacts {
            stereo_3d: false,
            ..stereo.clone()
        };
        assert!(!mentions(&flat, "4K 3D"));
    }

    /// 25.5/24 is the line: a 25 fps source into a 24 fps DCP stays under it.
    #[test]
    fn a_large_speed_change_is_hinted_and_a_small_one_is_not() {
        let fast = HintFacts {
            fps: 24,
            source_fps: Some(30.0),
            ..facts()
        };
        assert!(mentions(&fast, "wrong pitch"), "{:?}", texts(&fast));

        let near = HintFacts {
            source_fps: Some(25.0),
            ..fast.clone()
        };
        assert!(!mentions(&near, "wrong pitch"), "{:?}", texts(&near));
    }

    #[test]
    fn the_23_976_conform_is_spelled_out_and_a_matched_rate_says_nothing() {
        let pulled_up = HintFacts {
            fps: 24,
            source_fps: Some(24000.0 / 1001.0),
            conforms_with_pull_up: true,
            ..facts()
        };
        assert!(
            mentions(&pulled_up, "0.1% faster"),
            "{:?}",
            texts(&pulled_up)
        );
        assert!(mentions(&pulled_up, "pulled up"));

        let matched = HintFacts {
            conforms_with_pull_up: false,
            ..pulled_up.clone()
        };
        assert!(!mentions(&matched, "0.1% faster"));
    }

    #[test]
    fn a_loud_track_is_named_with_its_peak_and_a_quiet_one_is_not() {
        let loud = HintFacts {
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: -0.4,
            }],
            has_audio: true,
            audio_language: Some("en".to_string()),
            ..facts()
        };
        assert!(
            mentions(&loud, "-0.4 dBTP in sound.wav"),
            "{:?}",
            texts(&loud)
        );

        let quiet = HintFacts {
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: LOUD_TRUE_PEAK_DBTP,
            }],
            ..loud.clone()
        };
        assert!(!mentions(&quiet, "audio level"), "{:?}", texts(&quiet));
    }

    #[test]
    fn sound_without_a_language_is_hinted_and_sound_with_one_is_not() {
        let unset = HintFacts {
            has_audio: true,
            ..facts()
        };
        assert!(mentions(&unset, "no language set"));

        let blank = HintFacts {
            audio_language: Some("  ".to_string()),
            ..unset.clone()
        };
        assert!(mentions(&blank, "no language set"));

        let set = HintFacts {
            audio_language: Some("de-DE".to_string()),
            ..unset.clone()
        };
        assert!(!mentions(&set, "no language set"));

        assert!(!mentions(&facts(), "no language set"));
    }

    #[test]
    fn a_smpte_feature_without_the_credit_markers_is_hinted() {
        let unmarked = HintFacts {
            content_type: ContentType::Feature,
            ..facts()
        };
        assert!(mentions(&unmarked, "FFEC and FFMC"));

        let half = HintFacts {
            markers: vec![MarkerPlacement {
                label: "FFEC".to_string(),
                frame: 10,
            }],
            picture_frames: 100,
            ..unmarked.clone()
        };
        assert!(mentions(&half, "FFEC and FFMC"));

        let marked = HintFacts {
            markers: vec![
                MarkerPlacement {
                    label: "FFEC".to_string(),
                    frame: 10,
                },
                MarkerPlacement {
                    label: "FFMC".to_string(),
                    frame: 20,
                },
            ],
            ..half.clone()
        };
        assert!(!mentions(&marked, "FFEC and FFMC"), "{:?}", texts(&marked));

        let trailer = HintFacts {
            content_type: ContentType::Trailer,
            ..unmarked.clone()
        };
        assert!(!mentions(&trailer, "FFEC and FFMC"));
    }

    #[test]
    fn a_marker_at_the_picture_length_is_hinted_and_one_inside_it_is_not() {
        let past = HintFacts {
            markers: vec![MarkerPlacement {
                label: "FFEC".to_string(),
                frame: 100,
            }],
            picture_frames: 100,
            ..facts()
        };
        assert!(mentions(&past, "at or past"), "{:?}", texts(&past));

        let inside = HintFacts {
            markers: vec![MarkerPlacement {
                label: "FFEC".to_string(),
                frame: 99,
            }],
            ..past.clone()
        };
        assert!(!mentions(&inside, "at or past"));
    }

    #[test]
    fn a_first_cue_before_four_seconds_is_hinted_and_one_at_four_is_not() {
        let early = with_cues(vec![cue(3_999, 10_000, &["hello"])]);
        assert!(
            mentions(&early, "starts at 00:00:03.999"),
            "{:?}",
            texts(&early)
        );

        let late = with_cues(vec![cue(4_000, 10_000, &["hello"])]);
        assert!(!mentions(&late, "at least 4 seconds"), "{:?}", texts(&late));
    }

    /// 15 frames at 24 fps is 625 ms.
    #[test]
    fn a_cue_shorter_than_fifteen_frames_is_hinted_and_one_exactly_that_long_is_not() {
        let short = with_cues(vec![cue(10_000, 10_624, &["hello"])]);
        assert!(
            mentions(&short, "less than 15 frames"),
            "{:?}",
            texts(&short)
        );

        let long_enough = with_cues(vec![cue(10_000, 10_625, &["hello"])]);
        assert!(!mentions(&long_enough, "less than 15 frames"));
    }

    /// 2 frames at 24 fps is 83 ms.
    #[test]
    fn cues_closer_than_two_frames_are_hinted_and_an_overlap_counts() {
        let tight = with_cues(vec![
            cue(10_000, 12_000, &["first"]),
            cue(12_082, 14_000, &["second"]),
        ]);
        assert!(
            mentions(&tight, "less than 2 frames after"),
            "{:?}",
            texts(&tight)
        );

        let overlapping = with_cues(vec![
            cue(10_000, 12_000, &["first"]),
            cue(11_000, 14_000, &["second"]),
        ]);
        assert!(mentions(&overlapping, "less than 2 frames after"));

        let spaced = with_cues(vec![
            cue(10_000, 12_000, &["first"]),
            cue(12_083, 14_000, &["second"]),
        ]);
        assert!(
            !mentions(&spaced, "less than 2 frames after"),
            "{:?}",
            texts(&spaced)
        );
    }

    #[test]
    fn more_than_three_subtitle_lines_is_hinted_and_three_is_not() {
        let four = with_cues(vec![cue(10_000, 12_000, &["a", "b", "c", "d"])]);
        assert!(mentions(&four, "more than 3 lines"), "{:?}", texts(&four));

        let three = with_cues(vec![cue(10_000, 12_000, &["a", "b", "c"])]);
        assert!(!mentions(&three, "more than 3 lines"));
    }

    #[test]
    fn a_long_line_is_hinted_and_the_hard_limit_replaces_the_advised_one() {
        let advised = with_cues(vec![cue(10_000, 12_000, &["x".repeat(53).as_str()])]);
        assert!(
            mentions(&advised, "longer than 52 characters"),
            "{:?}",
            texts(&advised)
        );
        assert!(!mentions(&advised, "longer than 79 characters"));

        let at_the_limit = with_cues(vec![cue(10_000, 12_000, &["x".repeat(52).as_str()])]);
        assert!(!mentions(&at_the_limit, "characters"));

        let hard = with_cues(vec![cue(10_000, 12_000, &["x".repeat(80).as_str()])]);
        assert!(
            mentions(&hard, "longer than 79 characters"),
            "{:?}",
            texts(&hard)
        );
        assert!(
            !mentions(&hard, "longer than 52 characters"),
            "the 79 hint replaces the 52 one: {:?}",
            texts(&hard)
        );
    }

    /// Characters, not bytes: a line of accented letters is as long as it looks.
    #[test]
    fn line_length_counts_characters_not_bytes() {
        let accented = with_cues(vec![cue(10_000, 12_000, &["é".repeat(52).as_str()])]);
        assert!(!mentions(&accented, "characters"), "{:?}", texts(&accented));
    }

    /// Each rule speaks once for the whole job, however many cues break it.
    #[test]
    fn a_rule_is_said_once_however_many_cues_break_it() {
        let job = with_cues(vec![
            cue(10_000, 10_100, &["first"]),
            cue(20_000, 20_100, &["second"]),
            cue(30_000, 30_100, &["third"]),
        ]);
        let said = texts(&job)
            .iter()
            .filter(|text| text.contains("less than 15 frames"))
            .count();
        assert_eq!(said, 1);
    }

    fn with_captions(cues: Vec<HintCue>) -> HintFacts {
        HintFacts {
            captions: vec![SubtitleCues {
                file: "captions.srt".to_string(),
                cues,
            }],
            ..facts()
        }
    }

    #[test]
    fn a_caption_line_over_32_characters_is_hinted_and_one_at_32_is_not() {
        let long = with_captions(vec![cue(10_000, 12_000, &["x".repeat(33).as_str()])]);
        assert!(
            mentions(&long, "longer than 32 characters"),
            "{:?}",
            texts(&long)
        );

        let allowed = with_captions(vec![cue(10_000, 12_000, &["x".repeat(32).as_str()])]);
        assert!(!mentions(&allowed, "longer than 32 characters"));
    }

    #[test]
    fn more_than_three_caption_lines_is_hinted_and_three_is_not() {
        let four = with_captions(vec![cue(10_000, 12_000, &["a", "b", "c", "d"])]);
        assert!(mentions(&four, "will be truncated"), "{:?}", texts(&four));

        let three = with_captions(vec![cue(10_000, 12_000, &["a", "b", "c"])]);
        assert!(!mentions(&three, "will be truncated"));
    }

    #[test]
    fn overlapping_captions_are_hinted_on_interop_only() {
        let overlapping = vec![
            cue(10_000, 12_000, &["first"]),
            cue(11_000, 14_000, &["second"]),
        ];
        let interop = HintFacts {
            standard: Standard::Interop,
            ..with_captions(overlapping.clone())
        };
        assert!(mentions(&interop, "overlap at"), "{:?}", texts(&interop));

        let smpte = with_captions(overlapping);
        assert!(!mentions(&smpte, "overlap at"));

        let separated = HintFacts {
            standard: Standard::Interop,
            ..with_captions(vec![
                cue(10_000, 12_000, &["first"]),
                cue(12_000, 14_000, &["second"]),
            ])
        };
        assert!(!mentions(&separated, "overlap at"));
    }

    #[test]
    fn a_clean_job_raises_nothing() {
        let job = HintFacts {
            fps: FPS,
            content_type: ContentType::Short,
            video_bit_rate_mbps: 150,
            container: Some((1998, 1080)),
            content: Some((1998, 1080)),
            packaged_channels: Some(16),
            has_audio: true,
            audio_language: Some("en".to_string()),
            audio: vec![AudioLevel {
                file: "sound.wav".to_string(),
                true_peak_dbtp: -6.0,
            }],
            picture_frames: 1000,
            subtitles: vec![SubtitleCues {
                file: "subs.srt".to_string(),
                cues: vec![
                    cue(5_000, 7_000, &["a line", "another"]),
                    cue(8_000, 10_000, &["one more"]),
                ],
            }],
            ..Default::default()
        };
        assert_eq!(hints_from(&job), vec![]);
    }
}
