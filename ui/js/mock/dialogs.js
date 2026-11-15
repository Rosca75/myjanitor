// @ts-check
/**
 * Stand-in for the native file dialogs (docs/IPC.md 3.1, `dialog_pick_folder` and
 * `dialog_save_file`). A plain browser and `node --test` have no operating-system
 * dialog, so the mock answers at once and deterministically:
 *
 *  - by default as if the user accepted what the dialog proposed: the folder it
 *    opened in (`start_dir`, else the Documents folder of the mock's platform) and,
 *    for a save dialog, the suggested file name in that folder;
 *  - with a fixed path, or `null` (the user cancelled), once told to:
 *    `createMockBackend({ dialog: { folder, file } })`, `backend.answerDialogs({ ... })`
 *    or the page URL (`?mock=1&dialog=cancel`, `?mock=1&dialog=D:\Projects`).
 *
 * Requests are checked like the shell does (unknown fields are rejected), so a
 * wrong field name fails a UI test instead of being ignored.
 */
import { fail } from './handlers.js';

/** @typedef {{ folder?: string | null, file?: string | null }} DialogAnswers `null` cancels, a string is the answer, `undefined` accepts the proposal. */

const FOLDER_FIELDS = ['title', 'start_dir'];
const SAVE_FIELDS = ['title', 'start_dir', 'file_name', 'filters'];

/** @param {any} req @param {string[]} allowed */
function check(req, allowed) {
  for (const key of Object.keys(req)) {
    if (!allowed.includes(key)) throw fail('invalid_input', `unknown field \`${key}\``, 'This is a bug in the UI.');
  }
  for (const key of ['title', 'start_dir', 'file_name']) {
    if (req[key] !== undefined && req[key] !== null && typeof req[key] !== 'string') throw fail('invalid_input', `\`${key}\` must be a string`, 'This is a bug in the UI.');
  }
  for (const f of req.filters ?? []) {
    if (typeof f?.name !== 'string' || !Array.isArray(f?.extensions)) throw fail('invalid_input', 'a filter needs a name and extensions', 'This is a bug in the UI.');
  }
}

/** The answers the page URL asks for (`?dialog=cancel` or `?dialog=<folder>`). @param {string | null} value @returns {DialogAnswers} */
export function dialogAnswersFromUrl(value) {
  if (!value) return {};
  return value === 'cancel' ? { folder: null, file: null } : { folder: value };
}

/** @param {any} m The mock model; `m.dialog` holds the configured answers. */
export function createDialogs(m) {
  const documents = () => (m.platform === 'windows' ? 'C:\\Users\\Michel\\Documents' : '/home/michel/Documents');
  /** @param {string} dir @param {string} name */
  const join = (dir, name) => {
    const sep = /^[A-Za-z]:|^\\\\/.test(dir) ? '\\' : '/';
    return `${dir.replace(/[\\/]+$/, '')}${sep}${name}`;
  };
  return {
    /** @param {any} req DialogPickFolderRequest @returns {string | null} DialogPath */
    dialog_pick_folder: (req) => {
      check(req, FOLDER_FIELDS);
      return m.dialog.folder !== undefined ? m.dialog.folder : (req.start_dir ?? documents());
    },
    /** @param {any} req DialogSaveFileRequest @returns {string | null} DialogPath */
    dialog_save_file: (req) => {
      check(req, SAVE_FIELDS);
      return m.dialog.file !== undefined ? m.dialog.file : join(req.start_dir ?? documents(), req.file_name ?? 'export');
    },
  };
}
