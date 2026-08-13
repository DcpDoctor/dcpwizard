// DCP Timeline View - renders multi-reel timeline with visual reel segments and playback integration
import { invoke } from '@tauri-apps/api/core';
import { showEmbeddedPanel } from './preview.js';

let timelineData = null; // { reels: [], totalFrames, editRate }
let currentReel = -1;
let playheadFrame = 0;
let timelinePollingId = null;

export function initTimeline() {
  renderEmpty();
}

// Load timeline from an opened DCP's CPL
export async function loadTimelineFromCpl(cplPath) {
  try {
    const reels = await invoke('get_timeline', { cplPath });
    if (!reels || reels.length === 0) {
      renderEmpty();
      return;
    }
    buildTimelineData(reels);
    render();
    startTimelinePolling();
  } catch (e) {
    console.error('[timeline] Failed to load CPL:', e);
    renderEmpty();
  }
}

// Load timeline from the project model (for DCPs being built)
export function loadTimelineFromProject(reels, editRate) {
  if (!reels || reels.length === 0) {
    renderEmpty();
    return;
  }
  // Convert project reels to timeline format
  const entries = reels.map((reel, i) => ({
    reel_id: String(reel.id),
    reel_number: i + 1,
    duration_frames: reel.durationFrames || 0,
    entry_point: 0,
    edit_rate: editRate || '24 1',
    picture_asset_id: '',
    sound_asset_id: '',
    picture_file: reel.picture?.path || '',
    sound_file: reel.sound?.path || '',
  }));
  buildTimelineData(entries);
  render();
}

function buildTimelineData(reels) {
  let totalFrames = 0;
  const parsed = reels.map(r => {
    const fps = parseEditRate(r.edit_rate);
    const startFrame = totalFrames;
    totalFrames += r.duration_frames;
    return { ...r, startFrame, fps };
  });
  timelineData = { reels: parsed, totalFrames, editRate: parsed[0]?.fps || 24 };
}

function parseEditRate(er) {
  if (!er) return 24;
  const parts = er.trim().split(/\s+/);
  if (parts.length === 2) return Math.round(parseInt(parts[0]) / parseInt(parts[1]));
  return parseInt(parts[0]) || 24;
}

function renderEmpty() {
  const ruler = document.getElementById('timeline-ruler');
  const picture = document.getElementById('timeline-picture');
  const sound = document.getElementById('timeline-sound');
  const subtitle = document.getElementById('timeline-subtitle');
  if (ruler) ruler.innerHTML = '<span class="timeline-empty-msg">Open a DCP or build a project to see the timeline</span>';
  if (picture) picture.innerHTML = '';
  if (sound) sound.innerHTML = '';
  if (subtitle) subtitle.innerHTML = '';
  timelineData = null;
}

function render() {
  if (!timelineData || timelineData.reels.length === 0) {
    renderEmpty();
    return;
  }
  renderRuler();
  renderTracks();
  updatePlayheadPosition();
}

function renderRuler() {
  const ruler = document.getElementById('timeline-ruler');
  if (!ruler) return;
  ruler.innerHTML = '';
  ruler.style.position = 'relative';

  const { totalFrames, editRate } = timelineData;
  if (totalFrames === 0) return;

  // Generate tick marks at sensible intervals
  const totalSeconds = totalFrames / editRate;
  const interval = getTickInterval(totalSeconds);

  for (let t = 0; t <= totalSeconds; t += interval) {
    const pct = (t / totalSeconds) * 100;
    const tick = document.createElement('div');
    tick.className = 'ruler-tick';
    tick.style.left = `${pct}%`;
    tick.dataset.time = formatTimecodeShort(t, editRate);

    const label = document.createElement('span');
    label.className = 'ruler-label';
    label.textContent = formatTimecodeShort(t, editRate);
    tick.appendChild(label);

    ruler.appendChild(tick);
  }

  // Reel boundary markers
  for (const reel of timelineData.reels) {
    if (reel.startFrame > 0) {
      const pct = (reel.startFrame / totalFrames) * 100;
      const marker = document.createElement('div');
      marker.className = 'ruler-reel-marker';
      marker.style.left = `${pct}%`;
      marker.title = `Reel ${reel.reel_number}`;
      ruler.appendChild(marker);
    }
  }

  // Playhead in ruler
  const playhead = document.createElement('div');
  playhead.className = 'ruler-playhead';
  playhead.id = 'ruler-playhead';
  ruler.appendChild(playhead);

  // Click on ruler to seek
  ruler.addEventListener('mousedown', handleRulerSeek);
}

function renderTracks() {
  const pictureEl = document.getElementById('timeline-picture');
  const soundEl = document.getElementById('timeline-sound');
  const subtitleEl = document.getElementById('timeline-subtitle');
  if (!pictureEl || !soundEl) return;

  pictureEl.innerHTML = '';
  soundEl.innerHTML = '';
  if (subtitleEl) subtitleEl.innerHTML = '';

  const { reels, totalFrames } = timelineData;
  const colors = ['#7c3aed', '#6d28d9', '#5b21b6', '#4c1d95', '#8b5cf6'];
  const soundColors = ['#3b82f6', '#2563eb', '#1d4ed8', '#1e40af', '#60a5fa'];

  for (const reel of reels) {
    const widthPct = (reel.duration_frames / totalFrames) * 100;
    const leftPct = (reel.startFrame / totalFrames) * 100;

    // Picture segment
    if (reel.picture_file || reel.picture_asset_id) {
      const seg = createSegment(reel, leftPct, widthPct, colors[(reel.reel_number - 1) % colors.length], 'picture');
      pictureEl.appendChild(seg);
    }

    // Sound segment
    if (reel.sound_file || reel.sound_asset_id) {
      const seg = createSegment(reel, leftPct, widthPct, soundColors[(reel.reel_number - 1) % soundColors.length], 'sound');
      soundEl.appendChild(seg);
    }
  }

  // Click on track to seek
  pictureEl.addEventListener('mousedown', handleTrackSeek);
  soundEl.addEventListener('mousedown', handleTrackSeek);
}

function createSegment(reel, leftPct, widthPct, color, type) {
  const seg = document.createElement('div');
  seg.className = 'timeline-segment' + (reel.reel_number - 1 === currentReel ? ' active' : '');
  seg.style.left = `${leftPct}%`;
  seg.style.width = `${widthPct}%`;
  seg.style.backgroundColor = color;
  seg.dataset.reelIndex = reel.reel_number - 1;
  seg.dataset.type = type;

  const label = document.createElement('span');
  label.className = 'segment-label';
  label.textContent = `R${reel.reel_number}`;
  seg.appendChild(label);

  const dur = document.createElement('span');
  dur.className = 'segment-duration';
  dur.textContent = formatTimecodeShort(reel.duration_frames / (reel.fps || 24), reel.fps || 24);
  seg.appendChild(dur);

  return seg;
}

function handleRulerSeek(e) {
  const ruler = document.getElementById('timeline-ruler');
  if (!ruler || !timelineData) return;
  const rect = ruler.getBoundingClientRect();
  const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  seekToPercent(pct);
}

function handleTrackSeek(e) {
  const track = e.currentTarget;
  if (!track || !timelineData) return;
  const rect = track.getBoundingClientRect();
  const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  seekToPercent(pct);
}

async function seekToPercent(pct) {
  if (!timelineData) return;
  const targetFrame = Math.floor(pct * timelineData.totalFrames);

  // Find which reel this frame belongs to
  let targetReel = null;
  for (const reel of timelineData.reels) {
    if (targetFrame >= reel.startFrame && targetFrame < reel.startFrame + reel.duration_frames) {
      targetReel = reel;
      break;
    }
  }
  if (!targetReel) targetReel = timelineData.reels[timelineData.reels.length - 1];

  const reelIdx = targetReel.reel_number - 1;
  const frameInReel = targetFrame - targetReel.startFrame + targetReel.entry_point;
  const secondsInReel = frameInReel / (targetReel.fps || 24);

  // If different reel, load the new file
  if (reelIdx !== currentReel && targetReel.picture_file) {
    currentReel = reelIdx;
    showEmbeddedPanel();
    try {
      await invoke('preview_load', { filePath: targetReel.picture_file });
    } catch (e) {
      console.error('[timeline] Failed to load reel:', e);
      return;
    }
  }

  // Seek within the reel
  try {
    await invoke('preview_seek_absolute', { seconds: secondsInReel });
  } catch (e) {
    console.error('[timeline] Failed to seek:', e);
  }

  playheadFrame = targetFrame;
  updatePlayheadPosition();
}

function updatePlayheadPosition() {
  if (!timelineData || timelineData.totalFrames === 0) return;
  const pct = (playheadFrame / timelineData.totalFrames) * 100;

  const rulerPlayhead = document.getElementById('ruler-playhead');
  if (rulerPlayhead) rulerPlayhead.style.left = `${pct}%`;

  // Update segment active states
  document.querySelectorAll('.timeline-segment').forEach(seg => {
    const idx = parseInt(seg.dataset.reelIndex);
    seg.classList.toggle('active', idx === currentReel);
  });
}

export function startTimelinePolling() {
  if (timelinePollingId) return;
  timelinePollingId = setInterval(async () => {
    if (!timelineData) return;
    try {
      const resp = await invoke('preview_get_metadata');
      const meta = JSON.parse(resp);
      if (meta.position != null && meta.duration != null) {
        // Calculate global frame position
        const reel = timelineData.reels[currentReel] || timelineData.reels[0];
        if (reel) {
          const fps = reel.fps || 24;
          const frameInReel = Math.floor(meta.position * fps) - reel.entry_point;
          playheadFrame = reel.startFrame + Math.max(0, frameInReel);

          // Auto-advance to next reel at end
          if (meta.position >= meta.duration - 0.1 && currentReel < timelineData.reels.length - 1) {
            const nextReel = timelineData.reels[currentReel + 1];
            if (nextReel && nextReel.picture_file) {
              currentReel++;
              invoke('preview_load', { filePath: nextReel.picture_file }).catch(() => {});
            }
          }

          updatePlayheadPosition();
        }
      }
    } catch {
      // mpv not running
    }
  }, 250);
}

export function stopTimelinePolling() {
  if (timelinePollingId) {
    clearInterval(timelinePollingId);
    timelinePollingId = null;
  }
}

// Utility: get a sensible tick interval based on total duration
function getTickInterval(totalSeconds) {
  if (totalSeconds <= 10) return 1;
  if (totalSeconds <= 30) return 5;
  if (totalSeconds <= 60) return 10;
  if (totalSeconds <= 300) return 30;
  if (totalSeconds <= 600) return 60;
  if (totalSeconds <= 1800) return 300;
  return 600;
}

function formatTimecodeShort(seconds, fps) {
  if (!seconds || seconds <= 0) return '0:00';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  return `${m}:${String(s).padStart(2, '0')}`;
}
