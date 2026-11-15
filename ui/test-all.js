// Entry for `node --test ui/`: Node 22 resolves a directory argument through this
// package's "main", so this file imports every `*.test.js` below ui/ (except
// node_modules). Add tests as `ui/**/<name>.test.js`; nothing to register here.
import { readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));

function find(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    if (name === 'node_modules' || name.startsWith('.')) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) find(p, out);
    else if (name.endsWith('.test.js')) out.push(p);
  }
  return out;
}

for (const file of find(root)) {
  await import(pathToFileURL(file).href);
}
