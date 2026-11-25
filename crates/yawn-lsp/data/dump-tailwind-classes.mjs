// Regenerates tailwind-classes.txt from Tailwind's own design system.
//
//   cd crates/yawn-lsp/data && npm i --no-save tailwindcss@4.3.5 && node dump-tailwind-classes.mjs
//
// Output: one utility per line, then variants as `:name` lines (without the trailing colon).
import { __unstable__loadDesignSystem } from 'tailwindcss';
import fs from 'node:fs';

const root = new URL('./node_modules/tailwindcss/', import.meta.url);
const read = (p) => fs.readFileSync(new URL(p, root), 'utf8');
const ds = await __unstable__loadDesignSystem(read('index.css'), {
  loadStylesheet: async (id, base) => {
    const file = id === 'tailwindcss' ? 'index.css' : id.replace(/^tailwindcss\//, '');
    return { path: file, base, content: read(file) };
  },
});

const classes = ds.getClassList().map(([name]) => name).sort();
const variants = new Set();
for (const v of ds.getVariants()) {
  variants.add(v.name);
  for (const value of v.values) variants.add(`${v.name}-${value}`);
}
const out = classes.join('\n') + '\n' + [...variants].sort().map((v) => ':' + v).join('\n') + '\n';
fs.writeFileSync(new URL('./tailwind-classes.txt', import.meta.url), out);
console.log(`${classes.length} classes, ${variants.size} variants`);
