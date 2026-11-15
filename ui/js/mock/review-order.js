// @ts-check
/**
 * The mock's `candidates_page` order, mirroring the engine (crates/engine/src/
 * review_order.rs): flat, or with groups kept together (W6-A) where the sort
 * orders whole groups by an aggregate of the column, filters keep or drop whole
 * groups, the kept copy comes first and pages are packed by groups.
 */

const lc = (s) => String(s).toLowerCase();
const name = (r) => lc(String(r.path).split(/[\\/]/).pop());
const keeper = (r) => (r.badges ?? []).includes('keeper');
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const saving = (r) => r.size - r.size_after;

/** Natural order of two rows for a column (biggest first for size and saving), path as tie-breaker. */
function rowCmp(sort, a, b) {
  const byPath = cmp(a.path, b.path);
  switch (sort) {
    case 'size': return cmp(b.size, a.size) || byPath;
    case 'name': return cmp(name(a), name(b)) || byPath;
    case 'path': return byPath;
    case 'modified': return cmp(a.modified ?? -Infinity, b.modified ?? -Infinity) || byPath;
    case 'group': return cmp(a.group ?? -1, b.group ?? -1) || cmp(keeper(b), keeper(a)) || byPath;
    default: return cmp(saving(b), saving(a)) || byPath;
  }
}

/**
 * @param {any[]} list every row of the opportunity
 * @param {any} req CandidatesPageRequest
 * @param {number} limit
 * @param {boolean} hasGroups
 */
export function orderPage(list, req, limit, hasGroups) {
  const sort = req.sort ?? 'group';
  // Rows come grouped and kept copy first from the fixtures: 'group' keeps that order flat.
  const f = req.filter ? lc(req.filter) : '';
  const min = req.min_size > 0 ? req.min_size : 0;
  let rows = list.filter((r) => !req.sub_group || r.sub_group === req.sub_group);
  const grouped = !!req.keep_groups && hasGroups;
  const offset = req.offset ?? 0;
  if (!grouped) {
    rows = rows.filter((r) => r.size >= min && (!f || lc(r.path).includes(f)));
    if (sort !== 'group') rows.sort((a, b) => rowCmp(sort, a, b));
    if (req.descending) rows.reverse();
    const total = rows.length;
    return { rows: rows.slice(offset, offset + limit), total, offset, grouped, stats: [], page: Math.floor(offset / limit) + 1, pages: Math.max(1, Math.ceil(total / limit)), next: offset + limit < total ? offset + limit : null, prev: offset > 0 ? Math.max(0, offset - limit) : null };
  }
  /** @type {Map<any, any>} */
  const blocks = new Map();
  for (const r of rows) {
    const k = r.group ?? `row:${r.id}`;
    if (!blocks.has(k)) blocks.set(k, { group: r.group, members: [] });
    blocks.get(k).members.push(r);
  }
  let bs = [...blocks.values()].map((b) => {
    const lead = b.members.find(keeper) ?? [...b.members].sort((x, y) => cmp(x.path, y.path))[0];
    const sizes = b.members.map((r) => r.size);
    return { ...b, lead, largest: Math.max(...sizes), smallest: Math.min(...sizes), saving: b.members.reduce((a, r) => a + saving(r), 0), newest: Math.max(...b.members.map((r) => r.modified ?? -Infinity)) };
  });
  bs = bs.filter((b) => b.largest >= min && (!f || b.members.some((r) => lc(r.path).includes(f))));
  const byLead = (a, b) => cmp(a.lead.path, b.lead.path);
  const agg = {
    size: (a, b) => cmp(b.largest, a.largest),
    saving: (a, b) => cmp(b.saving, a.saving),
    name: (a, b) => cmp(name(a.lead), name(b.lead)),
    path: () => 0,
    modified: (a, b) => cmp(a.newest, b.newest),
    group: (a, b) => cmp(a.group ?? -1, b.group ?? -1),
  }[sort] ?? ((a, b) => cmp(b.saving, a.saving));
  bs.sort((a, b) => agg(a, b) || byLead(a, b));
  if (req.descending) bs.reverse();
  const ordered = [];
  const starts = [0];
  const stats = [];
  let used = 0;
  for (const b of bs) {
    b.members.sort((x, y) => cmp(keeper(y), keeper(x)) || rowCmp(sort, x, y));
    if (b.group !== undefined) {
      const kept = b.members.some(keeper);
      const freed = b.members.filter((r) => !keeper(r)).reduce((a, r) => a + saving(r), 0);
      stats.push({ id: b.group, files: b.members.length, largest: b.largest, smallest: b.smallest, reclaimable: kept ? freed : freed - Math.max(...b.members.map(saving)) });
    }
    const n = b.members.length;
    if (used > 0 && used + n > limit) { starts.push(ordered.length); used = 0; }
    let rem = n;
    while (used + rem > limit) { rem -= limit - used; starts.push(ordered.length + n - rem); used = 0; }
    used += rem;
    ordered.push(...b.members);
  }
  if (starts.length > 1 && starts.at(-1) >= ordered.length) starts.pop();
  let i = 0;
  while (i + 1 < starts.length && starts[i + 1] <= offset) i++;
  const end = starts[i + 1] ?? ordered.length;
  return { rows: ordered.slice(starts[i], end), total: ordered.length, offset: starts[i], grouped, stats, page: i + 1, pages: starts.length, next: starts[i + 1] ?? null, prev: i > 0 ? starts[i - 1] : null };
}
