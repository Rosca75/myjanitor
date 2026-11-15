import { test } from 'node:test';
import assert from 'node:assert/strict';
import { installFakeDom } from './fakedom.js';
import { h } from '../dom.js';

test('h builds elements with attributes, text children and no markup parsing', () => {
  installFakeDom();
  const el = h('div', { class: ['a', null, 'b'], 'data-x': 1, title: 'T' }, 'hi <b>there</b>', 5, null, [h('span', {}, 'x')]);
  assert.equal(el.getAttribute('class'), 'a b');
  assert.equal(el.getAttribute('data-x'), '1');
  assert.equal(el.textContent, 'hi <b>there</b>5x');
  assert.equal(el.querySelectorAll('b').length, 0);
});

test('h refuses inline handlers and accepts the on map', () => {
  installFakeDom();
  assert.throws(() => h('button', { onclick: 'alert(1)' }), /Inline handler/);
  let n = 0;
  const b = h('button', { on: { click: () => n++ } });
  b.click();
  assert.equal(n, 1);
});

test('disabled elements do not click; checkboxes toggle and fire change', () => {
  installFakeDom();
  let clicks = 0;
  const b = h('button', { disabled: true, on: { click: () => clicks++ } });
  b.click();
  assert.equal(clicks, 0);
  let changed = null;
  const c = h('input', { type: 'checkbox', on: { change: (e) => { changed = e.target.checked; } } });
  c.click();
  assert.equal(changed, true);
});

test('events bubble to ancestors and the document', () => {
  const doc = installFakeDom();
  const inner = h('i');
  const outer = h('div', {}, inner);
  doc.body.append(outer);
  const seen = [];
  outer.addEventListener('keydown', () => seen.push('outer'));
  doc.addEventListener('keydown', () => seen.push('doc'));
  inner.key('a');
  assert.deepEqual(seen, ['outer', 'doc']);
});

test('querySelector handles tag, class, id, attribute (with spaces) and descendant selectors', () => {
  const doc = installFakeDom();
  const tree = h('div', { id: 'root' }, h('ul', { class: 'list big' }, h('li', { 'aria-label': 'Move A up', class: 'x' }, 'one'), h('li', {}, 'two')));
  doc.body.append(tree);
  assert.equal(doc.getElementById('root'), tree);
  assert.equal(tree.querySelectorAll('li').length, 2);
  assert.equal(tree.querySelector('ul.big > li.x').textContent, 'one');
  assert.equal(tree.querySelector('[aria-label="Move A up"]').textContent, 'one');
  assert.equal(tree.querySelector('#root li').textContent, 'one', 'descendant combinator');
  assert.equal(tree.querySelector('#root > li'), null, 'child combinator needs a direct child');
  assert.equal(tree.querySelectorAll('li, ul').length, 3);
});

test('replaceChildren detaches old nodes and textContent replaces children', () => {
  installFakeDom();
  const a = h('p', {}, 'a');
  const box = h('div', {}, a);
  box.replaceChildren(h('b', {}, 'z'));
  assert.equal(a.parentNode, null);
  assert.equal(box.textContent, 'z');
  box.textContent = 'plain';
  assert.equal(box.children.length, 0);
});
