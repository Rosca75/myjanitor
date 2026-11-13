// @ts-check
/**
 * Platform awareness: which features need which platform or volume feature.
 * The backend decides availability (and gives the reason); this only turns the
 * facts from app_info, drives_list and tools_list into the short label a card
 * shows, so a feature that cannot run here is labelled instead of hidden.
 */

/** Requirement of an opportunity id: 'windows' | 'btrfs' | 'reflink' | a tool id. @type {Record<string, string>} */
const NEEDS = {
  'opt:fs_compress': 'compression',
  'opt:video': 'ffmpeg',
  'opt:audio': 'ffmpeg',
  'opt:btrfs_compress': 'btrfs',
  'opt:dehydrate': 'windows',
  'det:reflink_dedupe': 'reflink',
};

/**
 * Label for a feature this machine cannot run, or null when nothing is missing.
 * @param {string} oppId
 * @param {{ platform?: string } | null} info
 * @param {any[]} drives
 * @param {any[]} tools
 * @param {{ windowsOnly: string, btrfs: string, reflink: string, compression: string, tool: (id: string) => string }} words
 */
export function missingRequirement(oppId, info, drives, tools, words) {
  const need = NEEDS[oppId];
  if (!need) return null;
  const platform = info?.platform;
  if (need === 'windows') return platform && platform !== 'windows' ? words.windowsOnly : null;
  if (need === 'btrfs') return drives.length && !drives.some((d) => d.features?.btrfs_compression) ? words.btrfs : null;
  if (need === 'reflink') return drives.length && !drives.some((d) => d.features?.reflinks) ? words.reflink : null;
  if (need === 'compression') return drives.length && !drives.some((d) => d.features?.wof || d.features?.btrfs_compression) ? words.compression : null;
  const tool = tools.find((t) => t.id === need);
  return tool && !tool.found ? words.tool(need) : null;
}
