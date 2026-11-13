// @ts-check
/**
 * Deterministic fixtures for the mock backend: 18 opportunities that match the
 * Option B wireframe copy, drives C: and D:, the plan and the history. Shapes
 * are exactly the docs/IPC.md types.
 */

export const GB = 1024 ** 3;
export const MB = 1024 ** 2;

const ACTION_TRASH = { kind: 'trash' };
const ARCHIVE_ACTION = { kind: 'archive', format: '7z', level: 5, originals: { policy: 'recycle_bin' }, keep_sources: false };

/** The nine cards of the first page, in wireframe order, then nine smaller ones. */
const SPECS = [
  { id: 'det:exact_duplicates', src: { detector: 'exact_duplicates' }, cat: 'clean', title: 'Exact duplicates', desc: '214 files in 61 groups · keeps newest', safety: 'reversible', gb: 12.4, ratio: 1, count: 214, unit: 'files', in_plan: true, conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-01'] },
  { id: 'opt:fs_compress', src: { optimiser: 'fs_compress' }, cat: 'optimise', title: 'Transparent compression', desc: 'NTFS LZX on cold folders, files stay usable', safety: 'reversible', gb: 14, ratio: 0.45, count: 9, unit: 'folders', action: { kind: 'fs_compress', algorithm: 'lzx' }, actions: ['fs_compress'], feature: ['OPT-FS-01'] },
  { id: 'det:dev_artefacts', src: { detector: 'dev_artefacts' }, cat: 'clean', title: 'Developer artefacts', desc: 'node_modules ×14 · .venv ×3 · target/ ×2', safety: 'rebuildable', gb: 9.6, ratio: 1, count: 19, unit: 'folders', in_plan: true, conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-08'] },
  { id: 'det:app_caches', src: { detector: 'app_caches' }, cat: 'clean', title: 'App caches & junk', desc: 'Browsers, Teams, crash dumps, logs', safety: 'reversible', gb: 6.9, ratio: 1, count: 38, unit: 'apps', in_plan: true, conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-09'] },
  { id: 'opt:video', src: { optimiser: 'video' }, cat: 'optimise', title: 'Screen recordings', desc: 'Low-motion video → 10 fps AV1, text stays sharp', safety: 'lossy_opt_in', gb: 6.1, ratio: 0.8, count: 31, unit: 'videos', action: { kind: 'optimise', optimiser: 'video', params: {}, output: { mode: 'replace' }, originals: { policy: 'recycle_bin' }, keep_if_not_smaller: true }, actions: ['optimise'], feature: ['OPT-VID-01'] },
  { id: 'det:inactive_engagements', src: { detector: 'inactive_folders' }, cat: 'archive', title: 'Inactive engagements', desc: '7 client folders idle 18+ months → 7z archive', safety: 'reversible', gb: 5.5, ratio: 0.7, count: 7, unit: 'folders', action: ARCHIVE_ACTION, actions: ['archive'], feature: ['ARC-01'] },
  { id: 'opt:audio', src: { optimiser: 'audio' }, cat: 'optimise', title: 'Meeting recordings', desc: 'Speech preset, silences trimmed', safety: 'lossy_opt_in', gb: 4.8, ratio: 0.875, count: 23, unit: 'files', in_plan: true, action: { kind: 'optimise', optimiser: 'audio', params: {}, output: { mode: 'replace' }, originals: { policy: 'recycle_bin' }, keep_if_not_smaller: true }, actions: ['optimise'], feature: ['OPT-AUD-01', 'OPT-AUD-04'] },
  { id: 'opt:office', src: { optimiser: 'office' }, cat: 'optimise', title: 'Office decks & workbooks', desc: 'Lossless first, light copies kept beside originals', safety: 'light_copy', gb: 3.6, ratio: 0.4, count: 58, unit: 'files', action: { kind: 'optimise', optimiser: 'office', params: {}, output: { mode: 'light_copy', suffix: '_light' }, originals: { policy: 'recycle_bin' }, keep_if_not_smaller: true }, actions: ['optimise'], feature: ['OPT-OFF-01'] },
  { id: 'det:old_installers', src: { detector: 'old_installers' }, cat: 'clean', title: 'Old installers', desc: '.msi .exe .iso in Downloads, older than 6 months', safety: 'reversible', gb: 3.4, ratio: 1, count: 42, unit: 'files', conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-06'] },
  // ---- the nine smaller ones (1.9 GB together) ----
  { id: 'det:temp_files', src: { detector: 'temp_files' }, cat: 'clean', title: 'Temporary files', desc: 'Leftover temporary and partial downloads', safety: 'reversible', gb: 0.4, ratio: 1, count: 212, unit: 'files', conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-10'] },
  { id: 'det:thumbnail_caches', src: { detector: 'thumbnail_caches' }, cat: 'clean', title: 'Thumbnail caches', desc: 'Explorer and photo app thumbnails, rebuilt on demand', safety: 'reversible', gb: 0.3, ratio: 1, count: 4, unit: 'folders', conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-11'] },
  { id: 'det:old_logs', src: { detector: 'old_logs' }, cat: 'clean', title: 'Old log files', desc: '.log files not touched for 90 days', safety: 'reversible', gb: 0.2, ratio: 1, count: 87, unit: 'files', conf: 'exact', actions: ['trash', 'quarantine', 'move', 'permanent_delete'], feature: ['CLN-DET-12'] },
  { id: 'det:zero_byte', src: { detector: 'zero_byte' }, cat: 'clean', title: 'Empty files', desc: 'Files with no content', safety: 'reversible', gb: 0.01, ratio: 1, count: 31, unit: 'files', conf: 'exact', actions: ['trash', 'quarantine'], feature: ['CLN-DET-13'] },
  { id: 'opt:images', src: { optimiser: 'images' }, cat: 'optimise', title: 'Photos & screenshots', desc: 'Lossless recompression, metadata kept', safety: 'reversible', gb: 0.3, ratio: 0.3, count: 340, unit: 'files', action: { kind: 'optimise', optimiser: 'images', params: {}, output: { mode: 'replace' }, originals: { policy: 'recycle_bin' }, keep_if_not_smaller: true }, actions: ['optimise'], feature: ['OPT-IMG-01'] },
  { id: 'opt:pdf', src: { optimiser: 'pdf' }, cat: 'optimise', title: 'PDF documents', desc: 'Lossless structure clean-up', safety: 'reversible', gb: 0.2, ratio: 0.25, count: 66, unit: 'files', action: { kind: 'optimise', optimiser: 'pdf', params: {}, output: { mode: 'replace' }, originals: { policy: 'recycle_bin' }, keep_if_not_smaller: true }, actions: ['optimise'], feature: ['OPT-PDF-01'] },
  { id: 'det:large_old', src: { detector: 'large_old_files' }, cat: 'clean', title: 'Large old files', desc: 'Over 500 MB and untouched for 2 years', safety: 'review_only', gb: 0.2, ratio: 1, count: 6, unit: 'files', actions: ['none'], feature: ['CLN-DET-20'] },
  { id: 'det:similar_images', src: { detector: 'similar_images' }, cat: 'clean', title: 'Similar photos', desc: 'Near-identical shots, you pick the best', safety: 'review_only', gb: 0.15, ratio: 1, count: 48, unit: 'files', actions: ['none'], feature: ['CLN-DET-04'] },
  { id: 'det:sensitive', src: { detector: 'sensitive' }, cat: 'clean', title: 'Sensitive documents', desc: 'Passport scans and tax files in loose folders', safety: 'review_only', gb: 0.14, ratio: 1, count: 12, unit: 'files', actions: ['none'], feature: ['CLN-DET-21'] },
];

/** Meeting-recordings presets and settings (docs/IPC.md Opportunity example). */
const AUDIO_PRESETS = [
  { id: 'speech', label: 'Speech', description: 'Opus 24 kbps mono · smallest', values: { codec: 'opus', bitrate_kbps: 24, downmix_mono: true }, lossy: true },
  { id: 'transcription_safe', label: 'Transcription-safe', description: 'Opus 48 kbps · keeps detail for speech-to-text', values: { codec: 'opus', bitrate_kbps: 48 }, lossy: true },
  { id: 'lossless', label: 'Lossless', description: 'FLAC · no quality loss, ~45 % smaller', values: { codec: 'flac' }, lossy: false },
];
const AUDIO_SETTINGS = [
  { key: 'downmix_mono', label: 'Downmix to mono', kind: { type: 'toggle', default: true } },
  { key: 'remove_silences', label: 'Remove long silences', kind: { type: 'toggle', default: true }, lossy: true },
  { key: 'max_pause_s', label: 'Max pause', kind: { type: 'number', default: 2, min: 1, max: 30, step: 1, unit: 's max' } },
  { key: 'strip_cover_art', label: 'Strip cover art', kind: { type: 'toggle', default: true } },
  { key: 'transcribe', label: 'Transcribe on this PC, then archive audio', help: 'Keeps a searchable text file next to each meeting', kind: { type: 'toggle', default: false } },
  { key: 'normalise_loudness', label: 'Even out loudness', kind: { type: 'toggle', default: false }, advanced: true },
  { key: 'keep_dates', label: 'Keep original file dates', kind: { type: 'toggle', default: true }, advanced: true },
];
const AUDIO_SUBGROUPS = [
  { key: 'same_meeting', label: 'Same meeting twice', count: 6 },
  { key: 'needs_transcript', label: 'Need transcript', count: 4 },
  { key: 'skipped', label: 'Skipped', count: 2 },
];

/**
 * Build the opportunity list (full Opportunity shape) in rank order.
 * `size_before`, `saving` and `selected` are recomputed from the candidates by
 * the backend once rows exist; the values here are the starting point.
 */
export function buildOpportunities() {
  return SPECS.map((s, i) => {
    const before = Math.round(s.gb * GB);
    const saving = Math.round(before * s.ratio);
    const lossyAudio = s.id === 'opt:audio';
    return {
      id: s.id,
      source: s.src,
      category: s.cat,
      title: s.title,
      description: s.desc,
      safety: s.safety,
      size_before: before,
      saving,
      confidence: s.conf ?? 'estimated',
      count: s.count,
      unit: s.unit,
      candidates: s.count,
      selected: s.count,
      badges: [],
      default_action: s.action ?? (s.safety === 'review_only' ? { kind: 'none' } : ACTION_TRASH),
      available_actions: s.actions,
      presets: lossyAudio ? AUDIO_PRESETS : [],
      settings: lossyAudio ? AUDIO_SETTINGS : [],
      params: lossyAudio ? { downmix_mono: true, remove_silences: true, max_pause_s: 2, strip_cover_art: true, transcribe: false } : s.id === 'det:exact_duplicates' ? { keep_rule: 'newest' } : {},
      sub_groups: lossyAudio ? AUDIO_SUBGROUPS : [],
      availability: s.safety === 'review_only' ? { state: 'available' } : { state: 'available' },
      in_plan: !!s.in_plan,
      rank: 1e12 - i * 1e9,
      feature_ids: s.feature,
    };
  });
}

/** Plan step estimates of the wireframe (8.6 + 9.6 + 6.9 + 4.2 = 29.3 GB). */
export const PLAN_BASE = {
  'det:exact_duplicates': { before: 9234179686, after: 0, summary: 'Keep newest · others to Recycle Bin', params: { keep_rule: 'newest' }, files: 153 },
  'det:dev_artefacts': { before: 10307921510, after: 0, summary: 'To Recycle Bin · rebuild with npm / pip', params: {}, files: 19 },
  'det:app_caches': { before: 7408802365, after: 0, summary: 'Apps recreate these · quarantine 7 days', params: {}, files: 38, action: { kind: 'quarantine', purge_after_days: 7 } },
  'opt:audio': { before: 5153960755, after: 644245094, summary: 'Opus 24 kbps mono · originals to Recycle Bin', params: { codec: 'opus', bitrate_kbps: 24 }, files: 23 },
};

/** Summary of the hero (fixed numbers of the wireframe: 41.2 + 5.1 + 2.3 = 48.6 GB). */
export const HERO_SUMMARY = { total: 52183474995, reversible: 44238230323, opt_in: 5476083302, review_only: 2469606195, opportunities: 18 };

/** Volumes. `plan_saving` is for the full wireframe plan; the backend scales it. */
export const DRIVES = [
  { id: 'C:', mount: 'C:\\', label: 'System', fs_type: 'NTFS', total: 511101108224, free: 45 * GB, available: 45 * GB, removable: false, features: { hardlinks: true, reflinks: false, wof: true, btrfs_compression: false, trash: true, mft: false }, plan_saving: 41 * GB },
  { id: 'D:', mount: 'D:\\', label: 'Projects', fs_type: 'NTFS', total: 931 * GB, free: 354 * GB, available: 354 * GB, removable: false, features: { hardlinks: true, reflinks: false, wof: true, btrfs_compression: false, trash: true, mft: false }, plan_saving: 8 * GB },
];

export const TOOLS = [
  { id: 'ffmpeg', found: true, path: 'C:\\Tools\\ffmpeg\\bin\\ffmpeg.exe', version: '7.0.1', used_for: ['Video', 'Screen recordings', 'Meeting recordings'] },
  { id: '7z', found: true, path: 'C:\\Program Files\\7-Zip\\7z.exe', version: '24.07', used_for: ['Archive'] },
  { id: 'qpdf', found: false, used_for: ['PDF documents'] },
];

export const SETTINGS = {
  version: 1,
  general: { undo_days: 30, quarantine_days: 30, default_originals: { policy: 'recycle_bin' }, light_copy_suffix: '_light' },
  scan: { include_hidden: true, use_fast_index: true, use_cache: true, one_file_system: true, max_depth: null, threads: 0, collect_owner: false },
  safety: { protected_paths: [], exclusions: [], skip_signed: true, skip_labelled: true, skip_encrypted: true, synced_folder_warning: true },
  optimise: { lossless_only: true, keep_if_not_smaller: true, preserve_timestamps: true, preserve_permissions: true, preserve_ads: true },
  performance: { threads: 0, parallel_jobs: 0, low_memory: false, cpu_priority: 'normal' },
  tool_paths: {},
  integration: { context_menu: false, fs_compression_watcher: false, watched_folders: [] },
  type_groups: { installers: ['msi', 'exe', 'iso'] },
};

/** Folder tree for the in-app browser (Windows-style paths). */
export const FS = {
  places: [
    { label: 'Home', path: 'C:\\Users\\Michel', kind: 'home' },
    { label: 'Desktop', path: 'C:\\Users\\Michel\\Desktop', kind: 'desktop' },
    { label: 'Documents', path: 'C:\\Users\\Michel\\Documents', kind: 'documents' },
    { label: 'Downloads', path: 'C:\\Users\\Michel\\Downloads', kind: 'downloads' },
    { label: 'Recordings', path: 'C:\\Users\\Michel\\Recordings', kind: 'videos' },
    { label: 'System (C:)', path: 'C:\\', kind: 'drive' },
    { label: 'Projects (D:)', path: 'D:\\', kind: 'drive' },
  ],
  dirs: {
    'C:\\': ['Users', 'Program Files', 'Windows'],
    'C:\\Users': ['Michel'],
    'C:\\Users\\Michel': ['Desktop', 'Documents', 'Downloads', 'Pictures', 'Recordings', 'Projects'],
    'D:\\': ['Projects', 'Archive', 'Clients'],
    'D:\\Projects': ['app', 'website', 'notes'],
  },
};
