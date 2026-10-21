import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {selectUpdate, type Catalog, type ClientProfile} from '../src/routing.ts';
const catalog=JSON.parse(readFileSync(new URL('../../tests/fixtures/updater-v2/catalog.json',import.meta.url),'utf8')) as Catalog;
const vectors=JSON.parse(readFileSync(new URL('../../tests/fixtures/updater-v2/routes.json',import.meta.url),'utf8'));
test('macOS tar-gzip bundle routing agrees with the built-in archive handler',()=>{
  const mac=structuredClone(catalog);
  for(const p of mac.packages){p.platform='macos';p.installation='bundle';}
  for(const r of mac.routes){r.platform='macos';r.installation='bundle';}
  const client:ClientProfile={repository:mac.repository,channel:'stable',version:'1.1.0',revision:1,platform:'macos',architecture:'x64',installation:'bundle',capabilities:['archive.zip.v1','archive.tar-gzip.v1','layout.manager.v1','admin.bundle-replace.v1']};
  const actual=selectUpdate(mac,client);
  assert.equal(actual.outcome,'direct');
  assert.deepEqual(actual.offer,{immediate_package_id:'c-linux',final_package_id:'c-linux',route_ids:['to-c']});
});
test('shared Rust/Worker routing vectors',()=>{
  for(const v of vectors) {
    const client:ClientProfile={repository:catalog.repository,channel:'stable',version:v.version,revision:1,platform:'linux',architecture:'x64',installation:'portable',capabilities:['archive.zip.v1','layout.manager.v1',...(v.tar?['archive.tar-gzip.v1']:[])]};
    const actual=selectUpdate(catalog,client);assert.equal(actual.outcome,v.outcome,v.name);
    if(v.outcome!=='current') assert.deepEqual(actual.offer,{immediate_package_id:v.immediate,final_package_id:v.final,route_ids:v.routes});
  }
});
test('omitted optional capability and withdrawal arrays match Rust defaults',()=>{
  const minimal=structuredClone(catalog);
  delete (minimal as any).withdrawn;
  for(const p of minimal.packages)delete (p as any).required_capabilities;
  for(const r of minimal.routes){delete (r as any).required_capabilities;delete (r as any).destination_capabilities;}
  const client:ClientProfile={repository:catalog.repository,channel:'stable',version:'1.0.0',revision:1,platform:'linux',architecture:'x64',installation:'portable',capabilities:['archive.zip.v1','archive.tar-gzip.v1','layout.manager.v1']};
  assert.equal(selectUpdate(minimal,client).outcome,'direct');
});
