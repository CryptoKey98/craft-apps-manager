import test from 'node:test';
import {productionConfig} from './config-fixture.ts';
import assert from 'node:assert/strict';
import {readFileSync, mkdirSync, mkdtempSync, rmSync} from 'node:fs';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createPublicKey, verify} from 'node:crypto';
import {prepareTestDeployment} from '../scripts/prepare-test-deployment.ts';

test('test deployment is isolated from production routes, assets, limits and trust',()=>{
  const production=productionConfig();
  const config=JSON.parse(readFileSync(new URL('../wrangler.test.jsonc',import.meta.url),'utf8'));
  assert.equal(config.name,'craft-apps-manager-updates-test');
  assert.notEqual(config.name,production.name);
  assert.deepEqual(config.routes,[]);
  assert.equal(config.workers_dev,true);
  assert.equal(config.preview_urls,false);
  assert.equal(config.vars.DYNAMIC_ENABLED,'true');
  assert.notEqual(config.ratelimits[0].namespace_id,production.ratelimits[0].namespace_id);
  assert.equal(config.assets.directory,'./.wrangler/test-public');
  assert.notEqual(config.vars.UPDATE_PUBLIC_KEY,readFileSync(new URL('../../assets/update-public-key.hex',import.meta.url),'utf8').trim());
});

test('test catalog has separate scope, bounded expiry and only public fixture trust',()=>{
  const root=new URL('../.wrangler/',import.meta.url);
  mkdirSync(root,{recursive:true});
  const folder=mkdtempSync(join(fileURLToPath(root),'test-catalog-'));
  try {
    const now=1_800_000_000;
    const {publicKey}=prepareTestDeployment(folder,now);
    const envelope=JSON.parse(readFileSync(join(folder,'v2/catalog.json'),'utf8'));
    const catalog=JSON.parse(envelope.payload);
    const key=createPublicKey({format:'der',type:'spki',key:Buffer.concat([Buffer.from('302a300506032b6570032100','hex'),Buffer.from(publicKey,'hex')])});
    assert.ok(verify(null,Buffer.from(envelope.payload),key,Buffer.from(envelope.signature,'hex')));
    assert.equal(catalog.repository,'owner/manager');
    assert.equal(catalog.channel,'test');
    assert.equal(catalog.issued,now);
    assert.equal(catalog.expires,now+7*86400);
    assert.equal(publicKey,JSON.parse(readFileSync(new URL('../wrangler.test.jsonc',import.meta.url),'utf8')).vars.UPDATE_PUBLIC_KEY);
    assert.match(readFileSync(join(folder,'index.html'),'utf8'),/test service/i);
  } finally {rmSync(folder,{recursive:true,force:true});}
});
