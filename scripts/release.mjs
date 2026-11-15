// Releases a version: pushes main and a v<version> tag, which starts the
// GitHub Actions build of the Windows installers (a draft release).
//
//     npm run release             the version in app/tauri.conf.json
//     npm run release -- 0.2.0    sets 0.2.0 first (app and crates), commits it
//
// Pushes through the GitHub CLI's login, so `gh auth login` is all the setup.

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { cwd: root, encoding: 'utf8', stdio: opts.quiet ? 'pipe' : 'inherit', ...opts });
const git = (...args) => run('git', args, { quiet: true }).trim();
const fail = (msg) => {
  console.error(`release: ${msg}`);
  process.exit(1);
};
const push = (...args) => {
  try {
    run('git', ['-c', 'credential.helper=', '-c', 'credential.helper=!gh auth git-credential', 'push', ...args]);
  } catch {
    // git has printed why; a missing "workflow" permission is fixed with:
    fail('the push failed (see above). If GitHub asked for the "workflow" scope, run: gh auth refresh -h github.com -s workflow');
  }
};

const CONF = 'app/tauri.conf.json';
const CARGO = 'Cargo.toml';
const wanted = process.argv[2];

if (git('branch', '--show-current') !== 'main') fail('switch to main first');
if (git('status', '--porcelain')) fail('commit or stash your changes first');

if (wanted) {
  if (!/^\d+\.\d+\.\d+$/.test(wanted)) fail(`"${wanted}" is not a version like 1.2.3`);
  const conf = readFileSync(`${root}/${CONF}`, 'utf8');
  writeFileSync(`${root}/${CONF}`, conf.replace(/("version":\s*")[^"]+(")/, `$1${wanted}$2`));
  const cargo = readFileSync(`${root}/${CARGO}`, 'utf8');
  writeFileSync(`${root}/${CARGO}`, cargo.replace(/(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/, `$1${wanted}$2`));
  run('cargo', ['update', '--workspace', '--offline']); // the crates' versions in Cargo.lock
  run('git', ['commit', '-am', `Release v${wanted}`]);
}

const version = JSON.parse(readFileSync(`${root}/${CONF}`, 'utf8')).version;
const tag = `v${version}`;
if (git('tag', '--list', tag)) fail(`${tag} already exists; release a new version: npm run release -- <version>`);

push('origin', 'main');
run('git', ['tag', '-a', tag, '-m', `Coolmus ${version}`]);
push('origin', tag);

const repo = git('remote', 'get-url', 'origin').replace(/^.*github\.com[:/]/, '').replace(/\.git$/, '');
console.log(`\n${tag} pushed. The build: https://github.com/${repo}/actions`);
console.log(`When it's done, publish the draft: https://github.com/${repo}/releases`);
