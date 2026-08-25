// @ts-check
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateRawSync } from 'node:zlib';

/**
 * @typedef {object} Entry
 * @property {string} name
 * @property {string} path
 * @property {"file" | "dir" | "symlink" | "submodule"} type
 * @property {string | null} download_url
 */

const here = dirname(fileURLToPath(import.meta.url));
const resources = join(here, '..', 'src-tauri', 'resources');
const renpy = join(resources, 'renpy');
const unity = join(resources, 'unity');

const TYPETREES =
  'https://nightly.link/AssetRipper/Tpk/workflows/type_tree_tpk/master/lz4_file.zip';
const headers = {
  'user-agent': 'update-resources',
  accept: 'application/vnd.github+json',
};
const wanted = new Set(['unrpyc.py', 'deobfuscate.py', 'decompiler']);

/**
 * @param {string} url
 * @returns {Promise<Response>}
 */
async function ask(url) {
  const response = await fetch(url, { headers });
  if (!response.ok)
    throw new Error(`${response.status} ${response.statusText} for ${url}`);
  return response;
}

/**
 * @param {string} repo
 * @param {string} branch
 * @param {string} remote
 * @returns {Promise<Entry[]>}
 */
async function listing(repo, branch, remote) {
  const url = `https://api.github.com/repos/${repo}/contents/${remote}?ref=${branch}`;
  return await (await ask(url)).json();
}

/**
 * @param {string} url
 * @param {string} to
 * @returns {Promise<void>}
 */
async function save(url, to) {
  await writeFile(to, Buffer.from(await (await ask(url)).arrayBuffer()));
}

/**
 * @param {Buffer} zip
 * @returns {Buffer}
 */
function unzip(zip) {
  const tail = zip.lastIndexOf(Buffer.from('PK\x05\x06', 'latin1'));
  if (tail < 0) throw new Error('no end of central directory in zip');

  const central = zip.readUInt32LE(tail + 16);
  const method = zip.readUInt16LE(central + 10);
  const packed = zip.readUInt32LE(central + 20);
  const header = zip.readUInt32LE(central + 42);
  const names = zip.readUInt16LE(header + 26);
  const extras = zip.readUInt16LE(header + 28);
  const data = header + 30 + names + extras;

  const entry = zip.subarray(data, data + packed);
  return method === 0 ? Buffer.from(entry) : inflateRawSync(entry);
}

/**
 * @param {string} repo
 * @param {string} branch
 * @param {string} remote
 * @param {string} local
 * @param {Set<string>} [filter]
 * @returns {Promise<number>}
 */
async function pull(repo, branch, remote, local, filter) {
  await mkdir(local, { recursive: true });
  let count = 0;

  for (const entry of await listing(repo, branch, remote)) {
    if (filter && !filter.has(entry.name)) continue;

    if (entry.type === 'dir') {
      count += await pull(repo, branch, entry.path, join(local, entry.name));
    } else if (entry.type === 'file' && entry.download_url) {
      await save(entry.download_url, join(local, entry.name));
      count += 1;
    }
  }

  return count;
}

/**
 * @param {string} branch
 * @param {string} into
 * @returns {Promise<void>}
 */
async function unrpyc(branch, into) {
  const local = join(renpy, into);
  await rm(local, { recursive: true, force: true });
  const count = await pull(
    'CensoredUsername/unrpyc',
    branch,
    '',
    local,
    wanted,
  );
  console.log(`${into} <- unrpyc@${branch} (${count} files)`);
}

await unrpyc('master', 'unrpyc');
await unrpyc('legacy', 'unrpyc-legacy');

await save(
  'https://codeberg.org/shiz/rpatool/raw/branch/master/rpatool',
  join(renpy, 'rpatool.py'),
);
console.log('rpatool.py <- shiz/rpatool@master');

await mkdir(unity, { recursive: true });
const pack = unzip(Buffer.from(await (await ask(TYPETREES)).arrayBuffer()));
if (!pack.subarray(0, 4).equals(Buffer.from('TPK*')) || pack[4] !== 2)
  throw new Error(
    `typetrees.tpk arrived as format ${pack[4]}, expected format 2`,
  );
await writeFile(join(unity, 'typetrees.tpk'), pack);
console.log('unity/typetrees.tpk <- AssetRipper/Tpk@master');
console.log('done; rebuild to embed them');
