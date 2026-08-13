/* Flat C view of an AAF composition, narrowed to what conform needs: tracks,
 * clips with a source name, in and out points, and the edit rates they are
 * counted in. libaaf's own structs are large and full of back pointers, so the
 * Rust side never sees them.
 *
 * Every string points into libaaf's allocations and is valid until
 * aaf_shim_close(). */

#ifndef DCPWIZARD_AAF_SHIM_H
#define DCPWIZARD_AAF_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum AafShimItemKind {
	AAF_SHIM_AUDIO_CLIP = 0,
	AAF_SHIM_VIDEO_CLIP = 1,
	AAF_SHIM_TRANSITION = 2,
	AAF_SHIM_CLIP_WITHOUT_SOURCE = 3,
	AAF_SHIM_UNKNOWN_ITEM = 4
};

typedef struct AafShimItem {
	int32_t     kind;
	const char *source_name;
	const char *source_path;
	const char *track_name;
	uint32_t    track_number;
	/* position, length and source_offset are all counted in the track edit
	 * rate below, which for audio is usually the sample rate */
	int64_t     position;
	int64_t     length;
	int64_t     source_offset;
	int32_t     edit_rate_numerator;
	int32_t     edit_rate_denominator;
	/* Audio clips only, zero on everything else. A clip that draws its channels
	 * from several files becomes several items, and each one repeats the gain
	 * of the clip it came from. */
	double      gain_factor;
	int         has_constant_gain;
	int         has_gain_automation;
	int         muted;
	int         track_has_pan;
} AafShimItem;

typedef struct AafShimComposition {
	const char *name;
	int64_t     start;
	int32_t     start_rate_numerator;
	int32_t     start_rate_denominator;
	int32_t     frame_rate_numerator;
	int32_t     frame_rate_denominator;
	uint16_t    timecode_fps;
	uint8_t     timecode_drop;
	int32_t     item_count;
} AafShimComposition;

typedef struct AafShimReader AafShimReader;

/* Returns NULL on failure, writing libaaf's own last error message into
 * error_out when there is one. */
AafShimReader *aaf_shim_open(const char *path, char *error_out, size_t error_length);
void aaf_shim_close(AafShimReader *reader);
void aaf_shim_composition(const AafShimReader *reader, AafShimComposition *out);
/* NULL when index is out of range. */
const AafShimItem *aaf_shim_item(const AafShimReader *reader, int32_t index);

#ifdef __cplusplus
}
#endif

#endif
