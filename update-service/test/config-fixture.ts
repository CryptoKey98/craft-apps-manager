// Test the actual Rust-resolved config; do not duplicate TOML defaults in TypeScript.
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
export function productionConfig() {
  const view=JSON.parse(execFileSync('python',['../scripts/lib/updater_config.py','export','--target-os','linux','--view','worker'],{cwd:fileURLToPath(new URL('../',import.meta.url)),encoding:'utf8'}));
  return view.worker_config;
}
