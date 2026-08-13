#include "aaf_shim.h"

#include <libaaf.h>
#include <stdlib.h>
#include <string.h>

#define AAF_SHIM_ERROR_LENGTH 512

struct AafShimReader {
	AAF_Iface   *aafi;
	AafShimItem *items;
	int32_t      count;
	int32_t      capacity;
	char         last_error[AAF_SHIM_ERROR_LENGTH];
};

/* libaaf reports why a file failed to load through its log, so keep the last
 * error line to hand back instead of a bare return code. */
static void keep_last_error(struct aafLog *log, void *ctxdata, int lib, int type,
                            const char *srcfile, const char *srcfunc, int lineno,
                            const char *msg, void *user)
{
	(void)log;
	(void)ctxdata;
	(void)lib;
	(void)srcfile;
	(void)srcfunc;
	(void)lineno;

	struct AafShimReader *reader = (struct AafShimReader *)user;

	if (!reader || !msg || type != VERB_ERROR) {
		return;
	}

	strncpy(reader->last_error, msg, AAF_SHIM_ERROR_LENGTH - 1);
	reader->last_error[AAF_SHIM_ERROR_LENGTH - 1] = '\0';
}

static AafShimItem *push_item(struct AafShimReader *reader)
{
	if (reader->count == reader->capacity) {
		int32_t capacity = (reader->capacity == 0) ? 32 : reader->capacity * 2;
		AafShimItem *grown = realloc(reader->items, (size_t)capacity * sizeof(AafShimItem));

		if (!grown) {
			return NULL;
		}

		reader->items = grown;
		reader->capacity = capacity;
	}

	AafShimItem *item = &reader->items[reader->count++];
	memset(item, 0, sizeof(*item));
	return item;
}

static void set_edit_rate(AafShimItem *item, aafRational_t *edit_rate)
{
	if (edit_rate) {
		item->edit_rate_numerator = edit_rate->numerator;
		item->edit_rate_denominator = edit_rate->denominator;
	}
}

/* One audio clip can draw its channels from several mono files, so it becomes
 * one item per distinct file: in a flat reel plan each of those files is a
 * source in its own right. */
static int collect_audio_clip(struct AafShimReader *reader, aafiAudioTrack *track,
                              aafiTimelineItem *timelineItem)
{
	aafiAudioClip *clip = aafi_timelineItemToAudioClip(timelineItem);

	if (!clip) {
		return 1;
	}

	int32_t first = reader->count;
	aafiAudioEssencePointer *pointer = NULL;

	AAFI_foreachEssencePointer(clip->essencePointerList, pointer)
	{
		if (!pointer->essenceFile) {
			continue;
		}

		int already_seen = 0;

		for (int32_t i = first; i < reader->count; i++) {
			if (reader->items[i].source_name == pointer->essenceFile->name &&
			    reader->items[i].source_path == pointer->essenceFile->original_file_path) {
				already_seen = 1;
				break;
			}
		}

		if (already_seen) {
			continue;
		}

		AafShimItem *item = push_item(reader);

		if (!item) {
			return 0;
		}

		item->kind = AAF_SHIM_AUDIO_CLIP;
		item->source_name = pointer->essenceFile->name;
		item->source_path = pointer->essenceFile->original_file_path;
		item->track_name = track->name;
		item->track_number = track->number;
		item->position = clip->pos;
		item->length = clip->len;
		item->source_offset = clip->essence_offset;
		set_edit_rate(item, track->edit_rate);
	}

	if (reader->count > first) {
		return 1;
	}

	AafShimItem *item = push_item(reader);

	if (!item) {
		return 0;
	}

	item->kind = AAF_SHIM_CLIP_WITHOUT_SOURCE;
	item->track_name = track->name;
	item->track_number = track->number;
	item->position = clip->pos;
	item->length = clip->len;
	set_edit_rate(item, track->edit_rate);
	return 1;
}

static int collect_audio(struct AafShimReader *reader)
{
	aafiAudioTrack *track = NULL;

	if (!reader->aafi->Audio) {
		return 1;
	}

	AAFI_foreachAudioTrack(reader->aafi, track)
	{
		aafiTimelineItem *timelineItem = NULL;

		AAFI_foreachTrackItem(track, timelineItem)
		{
			if (timelineItem->type == AAFI_AUDIO_CLIP) {
				if (!collect_audio_clip(reader, track, timelineItem)) {
					return 0;
				}

				continue;
			}

			AafShimItem *item = push_item(reader);

			if (!item) {
				return 0;
			}

			item->kind = (timelineItem->type == AAFI_TRANS) ? AAF_SHIM_TRANSITION
			                                                : AAF_SHIM_UNKNOWN_ITEM;
			item->track_name = track->name;
			item->track_number = track->number;
			item->position = timelineItem->pos;
			item->length = timelineItem->len;
			set_edit_rate(item, track->edit_rate);
		}
	}

	return 1;
}

static int collect_video(struct AafShimReader *reader)
{
	aafiVideoTrack *track = NULL;

	if (!reader->aafi->Video) {
		return 1;
	}

	AAFI_foreachVideoTrack(reader->aafi, track)
	{
		aafiTimelineItem *timelineItem = NULL;

		AAFI_foreachTrackItem(track, timelineItem)
		{
			AafShimItem *item = push_item(reader);

			if (!item) {
				return 0;
			}

			item->track_name = track->name;
			item->track_number = track->number;
			item->position = timelineItem->pos;
			item->length = timelineItem->len;
			set_edit_rate(item, track->edit_rate);

			if (timelineItem->type != AAFI_VIDEO_CLIP) {
				item->kind = (timelineItem->type == AAFI_TRANS) ? AAF_SHIM_TRANSITION
				                                                : AAF_SHIM_UNKNOWN_ITEM;
				continue;
			}

			aafiVideoClip *clip = (aafiVideoClip *)timelineItem->data;

			if (!clip || !clip->Essence) {
				item->kind = AAF_SHIM_CLIP_WITHOUT_SOURCE;
				continue;
			}

			item->kind = AAF_SHIM_VIDEO_CLIP;
			item->source_name = clip->Essence->name;
			item->source_path = clip->Essence->original_file_path;
			item->position = clip->pos;
			item->length = clip->len;
			item->source_offset = clip->essence_offset;
		}
	}

	return 1;
}

AafShimReader *aaf_shim_open(const char *path, char *error_out, size_t error_length)
{
	if (error_out && error_length > 0) {
		error_out[0] = '\0';
	}

	if (!path) {
		return NULL;
	}

	struct AafShimReader *reader = calloc(1, sizeof(struct AafShimReader));

	if (!reader) {
		return NULL;
	}

	reader->aafi = aafi_alloc(NULL);

	if (!reader->aafi) {
		free(reader);
		return NULL;
	}

	aafi_set_debug(reader->aafi, VERB_ERROR, 0, NULL, &keep_last_error, reader);

	if (aafi_load_file(reader->aafi, path) != 0 || !collect_audio(reader) || !collect_video(reader)) {
		if (error_out && error_length > 0) {
			strncpy(error_out, reader->last_error, error_length - 1);
			error_out[error_length - 1] = '\0';
		}

		aaf_shim_close(reader);
		return NULL;
	}

	return reader;
}

void aaf_shim_close(AafShimReader *reader)
{
	if (!reader) {
		return;
	}

	if (reader->aafi) {
		aafi_release(&reader->aafi);
	}

	free(reader->items);
	free(reader);
}

void aaf_shim_composition(const AafShimReader *reader, AafShimComposition *out)
{
	if (!reader || !out) {
		return;
	}

	memset(out, 0, sizeof(*out));
	out->name = reader->aafi->compositionName;
	out->start = reader->aafi->compositionStart;
	out->item_count = reader->count;

	if (reader->aafi->compositionStart_editRate) {
		out->start_rate_numerator = reader->aafi->compositionStart_editRate->numerator;
		out->start_rate_denominator = reader->aafi->compositionStart_editRate->denominator;
	}

	if (!reader->aafi->Timecode) {
		return;
	}

	out->timecode_fps = reader->aafi->Timecode->fps;
	out->timecode_drop = reader->aafi->Timecode->drop;

	if (reader->aafi->Timecode->edit_rate) {
		out->frame_rate_numerator = reader->aafi->Timecode->edit_rate->numerator;
		out->frame_rate_denominator = reader->aafi->Timecode->edit_rate->denominator;
	}
}

const AafShimItem *aaf_shim_item(const AafShimReader *reader, int32_t index)
{
	if (!reader || index < 0 || index >= reader->count) {
		return NULL;
	}

	return &reader->items[index];
}
