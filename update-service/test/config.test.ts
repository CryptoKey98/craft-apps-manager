import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import YAML from 'yaml';
import {productionConfig} from './config-fixture.ts';
test('workflow YAML parses and deployment remains gated',()=>{
  for(const name of ['check.yml','release.yml','renew-update-catalog.yml']) {
    const doc=YAML.parse(readFileSync(new URL('../../.github/workflows/'+name,import.meta.url),'utf8'));
    assert.ok(doc.jobs);
    if(name==='renew-update-catalog.yml') {assert.equal(doc.jobs.deploy.environment,'manager-update-production');assert.ok(doc.jobs.deploy.if.includes('CRAFT_UPDATE_V2_DEPLOY_ENABLED'));}
  }
});
test('unknown paths use static 404 without Worker fallback',()=>{
  const config=productionConfig();
  assert.deepEqual(config.assets.run_worker_first,['/v2/check']);assert.equal(config.assets.not_found_handling,'404-page');assert.equal(config.vars.DYNAMIC_ENABLED,'false');
  assert.equal(config.ratelimits[0].simple.limit,120);assert.equal(config.ratelimits[0].simple.period,60);
});

test('release selects source explicitly and absent endpoint overrides do not clear TOML',()=>{
  const doc=YAML.parse(readFileSync(new URL('../../.github/workflows/release.yml',import.meta.url),'utf8'));
  assert.equal(doc.env.CRAFT_UPDATE_SOURCE,"${{ vars.CRAFT_UPDATE_V2_ENABLED == 'true' && 'service' || 'github' }}");
  for(const [alias,name] of [['CRAFT_RELEASE_CATALOG_URL','CRAFT_UPDATE_CATALOG_URL'],['CRAFT_RELEASE_CATALOG_MIRROR','CRAFT_UPDATE_CATALOG_MIRROR'],['CRAFT_RELEASE_CHECK_URL','CRAFT_UPDATE_CHECK_URL']]) {
    assert.equal(doc.env[alias], '${{ vars.'+name+' }}');
    assert.equal(doc.env[name],undefined);
  }
});

test('CI covers configured updater modes and canonical script entrypoints',()=>{
  const doc=YAML.parse(readFileSync(new URL('../../.github/workflows/check.yml',import.meta.url),'utf8'));
  const windows=doc.jobs.windows.steps;
  const modes=windows.find((step:{run?:string})=>step.run?.includes('scripts/testing/test_updater_build.py'));
  assert.ok(modes,'CI must exercise actual GitHub/service build selection');
  assert.equal(modes.env.CARGO_TARGET_DIR,'target');
  assert.equal(modes.if,"matrix.target == 'x86_64-pc-windows-msvc'");
  assert.ok(doc.jobs['updater-v2'].steps.some((step:{run?:string})=>step.run?.includes('scripts/testing/test_script_entrypoints.py')));
  for(const name of ['archlinux','macos']) {
    assert.ok(doc.jobs[name].steps.some((step:{run?:string})=>step.run?.includes('cargo clippy')&&step.run.includes('--features service-updater')&&step.run.includes('-D warnings')),name+' must lint the optional updater');
  }
});

test('CI packages the new Linux tar and macOS DMG formats',()=>{
  for(const name of ['check.yml','release.yml']) {
    const doc=YAML.parse(readFileSync(new URL('../../.github/workflows/'+name,import.meta.url),'utf8'));
    assert.ok(doc.jobs.linux.steps.some((step:{run?:string})=>step.run?.includes('scripts/packaging/package-linux-tar.sh')),name+' must create tar.gz');
    assert.ok(doc.jobs.macos.steps.some((step:{run?:string})=>step.run?.includes('scripts/packaging/package-macos-dmg.sh')),name+' must create a DMG');
    const artifact=doc.jobs.macos.steps.find((step:{uses?:string})=>step.uses==='actions/upload-artifact@v4');
    assert.ok(artifact.with.path.includes('*.zip')&&artifact.with.path.includes('*.dmg'),name+' must retain both Mac formats');
  }
  const check=YAML.parse(readFileSync(new URL('../../.github/workflows/check.yml',import.meta.url),'utf8'));
  assert.ok(check.jobs.macos.steps.some((step:{run?:string})=>step.run?.includes('scripts/testing/test_macos_dmg.py')&&step.run.includes('packaging/macos/dmg-requirements.txt')),'DMG layout regression must execute with its pinned dependencies');
});


test('official publication config is explicit and renewal is independent of client activation',()=>{
  const release=YAML.parse(readFileSync(new URL('../../.github/workflows/release.yml',import.meta.url),'utf8'));
  const renew=YAML.parse(readFileSync(new URL('../../.github/workflows/renew-update-catalog.yml',import.meta.url),'utf8'));
  assert.equal(release.env.CRAFT_UPDATER_CONFIG,"${{ vars.CRAFT_UPDATE_V2_ENABLED == 'true' && 'updater.production.toml' || 'updater.toml' }}");
  assert.equal(renew.env.CRAFT_UPDATER_CONFIG,'updater.production.toml');
  assert.ok(renew.jobs.sign.if.includes('CRAFT_UPDATE_CATALOG_RENEW_ENABLED'));
  assert.ok(renew.jobs.deploy.steps.some((s:{run?:string})=>s.run?.includes('mirror_url')&&s.run.includes('Published mirror bytes differ')));
});
