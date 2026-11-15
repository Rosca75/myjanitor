// Lane-a's review controls (filtered select-all on a grouped table, the recorded keep
// rule) feed lane-b's one savings account: the card shows what the selection frees and
// the hero stays the sum of the cards.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createMockBackend } from './mock/backend.js';

const DUP = 'det:exact_duplicates';

async function heroIsSumOfCards(b) {
  const all = await b.call('opportunities_list', { tab: 'all', sort: 'saving' });
  const available = all.items.filter((o) => o.availability.state === 'available');
  assert.equal(all.summary.total, available.reduce((a, o) => a + o.saving, 0), 'hero = sum of the cards');
  return available.find((o) => o.id === DUP);
}

test('filtered select-all on the grouped table and the keep rule keep the hero the sum of the cards', async () => {
  const b = createMockBackend({ scenario: 'results', speed: 0 });
  await b.call('selection_set', { opportunity_id: DUP, selected: false });
  assert.equal((await heroIsSumOfCards(b)).saving, 0);

  // Select all on what the grouped, filtered table shows: one group of four, its kept copy stays.
  await b.call('selection_set', { opportunity_id: DUP, filter: 'Report_Q3', keep_groups: true, selected: true });
  const page = await b.call('candidates_page', { opportunity_id: DUP, keep_groups: true, limit: 500 });
  const picked = page.rows.filter((r) => r.selected);
  assert.ok(picked.length > 0 && picked.every((r) => r.path.includes('Report_Q3') && !r.badges.includes('keeper')));
  assert.equal((await heroIsSumOfCards(b)).saving, picked.reduce((a, r) => a + r.size, 0));

  // The keep rule selects every copy but one in every group; the account follows.
  await b.call('selection_apply_keep_rule', { opportunity_id: DUP, rule: { rule: 'newest' } });
  const after = await b.call('candidates_page', { opportunity_id: DUP, keep_groups: true, limit: 500 });
  const freed = after.rows.filter((r) => r.selected).reduce((a, r) => a + r.size, 0);
  const card = await heroIsSumOfCards(b);
  assert.equal(card.saving, freed);
  const detail = await b.call('step_detail', { opportunity_id: DUP });
  assert.equal(detail.opportunity.params.keep_rule, 'newest', 'the rule is recorded');
});
