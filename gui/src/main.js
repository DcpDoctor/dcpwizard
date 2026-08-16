import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Command } from "@tauri-apps/plugin-shell";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { open as _open, save, confirm as tauriConfirm, message as tauriMessage } from "@tauri-apps/plugin-dialog";
import { documentDir, join } from "@tauri-apps/api/path";
import { initPreview, previewDcp, previewFile, previewPlayPause, previewSeek, previewSeekAbsolute, isPreviewVisible } from "../../extern/guikit/src/preview.js";
import { initTimeline, loadTimelineFromCpl } from "./timeline.js";
import { initShortcuts, getBinding } from "../../extern/guikit/src/shortcuts.js";

// === Browse wrapper (remembers last directory) ===
let lastBrowseDir = null;
async function open(opts = {}) {
  const result = await _open({ ...opts, defaultPath: opts.defaultPath || lastBrowseDir || undefined });
  if (result) {
    lastBrowseDir = opts.directory ? result : result.replace(/[/\\][^/\\]*$/, '');
  }
  return result;
}

// === Sidebar navigation ===
document.querySelectorAll(".sidebar-btn[data-view]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".sidebar-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
    btn.classList.add("active");
    const view = document.getElementById(`view-${btn.dataset.view}`);
    if (view) view.classList.add("active");

    // Auto-refresh jobs when switching to jobs view
    if (btn.dataset.view === "jobs") {
      refreshJobs();
      startJobsPolling();
    } else {
      stopJobsPolling();
    }
  });
});

// === Theme toggle ===
document.getElementById("theme-toggle")?.addEventListener("click", () => {
  document.body.classList.toggle("light");
  const btn = document.getElementById("theme-toggle");
  btn.textContent = document.body.classList.contains("light") ? "☀️" : "🌙";
});

// === Keyboard shortcuts ===
function switchView(viewName) {
  document.querySelectorAll(".sidebar-btn").forEach((b) => b.classList.remove("active"));
  document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
  const btn = document.querySelector(`.sidebar-btn[data-view="${viewName}"]`);
  if (btn) btn.classList.add("active");
  const view = document.getElementById(`view-${viewName}`);
  if (view) view.classList.add("active");
  if (viewName === "jobs") { refreshJobs(); startJobsPolling(); } else { stopJobsPolling(); }
}

const SHORTCUTS_KEY = "dcpwizard-shortcuts";
const PREVIEW_SEEK_SECONDS = 5;

const PROJECT_BUTTON_SHORTCUTS = [
  { id: "new-project", label: "New project", binding: "Ctrl+N", buttonId: "btn-new-project" },
  { id: "open-project", label: "Open DCP", binding: "Ctrl+O", buttonId: "btn-open-project" },
  { id: "build", label: "Build DCP", binding: "Ctrl+B", buttonId: "btn-build" },
  { id: "preview", label: "Preview", binding: "Ctrl+P", buttonId: "btn-preview" },
  { id: "import-video", label: "Import video", binding: "Ctrl+I", buttonId: "import-video" },
];
const THEME_BUTTON_SHORTCUT = { id: "toggle-theme", label: "Toggle light / dark theme", binding: "Ctrl+Shift+T", buttonId: "theme-toggle" };
const BUTTON_SHORTCUTS = [...PROJECT_BUTTON_SHORTCUTS, THEME_BUTTON_SHORTCUT];

function clickAction({ id, label, binding, buttonId }, category) {
  return { id, label, category, binding, handler: () => document.getElementById(buttonId)?.click() };
}

function viewAction(view, label, binding) {
  return { id: `view-${view}`, label, category: "Views", binding, handler: () => switchView(view) };
}

function previewAction(id, label, binding, handler) {
  return { id, label, category: "Preview", binding, when: isPreviewVisible, handler };
}

function refreshButtonTooltips() {
  for (const { id, label, buttonId } of BUTTON_SHORTCUTS) {
    const button = document.getElementById(buttonId);
    if (!button) continue;
    const binding = getBinding(id);
    button.title = binding ? `${label} (${binding})` : label;
  }
}

initShortcuts({
  storageKey: SHORTCUTS_KEY,
  onChange: refreshButtonTooltips,
  actions: [
    ...PROJECT_BUTTON_SHORTCUTS.map((shortcut) => clickAction(shortcut, "Project")),
    viewAction("project", "Project", "Ctrl+1"),
    viewAction("reels", "Reels & Timeline", "Ctrl+2"),
    viewAction("verify", "Verify", "Ctrl+3"),
    viewAction("security", "Encryption & KDM", "Ctrl+4"),
    viewAction("tools", "Tools", "Ctrl+5"),
    viewAction("jobs", "Jobs", "Ctrl+6"),
    viewAction("settings", "Settings", "Ctrl+7"),
    previewAction("preview-play-pause", "Play / pause", "Space", previewPlayPause),
    previewAction("preview-back", `Back ${PREVIEW_SEEK_SECONDS} seconds`, "ArrowLeft", () => previewSeek(-PREVIEW_SEEK_SECONDS)),
    previewAction("preview-forward", `Forward ${PREVIEW_SEEK_SECONDS} seconds`, "ArrowRight", () => previewSeek(PREVIEW_SEEK_SECONDS)),
    previewAction("preview-start", "Go to start", "Home", () => previewSeekAbsolute(0)),
    clickAction(THEME_BUTTON_SHORTCUT, "Appearance"),
  ],
});
refreshButtonTooltips();

// === Preferences (localStorage) ===
const PREFS_KEY = "dcpwizard-preferences";
const PREFS_VERSION = 4;

// clear of DCI's 250 on purpose: at 250 exactly, rate allocation overshoot is a
// peak bitrate failure, and validators warn from 230 up
const DEFAULT_BANDWIDTH_MBPS = 230;
const DEFAULT_FRAMERATE = 24;

const PREF_DEFAULTS = {
  standard: "SMPTE", resolution: "2K", framerate: DEFAULT_FRAMERATE,
  encrypt: false, stereo3d: false, validate: true,
  creator: "", facility: "", bandwidth: DEFAULT_BANDWIDTH_MBPS, gpu: -1,
  signingCert: "", signingKey: "", outputDir: "", naming: "",
  channels: "5.1",
};

function getPrefs() {
  try {
    const stored = JSON.parse(localStorage.getItem(PREFS_KEY)) || {};
    if ((stored._version || 0) < PREFS_VERSION) {
      const migrated = { ...PREF_DEFAULTS, ...stored, _version: PREFS_VERSION };
      // a saved 250 encodes at the DCI ceiling and fails validation
      migrated.bandwidth = Math.min(migrated.bandwidth, DEFAULT_BANDWIDTH_MBPS);
      savePrefs(migrated);
      return migrated;
    }
    return { ...PREF_DEFAULTS, ...stored };
  } catch { return { ...PREF_DEFAULTS, _version: PREFS_VERSION }; }
}

function savePrefs(prefs) {
  prefs._version = PREFS_VERSION;
  localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
}

// Load prefs into settings form
function loadSettings() {
  const prefs = getPrefs();
  const map = {
    "set-standard": prefs.standard,
    "set-resolution": prefs.resolution,
    "set-framerate": prefs.framerate,
    "set-creator": prefs.creator,
    "set-facility": prefs.facility,
    "set-bandwidth": prefs.bandwidth,
    "set-signing-cert": prefs.signingCert,
    "set-signing-key": prefs.signingKey,
    "set-output-dir": prefs.outputDir,
    "set-naming": prefs.naming,
  };
  for (const [id, val] of Object.entries(map)) {
    const el = document.getElementById(id);
    if (el) el.value = val;
  }
}

document.getElementById("settings-form")?.addEventListener("submit", (e) => {
  e.preventDefault();
  const prefs = {
    standard: document.getElementById("set-standard")?.value,
    resolution: document.getElementById("set-resolution")?.value,
    framerate: parseInt(document.getElementById("set-framerate")?.value) || DEFAULT_FRAMERATE,
    creator: document.getElementById("set-creator")?.value,
    facility: document.getElementById("set-facility")?.value,
    bandwidth: parseInt(document.getElementById("set-bandwidth")?.value) || DEFAULT_BANDWIDTH_MBPS,
    signingCert: document.getElementById("set-signing-cert")?.value,
    signingKey: document.getElementById("set-signing-key")?.value,
    outputDir: document.getElementById("set-output-dir")?.value,
    naming: document.getElementById("set-naming")?.value,
  };
  savePrefs(prefs);
  setStatus("Settings saved");
});

document.getElementById("set-reset")?.addEventListener("click", () => {
  localStorage.removeItem(PREFS_KEY);
  location.reload();
});

loadSettings();

// === Project State ===
const project = {
  title: "",
  assets: [],  // {id, type: 'video'|'audio'|'subtitle', path, name, meta}
  compositions: [
    { id: 1, name: "Main", contentKind: "feature", reels: [{ id: 1, picture: null, sound: null, subtitle: null }] }
  ],
  activeComposition: 0,  // index into compositions[]
};

// Convenience accessor for active composition reels
function getActiveReels() {
  return project.compositions[project.activeComposition]?.reels || [];
}
function setActiveReels(reels) {
  if (project.compositions[project.activeComposition]) {
    project.compositions[project.activeComposition].reels = reels;
  }
}

// Legacy alias for backward compat in this file
Object.defineProperty(project, 'reels', {
  get() { return getActiveReels(); },
  set(v) { setActiveReels(v); },
  configurable: true,
});

let nextAssetId = 1;

// === OS file drop (Tauri webview drag-drop) ===
// A webview's HTML drop event exposes no filesystem path (f.path is Electron
// only). Tauri's native drag-drop event carries absolute paths.
const dropOverlay = document.getElementById("drop-overlay");

getCurrentWebview().onDragDropEvent((event) => {
  const p = event.payload;
  if (p.type === "over" || p.type === "enter") {
    if (dropOverlay) dropOverlay.hidden = false;
  } else if (p.type === "drop") {
    if (dropOverlay) dropOverlay.hidden = true;
    for (const path of p.paths) {
      importAssetFromPath(path, guessType(path));
    }
  } else {
    // "leave" / "cancel"
    if (dropOverlay) dropOverlay.hidden = true;
  }
});

function guessType(name) {
  const ext = name.split('.').pop().toLowerCase();
  if (['mp4','mkv','mov','avi','mxf','webm','j2c','tiff','tif','dpx','exr'].includes(ext)) return 'video';
  if (['wav','aiff','flac','mp3','pcm'].includes(ext)) return 'audio';
  if (['xml','ttml','srt','vtt'].includes(ext)) return 'subtitle';
  return 'video';
}

// === Asset import ===
document.getElementById("import-video")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Video', extensions: ['mp4','mkv','mov','avi','mxf','webm'] },
      { name: 'Still image', extensions: ['png','jpg','jpeg','tif','tiff','bmp','dpx','exr'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) importAssetFromPath(path, 'video');
});

document.getElementById("import-audio")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Audio', extensions: ['wav','aiff','flac','mp3'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) importAssetFromPath(path, 'audio');
});

document.getElementById("import-subtitle")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Subtitle', extensions: ['xml','ttml','srt','vtt'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) importAssetFromPath(path, 'subtitle');
});

function importAssetFromPath(path, type) {
  const name = path.split(/[/\\]/).pop();
  const asset = { id: nextAssetId++, type, path, name, meta: '' };
  project.assets.push(asset);

  // Auto-assign to first reel if empty
  const reel = project.reels[0];
  if (type === 'video' && !reel.picture) {
    reel.picture = asset;
  } else if (type === 'audio' && !reel.sound) {
    reel.sound = asset;
  } else if (type === 'subtitle' && !reel.subtitle) {
    reel.subtitle = asset;
  }

  renderAssets();
  renderReels();
  updateStatusStats();
  setStatus(`Imported: ${name}`);

  // Auto-detect video properties
  if (type === 'video') {
    probeVideo(path).then(info => {
      if (!info) return;
      asset.meta = `${info.width}×${info.height} ${info.fps}`;
      if (project.assets.filter(a => a.type === 'video').length === 1) {
        // Pre-fill resolution from first video
        const resEl = document.getElementById("prop-resolution");
        if (resEl && resEl.value === "auto") {
          // Keep auto — the backend will handle it
        }
        // Pre-fill framerate
        const fpsMatch = info.fps?.match(/^(\d+)\/1$/);
        if (fpsMatch) {
          const fpsEl = document.getElementById("prop-framerate");
          if (fpsEl) {
            const fps = parseInt(fpsMatch[1]);
            for (const opt of fpsEl.options) {
              if (parseInt(opt.value) === fps) { fpsEl.value = opt.value; break; }
            }
          }
        }
      }
      renderAssets();
    });
  }
}

function renderAssets() {
  const list = document.getElementById("asset-list");
  if (!list) return;

  if (project.assets.length === 0) {
    list.innerHTML = '<div class="asset-empty"><p>Drag & drop video/audio files here<br>or use the buttons above</p></div>';
    return;
  }

  const icons = { video: '🎬', audio: '🔊', subtitle: '📝' };
  list.innerHTML = project.assets.map(a => `
    <div class="asset-item" data-asset-id="${a.id}" draggable="true">
      <span class="asset-icon">${icons[a.type]}</span>
      <span class="asset-name" title="${a.path}">${a.name}</span>
      <span class="asset-meta">${a.meta || a.type}</span>
      <button class="asset-remove" data-remove-id="${a.id}" title="Remove from project">✕</button>
    </div>
  `).join('');

  // Make assets draggable to reel tracks
  list.querySelectorAll('.asset-item').forEach(el => {
    el.addEventListener('dragstart', (e) => {
      e.dataTransfer.setData('text/plain', el.dataset.assetId);
    });
    el.addEventListener('contextmenu', (e) => {
      showContextMenu(e, parseInt(el.dataset.assetId));
    });
  });
  list.querySelectorAll('.asset-remove').forEach(el => {
    el.addEventListener('click', (e) => { e.stopPropagation(); removeAsset(parseInt(el.dataset.removeId)); });
  });

  // Re-apply filter
  const q = document.getElementById("asset-filter")?.value?.toLowerCase() || "";
  if (q) {
    list.querySelectorAll('.asset-item').forEach(el => {
      const name = el.querySelector(".asset-name")?.textContent?.toLowerCase() || "";
      el.style.display = name.includes(q) ? "" : "none";
    });
  }
}

function renderReels() {
  const list = document.getElementById("reel-list");
  if (!list) return;

  list.innerHTML = project.reels.map((reel, i) => `
    <div class="reel" data-reel="${reel.id}">
      <div class="reel-header">
        <span class="reel-label">Reel ${i + 1}</span>
        <span class="reel-duration">${reel.picture ? '—' : '--:--:--'}</span>
      </div>
      <div class="reel-tracks">
        <div class="track track-picture" data-reel-id="${reel.id}" data-track="picture">
          <span class="track-label">Picture</span>
          <span class="track-info ${reel.picture ? 'has-content' : ''}">${reel.picture ? reel.picture.name : 'Drop video here'}</span>
        </div>
        <div class="track track-sound" data-reel-id="${reel.id}" data-track="sound">
          <span class="track-label">Sound</span>
          <span class="track-info ${reel.sound ? 'has-content' : ''}">${reel.sound ? reel.sound.name : 'Drop audio here'}</span>
        </div>
        <div class="track track-subtitle" data-reel-id="${reel.id}" data-track="subtitle">
          <span class="track-label">Subtitle</span>
          <span class="track-info ${reel.subtitle ? 'has-content' : ''}">${reel.subtitle ? reel.subtitle.name : 'Optional'}</span>
        </div>
      </div>
    </div>
  `).join('');

  // Drop targets on reel tracks
  list.querySelectorAll('.track').forEach(track => {
    track.addEventListener('dragover', (e) => {
      e.preventDefault();
      track.style.background = 'var(--surface-hover)';
    });
    track.addEventListener('dragleave', () => {
      track.style.background = '';
    });
    track.addEventListener('drop', (e) => {
      e.preventDefault();
      track.style.background = '';
      const assetId = parseInt(e.dataTransfer.getData('text/plain'));
      const asset = project.assets.find(a => a.id === assetId);
      if (!asset) return;
      const reelId = parseInt(track.dataset.reelId);
      const reel = project.reels.find(r => r.id === reelId);
      if (!reel) return;
      const trackType = track.dataset.track;
      reel[trackType] = asset;
      renderReels();
    });
  });

  refreshAudioMapMatrix();
}

// === Source picture ===

document.getElementById("prop-auto-crop")?.addEventListener("click", async () => {
  const video = project.reels[0]?.picture?.path;
  const plan = document.getElementById("prop-crop-plan");
  if (!video) { tauriMessage("Import a video asset first"); return; }
  try {
    const crop = await invoke("detect_source_crop", {
      videoPath: video,
      threshold: parseFloat(document.getElementById("prop-auto-crop-threshold")?.value) || null,
      resolution: document.getElementById("prop-resolution")?.value || null,
    });
    for (const edge of ["left", "right", "top", "bottom"]) {
      const field = document.getElementById(`prop-crop-${edge}`);
      if (field) field.value = crop[edge];
    }
    if (plan) plan.textContent = crop.description;
  } catch (e) {
    if (plan) plan.textContent = "";
    tauriMessage(String(e), { title: "Auto-crop failed", kind: "error" });
  }
});

// === Audio channel mapping matrix ===

// the path the drawn matrix belongs to, so re-rendering the reels does not throw
// away gains the user has typed
let audioMapPath = null;

async function refreshAudioMapMatrix() {
  const grid = document.getElementById("prop-audio-map");
  const hint = document.getElementById("prop-audio-map-hint");
  if (!grid) return;
  const audio = project.reels[0]?.sound?.path || null;
  if (audio === audioMapPath) return;
  audioMapPath = audio;
  grid.innerHTML = "";
  if (!audio) {
    if (hint) hint.textContent = "Import a sound asset to map its channels.";
    return;
  }
  let panel;
  try {
    panel = await invoke("probe_audio_map", { audioPath: audio });
  } catch (e) {
    if (hint) hint.textContent = String(e);
    return;
  }
  if (hint) hint.textContent = "Empty leaves a channel unrouted. Click a cell to route it at 0 dB.";
  const header = panel.lanes.map(lane => `<th>${lane}</th>`).join("");
  const rows = Array.from({ length: panel.channels }, (_, channel) => {
    const cells = panel.lanes.map(lane =>
      `<td><input type="text" inputmode="decimal" data-input="${channel + 1}" data-lane="${lane}" title="Channel ${channel + 1} to ${lane}, gain in dB"></td>`
    ).join("");
    return `<tr><th>${channel + 1}</th>${cells}</tr>`;
  }).join("");
  grid.innerHTML = `<table><thead><tr><th></th>${header}</tr></thead><tbody>${rows}</tbody></table>`;
  grid.querySelectorAll("input").forEach(cell => {
    cell.addEventListener("focus", () => {
      if (!cell.value.trim()) cell.value = "0";
    });
  });
}

// The grid as an IN:LANE@GAIN spec, or null when nothing is routed.
function audioMapSpec() {
  const cells = document.querySelectorAll("#prop-audio-map input");
  const entries = [];
  for (const cell of cells) {
    const gain = cell.value.trim();
    if (!gain) continue;
    const pair = `${cell.dataset.input}:${cell.dataset.lane}`;
    entries.push(parseFloat(gain) === 0 ? pair : `${pair}@${gain}`);
  }
  return entries.length ? entries.join(",") : null;
}

// Add reel button
document.getElementById("add-reel")?.addEventListener("click", () => {
  const maxId = project.reels.reduce((m, r) => Math.max(m, r.id), 0);
  project.reels.push({ id: maxId + 1, picture: null, sound: null, subtitle: null });
  renderReels();
});

// === Multi-CPL Management ===
let nextCplId = 2;

function renderCplTabs() {
  const container = document.getElementById("cpl-tabs");
  if (!container) return;
  container.innerHTML = "";
  project.compositions.forEach((cpl, idx) => {
    const tab = document.createElement("button");
    tab.className = "cpl-tab" + (idx === project.activeComposition ? " active" : "");
    tab.dataset.cpl = idx;
    tab.textContent = cpl.name;
    if (project.compositions.length > 1) {
      const rm = document.createElement("span");
      rm.className = "cpl-tab-remove";
      rm.textContent = "\u00d7";
      rm.addEventListener("click", (e) => {
        e.stopPropagation();
        removeCpl(idx);
      });
      tab.appendChild(rm);
    }
    tab.addEventListener("click", () => switchCpl(idx));
    container.appendChild(tab);
  });
}

function switchCpl(idx) {
  if (idx < 0 || idx >= project.compositions.length) return;
  project.activeComposition = idx;
  renderCplTabs();
  renderReels();
  // Update properties panel content kind
  const cpl = project.compositions[idx];
  const kindEl = document.getElementById("prop-content-kind");
  if (kindEl && cpl.contentKind) kindEl.value = cpl.contentKind;
}

function removeCpl(idx) {
  if (project.compositions.length <= 1) return;
  project.compositions.splice(idx, 1);
  if (project.activeComposition >= project.compositions.length) {
    project.activeComposition = project.compositions.length - 1;
  }
  renderCplTabs();
  renderReels();
}

document.getElementById("add-cpl")?.addEventListener("click", () => {
  const name = prompt("Composition name:", `CPL ${nextCplId}`);
  if (!name) return;
  project.compositions.push({
    id: nextCplId++,
    name: name,
    contentKind: "feature",
    reels: [{ id: 1, picture: null, sound: null, subtitle: null }],
  });
  switchCpl(project.compositions.length - 1);
});

// Sync content kind changes to active composition
document.getElementById("prop-content-kind")?.addEventListener("change", (e) => {
  const cpl = project.compositions[project.activeComposition];
  if (cpl) cpl.contentKind = e.target.value;
});

// Initial render
renderCplTabs();

// === Output directory ===
document.getElementById("browse-output")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) {
    const outputEl = document.getElementById("prop-output");
    outputEl.value = dir;
    delete outputEl.dataset.autoFilled;
    refreshDiskSpace();
  }
});

document.getElementById("prop-output")?.addEventListener("input", (event) => {
  delete event.target.dataset.autoFilled;
});

// === Open existing DCP ===
document.getElementById("btn-open-project")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) {
    // Load as a DCP to preview/verify
    const name = dir.split(/[/\\]/).pop();
    document.getElementById("project-name").textContent = name;
    project.title = name;
    document.getElementById("prop-title").value = name;
    addRecentProject(dir, name);
    setStatus(`Opened: ${dir}`);
    previewDcp(dir);

    // Load timeline from the first CPL found
    try {
      const cpls = await invoke('list_cpls', { dcpDir: dir });
      if (cpls && cpls.length > 0) {
        const cplPath = dir + '/' + cpls[0].file_path;
        loadTimelineFromCpl(cplPath);
      }
    } catch (e) {
      console.warn('[main] Could not load timeline:', e);
    }
  }
});

// === Build DCP ===
let currentJobId = null;
let paused = false;

document.getElementById("prop-browse-key-out")?.addEventListener("click", async () => {
  const file = await save({
    defaultPath: "dcp.keys.json",
    filters: [{ name: "Keys", extensions: ["json"] }],
  });
  if (file) document.getElementById("prop-key-out").value = file;
});

document.getElementById("prop-browse-right-eye")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Video', extensions: ['mp4','mkv','mov','avi','mxf','webm'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-right-eye").value = path;
});

document.getElementById("prop-browse-atmos")?.addEventListener("click", async () => {
  const path = await open({ directory: false, multiple: false });
  if (path) document.getElementById("prop-atmos").value = path;
});

// === Delivery profiles ===
// A profile fills the panel controls when it is picked. Whatever is in the
// controls at build time wins, so editing a field afterwards overrides the
// profile, and the edited field drops out of the "set by" hint.
const PROFILE_DRIVEN_FIELDS = [
  ["standard", "prop-standard", "standard"],
  ["resolution", "prop-resolution", "resolution"],
  ["framerate", "prop-framerate", "frame rate"],
  ["bandwidth", "prop-bandwidth", "bandwidth"],
  ["content_kind", "prop-content-kind", "content kind"],
];

let profileSettings = [];

function updateProfileHint(profileName, drivenLabels) {
  const hint = document.getElementById("prop-profile-hint");
  if (!hint) return;
  hint.textContent = drivenLabels.length
    ? `Set by ${profileName}: ${drivenLabels.join(", ")}. Edit any field to override it.`
    : "";
}

function applyProfile(profileName) {
  const driven = [];
  for (const [, elementId] of PROFILE_DRIVEN_FIELDS) {
    document.getElementById(elementId)?.classList.remove("profile-driven");
  }
  const profile = profileSettings.find((p) => p.name === profileName);
  if (!profile) {
    updateProfileHint("", driven);
    return;
  }
  for (const [key, elementId, label] of PROFILE_DRIVEN_FIELDS) {
    const value = profile[key];
    const element = document.getElementById(elementId);
    if (value === null || value === undefined || !element) continue;
    element.value = value;
    element.classList.add("profile-driven");
    driven.push(label);
  }
  updateProfileHint(profileName, driven);
  setStatus(`Profile ${profileName}: ${profile.description}`);
}

(async () => {
  const select = document.getElementById("prop-profile");
  if (!select) return;
  try {
    profileSettings = await invoke("list_profiles");
  } catch (e) {
    console.warn("[main] Could not load delivery profiles:", e);
    return;
  }
  for (const profile of profileSettings) {
    const option = document.createElement("option");
    option.value = profile.name;
    option.textContent = `${profile.name} — ${profile.description}`;
    select.appendChild(option);
  }
  select.addEventListener("change", (e) => applyProfile(e.target.value));
  for (const [, elementId] of PROFILE_DRIVEN_FIELDS) {
    document.getElementById(elementId)?.addEventListener("input", (e) => {
      e.target.classList.remove("profile-driven");
    });
  }
})();

document.getElementById("prop-browse-versions")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Versions manifest', extensions: ['json'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-versions").value = path;
});

document.getElementById("prop-browse-audio-channel-dir")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) document.getElementById("prop-audio-channel-dir").value = dir;
});

document.getElementById("prop-browse-sign-language-video")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Video', extensions: ['mp4','mkv','mov','avi','mxf','webm'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-sign-language-video").value = path;
});

document.getElementById("prop-browse-hdr-lut")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: '3D LUT', extensions: ['cube','3dl','csp','dat','m3d'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-hdr-lut").value = path;
});

document.getElementById("prop-browse-burn-subtitle")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Subtitles', extensions: ['srt','ass','ssa','pac','mks','mkv','fcpxml','xml'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-burn-subtitle").value = path;
});

document.getElementById("prop-browse-burn-subtitle-font")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Fonts', extensions: ['ttf','otf','ttc'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-burn-subtitle-font").value = path;
});

document.getElementById("prop-browse-ccap")?.addEventListener("click", async () => {
  const path = await open({
    directory: false, multiple: false,
    filters: [
      { name: 'Captions', extensions: ['srt','xml','ttml','vtt'] },
      { name: 'All', extensions: ['*'] }
    ]
  });
  if (path) document.getElementById("prop-ccap").value = path;
});

document.getElementById("btn-build")?.addEventListener("click", async () => {
  // a second build would queue behind the first and encode all over again
  if (buildInFlight) return;

  const title = document.getElementById("prop-title")?.value?.trim();
  if (!title) { tauriMessage("Enter a project title in Properties"); return; }

  const reel = project.reels[0];
  if (!reel?.picture) { tauriMessage("Import a video asset first"); return; }

  const encrypt = document.getElementById("prop-encrypt")?.checked || false;
  const keyOut = document.getElementById("prop-key-out")?.value || "";
  if (encrypt && !keyOut) {
    tauriMessage("Encryption is on: choose a Key Output File in Properties. It holds the plaintext content keys, keep it secret and outside the DCP.");
    return;
  }

  const video = reel.picture.path;
  const audio = reel.sound?.path || null;
  // re-derive an auto-filled output folder so it follows the current title
  const outputEl = document.getElementById("prop-output");
  let output = outputEl?.value;
  if (!output || outputEl?.dataset.autoFilled) {
    const docs = await documentDir();
    output = await join(docs, title);
    if (outputEl) {
      outputEl.value = output;
      outputEl.dataset.autoFilled = "1";
    }
  }

  // Show progress
  const progressSection = document.getElementById("progress-section");
  const progressBar = document.getElementById("progress-bar");
  const stageEl = document.getElementById("progress-stage");
  const statsEl = document.getElementById("progress-stats");
  progressSection.style.display = "flex";
  progressBar.value = 0;
  stageEl.textContent = "Queued...";
  statsEl.textContent = "";
  paused = false;
  resetStatusBar();

  const unlisten = await listen("pipeline-progress", (event) => {
    const p = event.payload;
    if (currentJobId && p.job_id !== currentJobId) return;

    progressBar.value = p.percent;
    stageEl.textContent = p.stage.charAt(0).toUpperCase() + p.stage.slice(1);
    setTitleProgress(p.percent, p.stage);

    const elapsed = formatTime(p.elapsed_secs);
    let remaining = "";
    if (p.percent > 0 && p.percent < 100) {
      const eta = (p.elapsed_secs / p.percent) * (100 - p.percent);
      remaining = ` ETA ${formatTime(eta)}`;
    }
    const fpsStr = p.fps > 0 ? ` ${p.fps.toFixed(1)}fps` : "";
    statsEl.textContent = `${elapsed}${fpsStr}${remaining}`;

    if (p.stage === "done") {
      setStatus("Build complete");
      setTitleProgress(-1);
      notifyBuildComplete(true, title);
      addRecentProject(output, title);
      endBuild();
      unlisten();
      unlistenVal();
    } else if (p.stage === "cancelled") {
      setStatus("Cancelled");
      stageEl.textContent = "Cancelled";
      setTitleProgress(-1);
      endBuild();
      unlisten();
      unlistenVal();
    } else if (p.stage === "error") {
      setStatus("Build failed: " + p.message);
      setTitleProgress(-1);
      notifyBuildComplete(false, title);
      tauriMessage(p.message, { title: "Build failed", kind: "error" });
      endBuild();
      unlisten();
      unlistenVal();
    }
  });

  const unlistenVal = await listen("validation-result", (event) => {
    const v = event.payload;
    if (currentJobId && v.job_id !== currentJobId) return;
    lastValidation = v;
    const validEl = document.getElementById("status-validation");
    validEl.title = "Click for details";
    if (v.valid) {
      validEl.textContent = "✓ Valid";
      validEl.style.color = "#34d399";
    } else {
      validEl.textContent = `✗ ${(v.errors||[]).length} errors`;
      validEl.style.color = "#ff6b6b";
    }
  });

  try {
    beginBuild();
    currentJobId = await invoke("submit_job", {
      videoPath: video,
      title,
      outputDir: output,
      audioPath: audio,
      validate: document.getElementById("prop-validate")?.checked || false,
      standard: document.getElementById("prop-standard")?.value || "smpte",
      resolution: document.getElementById("prop-resolution")?.value || "2k-full",
      framerate: document.getElementById("prop-framerate")?.value || String(DEFAULT_FRAMERATE),
      bandwidth: parseInt(document.getElementById("prop-bandwidth")?.value) || DEFAULT_BANDWIDTH_MBPS,
      contentKind: document.getElementById("prop-content-kind")?.value || "feature",
      encrypt,
      keyOut: keyOut || null,
      rightEye: document.getElementById("prop-right-eye")?.value || null,
      atmos: document.getElementById("prop-atmos")?.value || null,
      subtitle: reel.subtitle?.path || null,
      subtitleLanguage: document.getElementById("prop-subtitle-language")?.value || "en",
      burnSubtitle: document.getElementById("prop-burn-subtitle")?.value || null,
      burnSubtitleFont: document.getElementById("prop-burn-subtitle-font")?.value || null,
      ccap: document.getElementById("prop-ccap")?.value || null,
      ccapLanguage: document.getElementById("prop-ccap-language")?.value || "en",
      loudnessTarget: document.getElementById("prop-loudness")?.value || null,
      truePeakCeiling: parseFloat(document.getElementById("prop-true-peak")?.value) || null,
      audioChannelDir: document.getElementById("prop-audio-channel-dir")?.value || null,
      audioMap: audioMapSpec(),
      audioInputOrder: document.getElementById("prop-audio-input-order")?.value || "dcp",
      signLanguageVideo: document.getElementById("prop-sign-language-video")?.value || null,
      signLanguageTag: document.getElementById("prop-sign-language-tag")?.value || null,
      padHead: document.getElementById("prop-pad-head")?.value || null,
      padTail: document.getElementById("prop-pad-tail")?.value || null,
      padColor: document.getElementById("prop-pad-color")?.value || null,
      audioDelayMs: parseInt(document.getElementById("prop-audio-delay")?.value) || 0,
      trimStart: document.getElementById("prop-trim-start")?.value || null,
      trimEnd: document.getElementById("prop-trim-end")?.value || null,
      stillLength: document.getElementById("prop-still-length")?.value || null,
      sourceColourspace: document.getElementById("prop-source-colourspace")?.value || "rec709",
      cropLeft: parseInt(document.getElementById("prop-crop-left")?.value) || 0,
      cropRight: parseInt(document.getElementById("prop-crop-right")?.value) || 0,
      cropTop: parseInt(document.getElementById("prop-crop-top")?.value) || 0,
      cropBottom: parseInt(document.getElementById("prop-crop-bottom")?.value) || 0,
      fillCrop: document.getElementById("prop-fill-crop")?.checked || false,
      deinterlace: document.getElementById("prop-deinterlace")?.checked || false,
      denoise: document.getElementById("prop-denoise")?.checked || false,
      rotate: document.getElementById("prop-rotate")?.value || "none",
      flip: document.getElementById("prop-flip")?.value || "none",
      upmix: document.getElementById("prop-upmix")?.value || "none",
      reelLengthMinutes: parseInt(document.getElementById("prop-reel-length")?.value) || 0,
      splitAt: document.getElementById("prop-split-at")?.value || null,
      splitChapters: document.getElementById("prop-split-chapters")?.checked || false,
      versions: document.getElementById("prop-versions")?.value || null,
      hdrDci: document.getElementById("prop-hdr-dci")?.checked || false,
      hdrToDciLut: document.getElementById("prop-hdr-lut")?.value || null,
      hdrAlreadyPq: document.getElementById("prop-hdr-already-pq")?.checked || false,
      allowGenericHdrTonemap: document.getElementById("prop-hdr-generic-tonemap")?.checked || false,
    });
    setStatus("Building DCP...");
  } catch (e) {
    stageEl.textContent = "Failed";
    setStatus("Error: " + e);
    tauriMessage(String(e), { title: "Build failed", kind: "error" });
    endBuild();
    unlisten();
    unlistenVal();
  }
});

// Cancel button in progress bar
document.getElementById("progress-cancel")?.addEventListener("click", async () => {
  if (currentJobId) {
    await invoke("cancel_job", { jobId: currentJobId });
    setStatus("Cancelled");
  }
});

// === Preview ===
document.getElementById("btn-preview")?.addEventListener("click", async () => {
  const output = document.getElementById("prop-output")?.value;
  if (output) {
    // Try to preview the built DCP
    previewDcp(output);
  } else {
    // Preview the first video asset
    const reel = project.reels[0];
    if (reel?.picture) previewFile(reel.picture.path);
  }
});

// === Verify ===
document.getElementById("verify-browse")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) {
    document.getElementById("verify-path").textContent = dir;
    document.getElementById("verify-run").disabled = false;
  }
});

document.getElementById("verify-run")?.addEventListener("click", async () => {
  const dir = document.getElementById("verify-path").textContent;
  if (!dir || dir.startsWith("No ")) return;

  const resultsBox = document.getElementById("verify-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Verifying...";

  const args = ["verify", dir];
  if (document.getElementById("verify-strict")?.checked) args.push("--strict");
  if (!document.getElementById("verify-mxf")?.checked) args.push("--no-picture-check");
  if (!document.getElementById("verify-hashes")?.checked) args.push("--no-hash-check");

  const cmd = Command.sidecar("dcpwizard", args);
  const result = await cmd.execute();
  if (result.code === 0) {
    resultsBox.textContent = "✓ DCP verification PASSED\n\n" + result.stdout;
    setStatus("Verification passed");
  } else {
    resultsBox.textContent = "✗ Verification failed\n\n" + (result.stderr || result.stdout);
    setStatus("Verification failed");
  }
});

// === Security: Encrypt ===
document.getElementById("crypt-browse-dcp")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) {
    document.getElementById("crypt-dcp").value = dir;
    checkEncryptReady();
  }
});
document.getElementById("crypt-browse-cert")?.addEventListener("click", async () => {
  const file = await open({ directory: false });
  if (file) {
    document.getElementById("crypt-cert").value = file;
    checkEncryptReady();
  }
});

function checkEncryptReady() {
  const btn = document.getElementById("run-encrypt");
  if (btn) btn.disabled = !(document.getElementById("crypt-dcp")?.value && document.getElementById("crypt-cert")?.value);
}

document.getElementById("run-encrypt")?.addEventListener("click", async () => {
  const resultsBox = document.getElementById("encrypt-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "To create an encrypted DCP: in the Properties panel, enable Encrypt and set a Key Output File (it holds the plaintext content keys, keep it secret and outside the DCP), then build. Feed that keys file to the KDM panel's Keys File field.\nStandalone encryption of an existing DCP is not currently supported.";
});

// === Security: KDM ===
document.getElementById("kdm-browse-dcp")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("kdm-dcp").value = dir; checkKdmReady(); }
});
document.getElementById("kdm-browse-cert")?.addEventListener("click", async () => {
  const file = await open({ directory: false });
  if (file) { document.getElementById("kdm-cert").value = file; checkKdmReady(); }
});
document.getElementById("kdm-browse-signer-cert")?.addEventListener("click", async () => {
  const file = await open({ directory: false });
  if (file) { document.getElementById("kdm-signer-cert").value = file; checkKdmReady(); }
});
document.getElementById("kdm-browse-signer-key")?.addEventListener("click", async () => {
  const file = await open({ directory: false });
  if (file) { document.getElementById("kdm-signer-key").value = file; checkKdmReady(); }
});
document.getElementById("kdm-browse-keys")?.addEventListener("click", async () => {
  const file = await open({ directory: false });
  if (file) { document.getElementById("kdm-keys").value = file; checkKdmReady(); }
});
document.getElementById("kdm-browse-output")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("kdm-output").value = dir + "/kdm.xml"; checkKdmReady(); }
});
document.getElementById("kdm-cpl-id")?.addEventListener("input", () => checkKdmReady());
document.getElementById("kdm-content-title")?.addEventListener("input", () => checkKdmReady());

function checkKdmReady() {
  const btn = document.getElementById("run-kdm");
  if (btn) btn.disabled = !(
    document.getElementById("kdm-cpl-id")?.value &&
    document.getElementById("kdm-cert")?.value &&
    document.getElementById("kdm-signer-cert")?.value &&
    document.getElementById("kdm-signer-key")?.value &&
    document.getElementById("kdm-content-title")?.value &&
    document.getElementById("kdm-output")?.value
  );
}

document.getElementById("run-kdm")?.addEventListener("click", async () => {
  const cplId = document.getElementById("kdm-cpl-id").value;
  const contentTitle = document.getElementById("kdm-content-title").value;
  const cert = document.getElementById("kdm-cert").value;
  const signerCert = document.getElementById("kdm-signer-cert").value;
  const signerKey = document.getElementById("kdm-signer-key").value;
  const keys = document.getElementById("kdm-keys").value;
  const output = document.getElementById("kdm-output").value;
  const from = document.getElementById("kdm-from").value;
  const to = document.getElementById("kdm-to").value;
  const template = document.getElementById("kdm-template")?.value.trim();
  const format = document.getElementById("kdm-format")?.value || "smpte";
  const formulation = document.getElementById("kdm-formulation")?.value || "";
  const noForensicPicture = document.getElementById("kdm-no-forensic-picture")?.checked || false;
  const audioMarking = document.getElementById("kdm-audio-marking")?.value || "on";
  const audioMarkingChannel = document.getElementById("kdm-audio-marking-channel")?.value;
  const resultsBox = document.getElementById("kdm-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Generating KDM...";
  const args = ["kdm", "--cpl-id", cplId, "--content-title", contentTitle, "--cert", cert,
    "--signer-cert", signerCert, "--signer-key", signerKey, "-o", output, "--format", format];
  if (keys) args.push("--keys", keys);
  if (template) args.push("--template", template);
  if (from) args.push("-f", from);
  if (to) args.push("-t", to);
  if (formulation) args.push("--formulation", formulation);
  if (noForensicPicture) args.push("--disable-forensic-marking-picture");
  // bare disables every channel, a value disables the channels above it
  if (audioMarking === "off") args.push("--disable-forensic-marking-audio");
  if (audioMarking === "above" && audioMarkingChannel) {
    args.push("--disable-forensic-marking-audio", audioMarkingChannel);
  }
  const cmd = Command.sidecar("dcpwizard", args);
  const result = await cmd.execute();
  resultsBox.textContent = result.code === 0
    ? "✓ KDM generated\n\n" + result.stdout
    : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Version File (Supplemental DCP) ===
let vfNextId = 1;
const vfReplacements = [{ id: vfNextId++, reel: 1, picture: "", sound: "" }];

function renderVfReplacements() {
  const list = document.getElementById("vf-repl-list");
  if (!list) return;
  list.innerHTML = vfReplacements.map(r => `
    <div class="field-row vf-repl-row" data-id="${r.id}">
      <div class="prop-field vf-reel-field">
        <label>Reel</label>
        <input type="number" class="vf-reel-num" min="1" value="${r.reel}">
      </div>
      <div class="prop-field">
        <label>Picture (optional)</label>
        <div class="input-with-btn">
          <input type="text" class="vf-picture" value="${r.picture}" placeholder="J2K dir or .mxf…" readonly>
          <button class="btn-sm vf-browse-picture" type="button">…</button>
        </div>
      </div>
      <div class="prop-field">
        <label>Sound (optional)</label>
        <div class="input-with-btn">
          <input type="text" class="vf-sound" value="${r.sound}" placeholder="WAV or .mxf…" readonly>
          <button class="btn-sm vf-browse-sound" type="button">…</button>
        </div>
      </div>
      <button class="btn-sm vf-remove" type="button" title="Remove reel" ${vfReplacements.length <= 1 ? "disabled" : ""}>✕</button>
    </div>
  `).join("");
  checkVfReady();
}

function checkVfReady() {
  const btn = document.getElementById("vf-create");
  if (!btn) return;
  const ok = document.getElementById("vf-ov")?.value &&
    document.getElementById("vf-output")?.value &&
    vfReplacements.some(r => r.picture || r.sound);
  btn.disabled = !ok;
}

document.getElementById("vf-browse-ov")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { document.getElementById("vf-ov").value = d; checkVfReady(); }
});
document.getElementById("vf-browse-output")?.addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) { document.getElementById("vf-output").value = d; checkVfReady(); }
});
document.getElementById("vf-title")?.addEventListener("input", checkVfReady);

document.getElementById("vf-add-repl")?.addEventListener("click", () => {
  const maxReel = vfReplacements.reduce((m, r) => Math.max(m, parseInt(r.reel) || 0), 0);
  vfReplacements.push({ id: vfNextId++, reel: maxReel + 1, picture: "", sound: "" });
  renderVfReplacements();
});

// Per-row picker / remove via delegation (rows are re-rendered).
document.getElementById("vf-repl-list")?.addEventListener("click", async (e) => {
  const row = e.target.closest(".vf-repl-row");
  if (!row) return;
  const rec = vfReplacements.find(r => r.id === parseInt(row.dataset.id));
  if (!rec) return;
  if (e.target.classList.contains("vf-browse-picture")) {
    const d = await open({ directory: true });
    if (d) { rec.picture = d; renderVfReplacements(); }
  } else if (e.target.classList.contains("vf-browse-sound")) {
    const f = await open({ directory: false, filters: [{ name: "Audio/MXF", extensions: ["wav", "mxf"] }, { name: "All", extensions: ["*"] }] });
    if (f) { rec.sound = f; renderVfReplacements(); }
  } else if (e.target.classList.contains("vf-remove")) {
    if (vfReplacements.length <= 1) return;
    vfReplacements.splice(vfReplacements.findIndex(r => r.id === rec.id), 1);
    renderVfReplacements();
  }
});
// Reel number edits shouldn't re-render (would drop focus).
document.getElementById("vf-repl-list")?.addEventListener("input", (e) => {
  if (!e.target.classList.contains("vf-reel-num")) return;
  const row = e.target.closest(".vf-repl-row");
  const rec = vfReplacements.find(r => r.id === parseInt(row.dataset.id));
  if (rec) rec.reel = e.target.value;
});

document.getElementById("vf-create")?.addEventListener("click", async () => {
  const ov = document.getElementById("vf-ov")?.value;
  const output = document.getElementById("vf-output")?.value;
  const title = document.getElementById("vf-title")?.value?.trim();
  if (!ov || !output) { tauriMessage("Select the OV and output directories"); return; }

  const replacements = vfReplacements
    .map(r => ({ reel_number: parseInt(r.reel) || 0, picture: r.picture || null, sound: r.sound || null }))
    .filter(r => r.picture || r.sound);
  if (replacements.length === 0) { tauriMessage("Add at least one reel with a picture or sound"); return; }

  const box = document.getElementById("vf-results");
  box.classList.add("visible");
  box.textContent = "Creating Version File DCP…";
  setStatus("Creating Version File…");
  try {
    const msg = await invoke("create_vf", { ovDir: ov, outputDir: output, title: title || null, replacements });
    box.textContent = "✓ " + msg;
    setStatus("Version File created");
    addRecentProject(output, title || "VF");
  } catch (e) {
    box.textContent = "✗ " + e;
    setStatus("Version File failed: " + e);
  }
});

renderVfReplacements();

// === Jobs ===
let jobsPollInterval = null;

async function refreshJobs() {
  const statusBadge = document.getElementById("jobs-status");
  try {
    const result = await Command.sidecar("dcpwizard", ["batch", "list"]).execute();
    if (result.code !== 0) {
      statusBadge.textContent = "Offline";
      document.getElementById("jobs-tbody").innerHTML =
        '<tr><td colspan="5" style="text-align:center">Daemon not running</td></tr>';
      return;
    }
    statusBadge.textContent = "Online";
    const lines = result.stdout.trim().split("\n");
    const tbody = document.getElementById("jobs-tbody");
    if (lines.length <= 1 || lines[0].startsWith("No jobs")) {
      tbody.innerHTML = '<tr><td colspan="5" style="text-align:center">No jobs</td></tr>';
      return;
    }
    const jobLines = lines.slice(2).filter(l => l.trim());
    tbody.innerHTML = jobLines.map(line => {
      const parts = line.trim().split(/\s+/);
      const [id, state, progress, type] = parts;
      return `<tr><td>${id}</td><td>${type}</td><td>${state}</td><td>${progress}</td>
        <td>${state === "running" || state === "queued" ? `<button class="btn-sm btn-cancel" data-job-id="${id}">✕</button>` : ''}</td></tr>`;
    }).join('');
    tbody.querySelectorAll(".btn-cancel").forEach(btn => {
      btn.addEventListener("click", async () => {
        await Command.sidecar("dcpwizard", ["batch", "cancel", btn.dataset.jobId]).execute();
        refreshJobs();
      });
    });
  } catch {
    statusBadge.textContent = "Error";
  }
}

function startJobsPolling() {
  if (!jobsPollInterval) jobsPollInterval = setInterval(refreshJobs, 3000);
}
function stopJobsPolling() {
  if (jobsPollInterval) { clearInterval(jobsPollInterval); jobsPollInterval = null; }
}

document.getElementById("jobs-refresh")?.addEventListener("click", refreshJobs);

// === Status bar ===
function setStatus(text) {
  const el = document.getElementById("status-text");
  if (el) {
    el.textContent = text;
    el.title = text;
  }
}

let lastValidation = null;

function resetStatusBar() {
  setStatus("");
  lastValidation = null;
  const validEl = document.getElementById("status-validation");
  if (validEl) {
    validEl.textContent = "";
    validEl.title = "";
  }
}

document.getElementById("status-validation")?.addEventListener("click", () => {
  if (!lastValidation) return;
  const errors = lastValidation.errors || [];
  const warnings = lastValidation.warnings || [];
  const text = [
    ...errors.map((e) => `ERROR: ${e}`),
    ...warnings.map((w) => `WARNING: ${w}`),
  ].join("\n\n");
  if (!text) return;
  tauriMessage(text, {
    title: "Validation",
    kind: errors.length ? "error" : "warning",
  });
});

function formatTime(secs) {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return m > 0 ? `${m}m${s}s` : `${s}s`;
}

// === Free disk ===
const DISK_REFRESH_MS = 30000;
const DISK_LOW_PERCENT = 10;

function formatBytes(bytes) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) { value /= 1000; unit++; }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

async function refreshDiskSpace() {
  const el = document.getElementById("status-disk");
  if (!el) return;
  const path = document.getElementById("prop-output")?.value || await documentDir();
  let space;
  try {
    space = await invoke("disk_space", { path });
  } catch {
    el.textContent = "";
    el.title = "";
    return;
  }
  const percent = Math.round(space.percent_free);
  el.textContent = `💾 ${percent}%`;
  el.title = `${formatBytes(space.free_bytes)} free of ${formatBytes(space.total_bytes)} on ${path}`;
  el.style.color = percent <= DISK_LOW_PERCENT ? "#ff6b6b" : "";
}

refreshDiskSpace();
setInterval(refreshDiskSpace, DISK_REFRESH_MS);

// === Title sync ===
document.getElementById("prop-title")?.addEventListener("input", (e) => {
  const title = e.target.value.trim();
  document.getElementById("project-name").textContent = title || "Untitled Project";
  project.title = title;
});

// === Tools: Encode J2K ===
document.getElementById("enc-browse-input")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("enc-input").value = dir; checkEncodeReady(); }
});
document.getElementById("enc-browse-output")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("enc-output").value = dir; checkEncodeReady(); }
});

function checkEncodeReady() {
  const btn = document.getElementById("run-encode");
  if (btn) btn.disabled = !(document.getElementById("enc-input")?.value && document.getElementById("enc-output")?.value);
}

document.getElementById("run-encode")?.addEventListener("click", async () => {
  const input = document.getElementById("enc-input").value;
  const output = document.getElementById("enc-output").value;
  const resolution = document.getElementById("enc-resolution").value;
  const bandwidth = document.getElementById("enc-bandwidth").value;
  const framerate = document.getElementById("enc-framerate").value;
  const resultsBox = document.getElementById("encode-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Encoding...";
  const args = ["encode", "-i", input, "-o", output, "--bandwidth", bandwidth];
  const cmd = Command.sidecar("dcpwizard", args);
  const result = await cmd.execute();
  resultsBox.textContent = result.code === 0
    ? "✓ Encode complete\n\n" + result.stdout
    : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: Transcode ===
document.getElementById("tc-browse-input")?.addEventListener("click", async () => {
  const file = await open({ directory: false, filters: [{ name: 'Video', extensions: ['mp4','mkv','mov','avi','mxf','webm'] }, { name: 'All', extensions: ['*'] }] });
  if (file) { document.getElementById("tc-input").value = file; checkTranscodeReady(); }
});
document.getElementById("tc-browse-output")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("tc-output").value = dir; checkTranscodeReady(); }
});

function checkTranscodeReady() {
  const btn = document.getElementById("tc-start");
  if (btn) btn.disabled = !(document.getElementById("tc-input")?.value && document.getElementById("tc-output")?.value);
}

document.getElementById("tc-start")?.addEventListener("click", async () => {
  const input = document.getElementById("tc-input").value;
  const output = document.getElementById("tc-output").value;
  const format = document.getElementById("tc-format").value;
  const bitdepth = document.getElementById("tc-bitdepth").value;
  const resultsBox = document.getElementById("tc-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Transcoding...";
  const args = ["transcode", "-i", input, "-o", output, "--format", format, "--bit-depth", bitdepth];
  const cmd = Command.sidecar("dcpwizard", args);
  const result = await cmd.execute();
  resultsBox.textContent = result.code === 0
    ? "✓ Transcode complete\n\n" + result.stdout
    : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: Loudness ===
document.getElementById("loud-browse")?.addEventListener("click", async () => {
  const file = await open({ directory: false, filters: [{ name: 'Audio', extensions: ['wav','aiff','flac','mp3'] }, { name: 'All', extensions: ['*'] }] });
  if (file) { document.getElementById("loud-input").value = file; document.getElementById("loud-measure").disabled = false; }
});

document.getElementById("loud-measure")?.addEventListener("click", async () => {
  const input = document.getElementById("loud-input").value;
  const resultsBox = document.getElementById("loud-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Measuring loudness...";
  const cmd = Command.sidecar("dcpwizard", ["loudness", input]);
  const result = await cmd.execute();
  resultsBox.textContent = result.code === 0
    ? "✓ Loudness measured\n\n" + result.stdout
    : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: Copy DCP ===
document.getElementById("copy-browse-source")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("copy-source").value = dir; checkCopyReady(); }
});
document.getElementById("copy-browse-dest")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("copy-dest").value = dir; checkCopyReady(); }
});

function checkCopyReady() {
  const btn = document.getElementById("copy-start");
  if (btn) btn.disabled = !(document.getElementById("copy-source")?.value && document.getElementById("copy-dest")?.value);
}

document.getElementById("copy-start")?.addEventListener("click", async () => {
  const source = document.getElementById("copy-source").value;
  const dest = document.getElementById("copy-dest").value;
  const resultsBox = document.getElementById("copy-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Copying...";
  const args = ["copy", "--src", source, "--dst", dest];
  const cmd = Command.sidecar("dcpwizard", args);
  const result = await cmd.execute();
  resultsBox.textContent = result.code === 0
    ? "✓ Copy complete\n\n" + result.stdout
    : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Tools: QC Report ===
document.getElementById("report-browse")?.addEventListener("click", async () => {
  const dir = await open({ directory: true });
  if (dir) { document.getElementById("report-dcp").value = dir; document.getElementById("report-start").disabled = false; }
});

document.getElementById("report-start")?.addEventListener("click", async () => {
  const dcp = document.getElementById("report-dcp").value;
  const format = document.getElementById("report-format").value;
  const resultsBox = document.getElementById("report-results");
  resultsBox.classList.add("visible");
  resultsBox.textContent = "Generating report...";
  const args = ["report", "--dcp", dcp, "-o", dcp + "/report." + format];
  const cmd = Command.sidecar("dcpwizard", args);
  const result = await cmd.execute();
  resultsBox.textContent = result.code === 0
    ? result.stdout
    : "✗ Failed\n\n" + (result.stderr || result.stdout);
});

// === Recent Projects ===
const RECENT_KEY = "dcpwizard-recent-projects";
const MAX_RECENT = 20;

function getRecentProjects() {
  try { return JSON.parse(localStorage.getItem(RECENT_KEY)) || []; }
  catch { return []; }
}

function addRecentProject(path, title) {
  let recent = getRecentProjects().filter(r => r.path !== path);
  recent.unshift({ path, title, time: Date.now() });
  if (recent.length > MAX_RECENT) recent = recent.slice(0, MAX_RECENT);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  renderRecentProjects();
}

function removeRecentProject(path) {
  const recent = getRecentProjects().filter(r => r.path !== path);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  renderRecentProjects();
}

function renderRecentProjects() {
  const section = document.getElementById("recent-projects");
  const list = document.getElementById("recent-list");
  if (!section || !list) return;
  const recent = getRecentProjects();
  if (recent.length === 0) { section.hidden = true; return; }
  section.hidden = false;
  list.innerHTML = recent.map(r => `
    <div class="recent-item" data-path="${r.path}" title="${r.path}">
      <div class="recent-item-text">
        <span class="recent-title">${r.title || r.path.split(/[/\\]/).pop()}</span>
        <span class="recent-path">${r.path}</span>
      </div>
      <button class="recent-retitle" data-path="${r.path}" title="Give this DCP a new content title">✎</button>
      <button class="recent-delete" data-path="${r.path}" title="Delete this DCP from disk">✕</button>
    </div>
  `).join('');
  list.querySelectorAll('.recent-retitle').forEach(el => {
    el.addEventListener('click', async (event) => {
      event.stopPropagation();
      const dir = el.dataset.path;
      const title = prompt("New content title:", dir.split(/[/\\]/).pop());
      if (!title?.trim()) return;
      const ok = await tauriConfirm(
        `Retitle to ${title}? The CPL gets a new composition id, so any KDM or delivery made from the old one no longer matches. A signed package loses its signature.`,
        { title: "Retitle DCP", kind: "warning" },
      );
      if (!ok) return;
      let newPath;
      try {
        newPath = await invoke("retitle_dcp", { path: dir, title });
      } catch (e) {
        tauriMessage(String(e), { title: "Retitle failed", kind: "error" });
        return;
      }
      removeRecentProject(dir);
      addRecentProject(newPath, title.trim());
      setStatus(`Retitled to ${title.trim()}`);
    });
  });
  list.querySelectorAll('.recent-delete').forEach(el => {
    el.addEventListener('click', async (event) => {
      event.stopPropagation();
      const dir = el.dataset.path;
      const ok = await tauriConfirm(`Delete ${dir} and everything in it?`, {
        title: "Delete DCP",
        kind: "warning",
      });
      if (!ok) return;
      try {
        await invoke("delete_dcp", { path: dir });
      } catch (e) {
        tauriMessage(String(e), { title: "Delete failed", kind: "error" });
        return;
      }
      removeRecentProject(dir);
      setStatus(`Deleted ${dir}`);
      refreshDiskSpace();
    });
  });
  list.querySelectorAll('.recent-item').forEach(el => {
    el.addEventListener('click', () => {
      const dir = el.dataset.path;
      const name = dir.split(/[/\\]/).pop();
      document.getElementById("project-name").textContent = name;
      project.title = name;
      document.getElementById("prop-title").value = name;
      setStatus(`Opened: ${dir}`);
      previewDcp(dir);
    });
  });
}

// === Desktop Notifications ===
function notifyBuildComplete(success, title) {
  if (Notification.permission === "granted") {
    new Notification(success ? "Build Complete" : "Build Failed", {
      body: success ? `"${title}" built successfully` : `"${title}" build failed`,
      icon: success ? undefined : undefined,
    });
  } else if (Notification.permission !== "denied") {
    Notification.requestPermission();
  }
}

// Request notification permission early
if ("Notification" in window && Notification.permission === "default") {
  Notification.requestPermission();
}

// === Confirmation Dialogs ===
document.getElementById("btn-new-project")?.addEventListener("click", async () => {
  if (project.assets.length > 0) {
    if (!(await tauriConfirm("Clear current project and start new? Unsaved changes will be lost."))) return;
  }
  project.title = "";
  project.assets = [];
  project.reels = [{ id: 1, picture: null, sound: null, subtitle: null }];
  nextAssetId = 1;
  const titleEl = document.getElementById("prop-title");
  if (titleEl) titleEl.value = "";
  document.getElementById("prop-output") && (document.getElementById("prop-output").value = "");
  document.getElementById("project-name").textContent = "Untitled Project";
  switchView("project");
  renderAssets();
  renderReels();
  updateStatusStats();
  setStatus("New project — enter a title to get started");
  if (titleEl) { titleEl.focus(); titleEl.select(); }
});

// === Status Bar Stats ===
function updateStatusStats() {
  const el = document.getElementById("status-stats");
  if (!el) return;
  const n = project.assets.length;
  const v = project.assets.filter(a => a.type === 'video').length;
  const a = project.assets.filter(a => a.type === 'audio').length;
  if (n === 0) { el.textContent = ""; } else {
    const parts = [];
    if (v) parts.push(`${v} video`);
    if (a) parts.push(`${a} audio`);
    const s = project.assets.filter(a => a.type === 'subtitle').length;
    if (s) parts.push(`${s} sub`);
    el.textContent = `${n} assets (${parts.join(', ')})`;
  }
  updateToolbarState();
}

// === Toolbar Button State ===
let buildInFlight = false;

function beginBuild() {
  buildInFlight = true;
  updateToolbarState();
}

function endBuild() {
  buildInFlight = false;
  updateToolbarState();
  refreshDiskSpace();
}

function updateToolbarState() {
  const hasVideo = project.reels.some(r => r.picture);
  const hasTitle = !!(document.getElementById("prop-title")?.value?.trim());
  const buildBtn = document.getElementById("btn-build");
  const previewBtn = document.getElementById("btn-preview");
  if (buildBtn) buildBtn.disabled = buildInFlight || !(hasVideo && hasTitle);
  if (previewBtn) previewBtn.disabled = !hasVideo && !document.getElementById("prop-output")?.value;
}

// Keep title in sync and update toolbar state
const _origTitleHandler = document.getElementById("prop-title");
_origTitleHandler?.addEventListener("input", () => { updateToolbarState(); });

// === Context Menu ===
const ctxMenu = document.getElementById("context-menu");
let ctxAssetId = null;

function showContextMenu(e, assetId) {
  e.preventDefault();
  ctxAssetId = assetId;
  ctxMenu.style.left = e.clientX + "px";
  ctxMenu.style.top = e.clientY + "px";
  ctxMenu.hidden = false;
}

document.addEventListener("click", () => { if (ctxMenu) ctxMenu.hidden = true; });

async function removeAsset(assetId) {
  const asset = project.assets.find(a => a.id === assetId);
  if (!asset) return;
  if (!(await tauriConfirm(`Remove "${asset.name}" from project?`))) return;
  project.assets = project.assets.filter(a => a.id !== assetId);
  project.reels.forEach(r => {
    if (r.picture?.id === assetId) r.picture = null;
    if (r.sound?.id === assetId) r.sound = null;
    if (r.subtitle?.id === assetId) r.subtitle = null;
  });
  renderAssets();
  renderReels();
  updateStatusStats();
}

ctxMenu?.querySelectorAll("button").forEach(btn => {
  btn.addEventListener("click", () => {
    const action = btn.dataset.action;
    const asset = project.assets.find(a => a.id === ctxAssetId);
    if (!asset) return;
    if (action === "preview") {
      previewFile(asset.path);
    } else if (action === "remove") {
      removeAsset(ctxAssetId);
    } else if (action === "reveal") {
      // reveals the file in the OS file manager (shell open only accepts URLs)
      revealItemInDir(asset.path);
    }
    ctxMenu.hidden = true;
  });
});

// === Progress in Title Bar ===
function setTitleProgress(percent, stage) {
  if (percent >= 0 && percent < 100) {
    document.title = `DCP Wizard — ${stage} ${Math.round(percent)}%`;
  } else {
    document.title = "DCP Wizard";
  }
}

// === Asset Filter ===
document.getElementById("asset-filter")?.addEventListener("input", (e) => {
  const q = e.target.value.toLowerCase();
  document.querySelectorAll("#asset-list .asset-item").forEach(el => {
    const name = el.querySelector(".asset-name")?.textContent?.toLowerCase() || "";
    el.style.display = name.includes(q) ? "" : "none";
  });
});

// === Auto-detect Video Properties (ffprobe) ===
async function probeVideo(path) {
  try {
    const cmd = Command.create("ffprobe", [
      "-v", "quiet", "-print_format", "json",
      "-show_streams", "-show_format", path
    ]);
    const result = await cmd.execute();
    if (result.code !== 0) return null;
    const info = JSON.parse(result.stdout);
    const vs = info.streams?.find(s => s.codec_type === "video");
    if (!vs) return null;
    return {
      width: vs.width,
      height: vs.height,
      fps: vs.r_frame_rate,
      duration: parseFloat(info.format?.duration || vs.duration || "0"),
    };
  } catch { return null; }
}

// === Init ===
renderAssets();
renderReels();
renderRecentProjects();
updateStatusStats();
initPreview();
initTimeline();
setStatus("Ready");

// === SRT → SMPTE Subtitle Conversion ===
document.getElementById("srt-browse-input")?.addEventListener("click", async () => {
  const path = await open({ filters: [{ name: "SRT", extensions: ["srt"] }] });
  if (path) {
    document.getElementById("srt-input").value = path;
    document.getElementById("srt-convert").disabled = false;
  }
});
document.getElementById("srt-browse-output")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) document.getElementById("srt-output").value = path;
});
document.getElementById("srt-convert")?.addEventListener("click", async () => {
  const input = document.getElementById("srt-input").value;
  const output = document.getElementById("srt-output").value;
  const lang = document.getElementById("srt-language").value || "en";
  const fps = document.getElementById("srt-framerate").value || String(DEFAULT_FRAMERATE);
  const vposition = document.getElementById("srt-vposition").value || "8";
  if (!input) return;

  const resultsEl = document.getElementById("srt-results");
  resultsEl.textContent = "Converting…";
  resultsEl.classList.add("visible");

  try {
    const args = ["subtitle-convert", "-i", input, "-l", lang, "--fps", fps, "--vposition", vposition];
    if (output) args.push("-o", output);
    const cmd = Command.sidecar("dcpwizard", args);
    const result = await cmd.execute();
    resultsEl.textContent = result.code === 0
      ? `✓ Conversion complete\n${result.stdout}`
      : `✗ Error:\n${result.stderr || result.stdout}`;
  } catch (e) {
    resultsEl.textContent = `✗ Failed: ${e}`;
  }
});

// === Subtitle Extraction ===
document.getElementById("subextract-browse-dir")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) {
    document.getElementById("subextract-input").value = path;
    document.getElementById("subextract-start").disabled = false;
  }
});
document.getElementById("subextract-browse-file")?.addEventListener("click", async () => {
  const path = await open({ filters: [{ name: "Subtitle", extensions: ["xml", "mxf"] }] });
  if (path) {
    document.getElementById("subextract-input").value = path;
    document.getElementById("subextract-start").disabled = false;
  }
});
document.getElementById("subextract-browse-output")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) document.getElementById("subextract-output").value = path;
});
document.getElementById("subextract-start")?.addEventListener("click", async () => {
  const input = document.getElementById("subextract-input").value;
  const format = document.getElementById("subextract-format").value;
  let outDir = document.getElementById("subextract-output").value;
  if (!input) return;
  // default the output beside the input when none is chosen
  if (!outDir) outDir = input.replace(/[\\/][^\\/]*$/, "") || input;

  const resultsEl = document.getElementById("subextract-results");
  resultsEl.textContent = "Extracting…";
  resultsEl.classList.add("visible");

  try {
    const output = outDir + "/subtitles." + format;
    const args = ["subtitle-extract", "-i", input, "-o", output];
    const cmd = Command.sidecar("dcpwizard", args);
    const result = await cmd.execute();
    resultsEl.textContent = result.code === 0
      ? `✓ Extracted to ${output}\n${result.stdout}`
      : `✗ Error:\n${result.stderr || result.stdout}`;
  } catch (e) {
    resultsEl.textContent = `✗ Failed: ${e}`;
  }
});

// === Subtitle Burn-in ===
document.getElementById("burnin-browse-video")?.addEventListener("click", async () => {
  const path = await open({ filters: [{ name: "Video", extensions: ["mp4", "mkv", "mov", "mxf"] }] });
  if (path) {
    document.getElementById("burnin-video").value = path;
    updateBurninBtn();
  }
});
document.getElementById("burnin-browse-sub")?.addEventListener("click", async () => {
  const path = await open({ filters: [{ name: "Subtitle", extensions: ["srt", "xml", "ttml"] }] });
  if (path) {
    document.getElementById("burnin-sub").value = path;
    updateBurninBtn();
  }
});
document.getElementById("burnin-browse-output")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) document.getElementById("burnin-output").value = path;
});
function updateBurninBtn() {
  const v = document.getElementById("burnin-video").value;
  const s = document.getElementById("burnin-sub").value;
  document.getElementById("burnin-start").disabled = !(v && s);
}
document.getElementById("burnin-start")?.addEventListener("click", async () => {
  const video = document.getElementById("burnin-video").value;
  const sub = document.getElementById("burnin-sub").value;
  const output = document.getElementById("burnin-output").value;
  if (!video || !sub) return;

  const resultsEl = document.getElementById("burnin-results");
  resultsEl.textContent = "Burning in subtitles…";
  resultsEl.classList.add("visible");

  try {
    const args = ["burnin", "-i", video, "-s", sub];
    if (output) args.push("-o", output);
    const cmd = Command.sidecar("dcpwizard", args);
    const result = await cmd.execute();
    resultsEl.textContent = result.code === 0
      ? `✓ Burn-in complete\n${result.stdout}`
      : `✗ Error:\n${result.stderr || result.stdout}`;
  } catch (e) {
    resultsEl.textContent = `✗ Failed: ${e}`;
  }
});

// === Target Conversion (Scale/Crop/Letterbox) ===
document.getElementById("convert-browse-input")?.addEventListener("click", async () => {
  const path = await open({ filters: [
    { name: "Video", extensions: ["mp4", "mkv", "mov", "mxf"] },
    { name: "All", extensions: ["*"] }
  ]});
  if (path) {
    document.getElementById("convert-input").value = path;
    document.getElementById("convert-start").disabled = false;
  }
});
document.getElementById("convert-browse-output")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) document.getElementById("convert-output").value = path;
});
document.getElementById("convert-start")?.addEventListener("click", async () => {
  const input = document.getElementById("convert-input").value;
  const container = document.getElementById("convert-container").value;
  const method = document.getElementById("convert-method").value;
  const output = document.getElementById("convert-output").value;
  if (!input) return;

  const resultsEl = document.getElementById("convert-results");
  resultsEl.textContent = "Converting…";
  resultsEl.classList.add("visible");

  try {
    const args = ["convert", "-i", input, "-t", container, "-m", method];
    if (output) args.push("-o", output);
    const cmd = Command.sidecar("dcpwizard", args);
    const result = await cmd.execute();
    resultsEl.textContent = result.code === 0
      ? `✓ Conversion complete\n${result.stdout}`
      : `✗ Error:\n${result.stderr || result.stdout}`;
  } catch (e) {
    resultsEl.textContent = `✗ Failed: ${e}`;
  }
});

// === Re-ingest Package (rebuild ASSETMAP/PKL) ===
document.getElementById("ingest-browse-dir")?.addEventListener("click", async () => {
  const path = await open({ directory: true });
  if (path) {
    document.getElementById("ingest-dir").value = path;
    document.getElementById("ingest-start").disabled = false;
  }
});
document.getElementById("ingest-start")?.addEventListener("click", async () => {
  const dir = document.getElementById("ingest-dir").value;
  if (!dir) return;

  const resultsEl = document.getElementById("ingest-results");
  resultsEl.textContent = "Rebuilding ASSETMAP and PKL…";
  resultsEl.classList.add("visible");

  try {
    const cmd = Command.sidecar("dcpwizard", ["ingest-package", dir]);
    const result = await cmd.execute();
    resultsEl.textContent = result.code === 0
      ? `✓ Repackaged\n${result.stdout}`
      : `✗ Error:\n${result.stderr || result.stdout}`;
  } catch (e) {
    resultsEl.textContent = `✗ Failed: ${e}`;
  }
});
