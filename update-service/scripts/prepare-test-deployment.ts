// Isolated metadata fixtures only. Never reads the production signing key.
import {readFileSync, mkdirSync, writeFileSync, copyFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createPrivateKey, createPublicKey, sign} from 'node:crypto';
const service=resolve(dirname(fileURLToPath(import.meta.url)),'..');

export function prepareTestDeployment(outDir:string, now=Math.floor(Date.now()/1000)):{publicKey:string} {
  const config=JSON.parse(readFileSync(join(service,'wrangler.test.jsonc'),'utf8'));
  const catalog=JSON.parse(readFileSync(join(service,'../tests/fixtures/updater-v2/catalog.json'),'utf8'));
  // Public, deterministic fixture seed already used by the isolated Rust tests.
  const key=createPrivateKey({format:'der',type:'pkcs8',key:Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'),Buffer.alloc(32,7)])});
  const publicKey=createPublicKey(key).export({format:'der',type:'spki'}).subarray(-32).toString('hex');
  if(publicKey===readFileSync(join(service,'../assets/update-public-key.hex'),'utf8').trim())throw new Error('Fixture trust must differ from production');
  if(publicKey!==config.vars.UPDATE_PUBLIC_KEY)throw new Error('Test config key mismatch');
  if(config.name!=='craft-apps-manager-updates-test'||config.routes.length!==0)throw new Error('Test deployment isolation mismatch');
  catalog.channel='test';
  catalog.sequence=now;
  catalog.issued=now;
  catalog.expires=now+7*86400;
  const payload=JSON.stringify(catalog);
  const envelope={payload,signature:sign(null,Buffer.from(payload),key).toString('hex')};
  mkdirSync(join(outDir,'v2'),{recursive:true});
  writeFileSync(join(outDir,'v2/catalog.json'),JSON.stringify(envelope));
  copyFileSync(join(service,'public/_headers'),join(outDir,'_headers'));
  copyFileSync(join(service,'public/404.html'),join(outDir,'404.html'));
  writeFileSync(join(outDir,'index.html'),'<!doctype html><html lang="en"><meta charset="utf-8"><title>Craft Apps Manager updater test service</title><h1>Updater test service</h1><p>Isolated signed metadata and routing fixtures. No installable application releases are hosted here. Production managers do not trust this test key.</p></html>');
  return {publicKey};
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  prepareTestDeployment(join(service,'.wrangler/test-public'));
  console.log('Prepared test-only signed catalog (7-day validity); production assets unchanged.');
}
