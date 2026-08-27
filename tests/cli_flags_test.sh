#!/usr/bin/env bash
# Verify the GUI's CLI invocations parse against the real binary.
# Run from the project root: bash tests/cli_flags_test.sh [path-to-binary]
#
# Two checks:
#  1. Every flag the GUI passes exists in that subcommand's --help.
#  2. Each GUI command line is actually invoked (with dummy values) and must
#     not fail clap parsing. This catches missing required flags and unknown
#     flags, which a --help grep alone misses (e.g. the KDM panel that used to
#     omit --signer-cert/--signer-key and failed every run).

set -uo pipefail

BINARY="${1:-./build/dcpwizard}"
JS_FILE="gui/src/main.js"
FAILURES=0
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [[ ! -x "$BINARY" ]]; then
  echo "ERROR: Binary not found at $BINARY"
  echo "Usage: $0 [path-to-dcpwizard-binary]"
  exit 1
fi
if [[ ! -f "$JS_FILE" ]]; then
  echo "ERROR: $JS_FILE not found. Run from project root."
  exit 1
fi

# clap prints these to stderr and exits 2 when parsing fails. Runtime errors
# (missing files, etc.) are fine: they mean parsing succeeded.
PARSE_ERR='unexpected argument|required arguments were not provided|a value is required for|unrecognized subcommand|invalid value|cannot be used with'

echo "=== DCPWizard CLI Flag Verification ==="
echo "Binary: $BINARY"
echo ""

# ── Check 1: every flag the GUI sends exists in --help ──────────────────────
JS_SUBCMDS=$(grep -oP 'args\s*=\s*\["([a-z][-a-z]*)"|Command\.(?:sidecar|create)\("dcpwizard",\s*\["([a-z][-a-z]*)' "$JS_FILE" \
  | grep -oP '\["[a-z][-a-z]*' | tr -d '["' | sort -u)

for subcmd in $JS_SUBCMDS; do
  if ! "$BINARY" "$subcmd" --help &>/dev/null; then
    echo "FAIL: subcommand '$subcmd' does not exist in binary"
    FAILURES=$((FAILURES + 1))
    continue
  fi
  help_text=$("$BINARY" "$subcmd" --help 2>&1 || true)
  FLAGS=$(grep -P "\[\"$subcmd\"" "$JS_FILE" \
    | grep -oP '"--[a-z][-a-z0-9]*"|"-[a-z]"' | tr -d '"' | sort -u || true)
  for flag in $FLAGS; do
    if ! echo "$help_text" | grep -qF -- "$flag"; then
      echo "FAIL: '$subcmd $flag' not found in '$subcmd --help'"
      FAILURES=$((FAILURES + 1))
    fi
  done
done

# ── Check 2: invoke each GUI command line, reject clap parse errors ──────────
# Dummy paths that don't need to exist; we only care that parsing succeeds.
C="$TMP/f"                # generic file/dir placeholder
UUID="00000000-0000-0000-0000-000000000000"

smoke() {
  local label="$1"; shift
  local out
  out=$("$BINARY" "$@" 2>&1 || true)
  if echo "$out" | grep -qiE "$PARSE_ERR"; then
    echo "FAIL: '$label' rejected at parse time:"
    echo "$out" | grep -iE "$PARSE_ERR" | sed 's/^/    /'
    FAILURES=$((FAILURES + 1))
  else
    echo "ok: $label"
  fi
}

# Mirror the exact flag sets gui/src/main.js builds.
smoke "verify"          verify "$C" --strict --no-picture-check --no-hash-check
smoke "kdm"             kdm --cpl-id "$UUID" --content-title x --cert "$C" \
                            --signer-cert "$C" --signer-key "$C" --keys "$C" \
                            -o "$C" -f now -t "2 weeks"
smoke "batch list"      batch list
smoke "batch cancel"    batch cancel 1
smoke "encode"          encode -i "$C" -o "$C" --bandwidth 250
smoke "transcode"       transcode -i "$C" -o "$C"
smoke "loudness"        loudness "$C"
smoke "copy"            copy --src "$C" --dst "$C"
smoke "report"          report --dcp "$C" -o "$C/report.html"
smoke "report scan"     report --dcp "$C" -o "$C/report.html" --scan-picture
smoke "subtitle-convert" subtitle-convert -i "$C" -l en --fps 24 -o "$C"
smoke "burnin"          burnin -i "$C" -s "$C" -o "$C"
smoke "burnin styled"   burnin -i "$C" -s "$C" -o "$C" --colour FFFF00 \
                            --position top --video-codec libx264 --crf 0
smoke "convert"         convert -i "$C" -t mov -m fast -o "$C"
smoke "watermark"       watermark -i "$C" -o "$C" -p DIST-001 --font-size 40 \
                            --colour yellow --position top --video-codec libx264 --crf 0
smoke "create+encrypt"  create --title x --video "$C" --output "$C" --encrypt --key-out "$C"
smoke "assemble"        assemble --input "$C" --input "$C" --output "$C" --title x
smoke "edit"            edit --input "$C" --output "$C" --title x --annotation a --content-kind FTR --issuer i
smoke "create-vf subs"  create-vf --ov "$C" --output "$C" --add-subtitle 1="$C" --replace-subtitle 2="$C" --subtitle-language fr
smoke "create-multi"    create-multi --compositions "$C" --output "$C" --standard smpte --frame-rate 24 --subtitle-language en
# new create features: background colour, custom container, splits, input range
smoke "create pad-color" create --title x --video "$C" --output "$C" --pad-head 12f --pad-tail 12f --pad-color ff0000
smoke "create container-dims" create --title x --video "$C" --output "$C" --container-dims 1920x1080
smoke "create split-at"  create --title x --video "$C" --output "$C" --split-at 00:00:01,00:00:02
smoke "create split-chapters" create --title x --video "$C" --output "$C" --split-chapters
smoke "create input-range" create --title x --video "$C" --output "$C" --input-range full
smoke "create sign-language" create --title x --video "$C" --output "$C" --sign-language-video "$C" --sign-language-lang sgn-ase
smoke "create hdr-dci flags" create --title x --video "$C" --output "$C" --hdr-dci --hdr-already-pq
# W6 subtitle wiring: placement / RTL / wrap / 3D depth / font embed
smoke "create subtitle placement" create --title x --video "$C" --output "$C" --subtitle "$C" \
                            --subtitle-halign left --subtitle-valign top --subtitle-vposition 10 \
                            --subtitle-zposition 2.0 --subtitle-rtl on --subtitle-wrap 42 \
                            --subtitle-font "$C" --subtitle-no-subset
smoke "create burn-subtitle" create --title x --video "$C" --output "$C" \
                            --burn-subtitle "$C" --burn-subtitle-font "$C"
smoke "subtitle-edit list"  subtitle-edit -i "$C" --list
smoke "subtitle-edit shift" subtitle-edit -i "$C" -o "$C" --shift-ms 500 --index 1 --text hi \
                            --set-start-ms 0 --set-end-ms 1000 --fps 25
# W5 audio + encode QoL. --start-at +0s returns immediately; dummy input fails
# the J2K branch before any shutdown, so --shutdown-when-done never fires.
smoke "create loudness+upmix" create --title x --video "$C" --output "$C" \
                            --loudness-target leqm=85 --true-peak-ceiling=-1.0 --upmix a
smoke "create start-at+resume" create --title x --video "$C" --output "$C" \
                            --start-at +0s --resume --shutdown-when-done
smoke "create audio-delay" create --title x --video "$C" --output "$C" --audio-delay -250
smoke "create trim"      create --title x --video "$C" --output "$C" --trim-start 48f --trim-end 2s
smoke "create colourspace" create --title x --video "$C" --output "$C" --source-colourspace xyz
smoke "create quality-psnr" create --title x --video "$C" --output "$C" \
                            --video-bit-rate 250 --quality-psnr 45
# KDM formulation + forensic marking, on all three KDM commands
smoke "kdm formulation"  kdm --cpl-id "$UUID" --content-title x --cert "$C" \
                            --signer-cert "$C" --signer-key "$C" -o "$C" \
                            --formulation dci-any -p -a 12
smoke "kdm marking bare" kdm --cpl-id "$UUID" --content-title x --cert "$C" \
                            --signer-cert "$C" --signer-key "$C" -o "$C" \
                            --disable-forensic-marking-audio
smoke "kdm-batch formulation" kdm-batch --cpl-id "$UUID" --content-title x --cert "$C" \
                            --signer-cert "$C" --signer-key "$C" -o "$C" \
                            --formulation dci-any --disable-forensic-marking-picture
smoke "kdm-rewrap formulation" kdm-rewrap --dkdm "$C" --dkdm-key "$C" --cert "$C" \
                            --signer-cert "$C" --signer-key "$C" -o "$C" \
                            --formulation multiple-modified-transitional-1 --device-cert "$C"
smoke "crossfade"        crossfade --a "$C" --b "$C" -o "$C" --overlap 1.0
smoke "mid-side-decode"  mid-side-decode -i "$C" -o "$C" --mid 0 --side 1
smoke "pipeline input-range" pipeline -i "$C" -t x -o "$C" --input-range legal --split-chapters
# disk writer commands
smoke "format-drive"     format-drive "$C" --fs ext2 --label DCP_DELIVERY --yes --image
smoke "check-drive"      check-drive "$C"

# ── KDM distribution commands (cinema db, templates, history, email, cert-fetch)
smoke "cinema add"        cinema --db "$C" add --name X --email a@b.test --notes n
smoke "cinema list"       cinema --db "$C" list
smoke "cinema add-screen" cinema --db "$C" add-screen --cinema X --name S1 --cert "$C" --inline
smoke "cinema remove-screen" cinema --db "$C" remove-screen --cinema X --name S1
smoke "cinema search"     cinema --db "$C" search foo
smoke "cinema import-flm" cinema --db "$C" import-flm "$C"
smoke "cinema remove"     cinema --db "$C" remove --name X
smoke "kdm-history"       kdm-history --history-file "$C" --title x --recipient r --since 2026-01 --until 2026-12
smoke "kdm-template add"  kdm-template --templates-file "$C" add --name preshow --start-offset "0 days" --duration "1 week" --tz-offset "+02:00"
smoke "kdm-template list" kdm-template --templates-file "$C" list
smoke "kdm-template rm"   kdm-template --templates-file "$C" remove --name preshow
# christie is rejected at the vendor stage (no network) but exercises the flags
smoke "cert-fetch"        cert-fetch --vendor christie --serial 123456 --type QXPD -o "$C"
smoke "kdm --template+email" kdm --cpl-id "$UUID" --content-title x --cert "$C" \
                            --signer-cert "$C" --signer-key "$C" -o "$C" \
                            --template preshow --templates-file "$C" --history-file "$C" \
                            --email-to a@b.test --smtp-config "$C"
smoke "kdm-batch cinema"  kdm-batch --cpl-id "$UUID" --content-title x --cinema X --screen X/S1 \
                            --db "$C" --signer-cert "$C" --signer-key "$C" -o "$C" \
                            --template preshow --templates-file "$C" --history-file "$C" \
                            --email-to a@b.test --smtp-config "$C" --email-only-additional

# --encrypt must require --key-out: this call MUST be rejected at parse time.
reject() {
  local label="$1"; shift
  local out
  out=$("$BINARY" "$@" 2>&1 || true)
  if echo "$out" | grep -qiE "$PARSE_ERR"; then
    echo "ok (rejected): $label"
  else
    echo "FAIL: '$label' should have been rejected but parsed"
    FAILURES=$((FAILURES + 1))
  fi
}
reject "create --encrypt without --key-out" \
       create --title x --video "$C" --output "$C" --encrypt
# --split-at and --reel-length are mutually exclusive
reject "create --split-at with --reel-length" \
       create --title x --video "$C" --output "$C" --split-at 00:00:01 --reel-length 20
# --container-dims and --container are mutually exclusive
reject "create --container-dims with --container" \
       create --title x --video "$C" --output "$C" --container-dims 1920x1080 --container 2k-flat
# --sign-language-video requires --sign-language-lang
reject "create --sign-language-video without --sign-language-lang" \
       create --title x --video "$C" --output "$C" --sign-language-video "$C"

# --hdr-dci authors a DCI HDR DCP (ST 2084 / P3-D65 on the picture MXF). It still
# validates the flag combo up front: without --hdr-to-dci-lut or --hdr-already-pq
# it fails loud because the source is not on a PQ path.
refuse() {
  local label="$1"; local needle="$2"; shift 2
  local out
  out=$("$BINARY" "$@" 2>&1 || true)
  if echo "$out" | grep -qiF "$needle"; then
    echo "ok (refused): $label"
  else
    echo "FAIL: '$label' should have printed: $needle"
    FAILURES=$((FAILURES + 1))
  fi
}
refuse "create --hdr-dci needs PQ path" "needs the source path to PQ" \
       create --title x --video "$C" --output "$C" --hdr-dci

# A scene-referred space has no matrix into X'Y'Z', so it must be refused
# rather than approximated, naming the pass that does convert it.
refuse "create --source-colourspace aces" "3D LUT" \
       create --title x --video "$C" --output "$C" --source-colourspace aces
# P3, Rec.2020 and LogC do convert, but only where an encode runs: a codestream
# directory is already encoded.
refuse "create --source-colourspace p3 on J2K input" "already encoded" \
       create --title x --video "$C" --output "$C" --source-colourspace p3
refuse "create --source-colourspace logc on J2K input" "already encoded" \
       create --title x --video "$C" --output "$C" --source-colourspace logc
# a PSNR target outside 20..80 dB is refused before anything is encoded
refuse "create --quality-psnr out of range" "outside the range" \
       create --title x --video "$C" --output "$C" --quality-psnr 5
# --still-length only means something for a single-image --video
refuse "create --still-length without a still" "carries its own length" \
       create --title x --video "$C" --output "$C" --still-length 5s
# --burn-subtitle draws display-RGB text onto decoded frames, so it is refused
# wherever the encoder gets X'Y'Z' already, or nothing to draw on.
BURN="$TMP/cues.srt"
printf '1\n00:00:00,000 --> 00:00:02,000\nhello\n\n' > "$BURN"
VID="$TMP/clip.mov"
: > "$VID"
refuse "create --burn-subtitle with xyz source" "X'Y'Z' already" \
       create --title x --video "$VID" --output "$C" \
       --burn-subtitle "$BURN" --source-colourspace xyz
refuse "create --burn-subtitle with --hdr-already-pq" "X'Y'Z' already" \
       create --title x --video "$VID" --output "$C" \
       --burn-subtitle "$BURN" --hdr-dci --hdr-already-pq
refuse "create --burn-subtitle with an HDR LUT" "X'Y'Z' already" \
       create --title x --video "$VID" --output "$C" \
       --burn-subtitle "$BURN" --hdr-dci --hdr-to-dci-lut "$C"
refuse "create --burn-subtitle on a J2K directory" "already compressed" \
       create --title x --video "$TMP" --output "$C" --burn-subtitle "$BURN"
# a burnt-in subtitle is part of the picture, so it must not also become a
# timed-text track from the same file
refuse "create --burn-subtitle same file as --subtitle" "pick one" \
       create --title x --video "$VID" --output "$C" \
       --burn-subtitle "$BURN" --subtitle "$BURN"
# a device-listing formulation has no device list to name on a batch
refuse "kdm-batch --formulation dci-specific" "spans cinemas" \
       kdm-batch --cpl-id "$UUID" --content-title x --cert "$C" \
       --signer-cert "$C" --signer-key "$C" -o "$C" --formulation dci-specific
# an unknown formulation is a clap value error, not a runtime one
reject "kdm --formulation nonsense" \
       kdm --cpl-id "$UUID" --content-title x --cert "$C" \
       --signer-cert "$C" --signer-key "$C" -o "$C" --formulation nonsense

echo ""
echo "=== Summary ==="
if [[ $FAILURES -eq 0 ]]; then
  echo "All CLI invocations parse successfully."
  exit 0
else
  echo "$FAILURES check(s) failed."
  exit 1
fi
