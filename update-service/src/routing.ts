export interface Position {version:string;revision:number}
export interface ClientProfile extends Position {repository:string;channel:string;platform:string;architecture:string;installation:string;capabilities:string[]}
interface Release {id:string;version:string;tag:string}
interface Package {id:string;release:string;revision:number;platform:string;architecture:string;installation:string;handler:string;required_capabilities?:string[];layout:{schema:number}}
interface Route {id:string;min_source:Position;max_source?:Position;platform:string;architecture:string;installation:string;required_capabilities?:string[];target:string;bridge:boolean;destination_capabilities?:string[]}
export interface Catalog {schema:number;repository:string;channel:string;sequence:number;issued:number;expires:number;releases:Release[];packages:Package[];routes:Route[];withdrawn?:string[]}
export interface Offer {immediate_package_id:string;final_package_id:string;route_ids:string[]}
export interface SelectionResult {outcome:'current'|'direct'|'bridge'|'manual'|'unsupported';offer?:Offer}
const compareText=(a:string,b:string)=>a<b?-1:a>b?1:0;
function key(p:Position):number[] {
  if(!Number.isSafeInteger(p.revision)||p.revision<1||p.revision>65535||!/^v?\d+\.\d+(?:\.\d+){0,2}$/.test(p.version)) throw new Error('Invalid version/revision');
  const parts=p.version.replace(/^v/,'').split('.').map(Number);while(parts.length<4)parts.push(0);
  if(parts.some(v=>!Number.isSafeInteger(v)))throw new Error('Version too large');return [...parts,p.revision];
}
function compare(a:Position,b:Position):number {const x=key(a),y=key(b);for(let i=0;i<x.length;i++)if(x[i]!==y[i])return x[i]<y[i]?-1:1;return 0;}
function subset(a:string[],b:string[]):boolean{return a.every(v=>b.includes(v));}
function array(value?:string[]):string[] {if(value===undefined)return [];if(!Array.isArray(value)||value.some(v=>typeof v!=='string'))throw new Error('Invalid capability/withdrawal array');return value;}
function supported(p:Package):boolean {
  return (p.handler==='archive.zip.v1'&&(['windows','linux'].includes(p.platform)&&p.installation==='portable'||p.platform==='macos'&&p.installation==='bundle'))
    || p.handler==='archive.tar-gzip.v1'&&(p.platform==='linux'&&p.installation==='portable'||p.platform==='macos'&&p.installation==='bundle')
    || p.handler==='disk-image.dmg.v1'&&p.platform==='macos'&&p.installation==='bundle'
    || [['native.msi.v1','windows','msi'],['native.deb.v1','linux','deb'],['native.rpm.v1','linux','rpm'],['native.arch.v1','linux','arch']].some(([h,os,kind])=>p.handler===h&&p.platform===os&&p.installation===kind);
}
export function validateCatalog(c:Catalog):void {
  if(c.schema!==2||!Number.isSafeInteger(c.sequence)||c.sequence<1||c.releases.length>128||c.packages.length>512||c.routes.length>512||array(c.withdrawn).length>512)throw new Error('Invalid catalog bounds');
  for(const values of [c.releases,c.packages,c.routes])if(new Set(values.map(v=>v.id)).size!==values.length)throw new Error('Duplicate ID');
  for(const p of c.packages)if(!c.releases.some(r=>r.id===p.release)||array(p.required_capabilities).length>32)throw new Error('Invalid package');
  for(const r of c.routes) {if(!c.packages.some(p=>p.id===r.target)||array(r.required_capabilities).length>32||array(r.destination_capabilities).length>32)throw new Error('Invalid route');key(r.min_source);if(r.max_source&&compare(r.max_source,r.min_source)<0)throw new Error('Invalid range');}
}
export function selectUpdate(c:Catalog,client:ClientProfile):SelectionResult {
  validateCatalog(c);key(client);
  if(c.repository!==client.repository||c.channel!==client.channel)throw new Error('Client catalog scope mismatch');
  const position=(p:Package):Position=>({version:c.releases.find(r=>r.id===p.release)!.version,revision:p.revision});
  const matches=(p:Package,s:ClientProfile)=>p.platform===s.platform&&p.architecture===s.architecture&&p.installation===s.installation&&!array(c.withdrawn).includes(p.id);
  const targets=c.packages.filter(p=>matches(p,client));
  if(!targets.length)return {outcome:'unsupported'};
  if(!targets.some(p=>compare(position(p),client)>0))return {outcome:'current'};
  const queue:{state:ClientProfile;path:Route[]}[]=[{state:client,path:[]}];const seen=new Set<string>();
  let best:{p:Package;offer:Offer}|undefined;
  for(let n=0;n<queue.length;n++) {
    const {state,path}=queue[n];if(path.length>=8)continue;
    for(const r of [...c.routes].sort((a,b)=>compareText(a.id,b.id))) {
      const p=c.packages.find(p=>p.id===r.target)!;
      if(!matches(p,state)||p.layout.schema!==1||!supported(p)||!state.capabilities.includes(p.handler)||!state.capabilities.includes('layout.manager.v1')||!subset(array(p.required_capabilities),state.capabilities)||!subset(array(r.required_capabilities),state.capabilities)||compare(r.min_source,state)>0||r.max_source&&compare(state,r.max_source)>0||compare(position(p),state)<=0)continue;
      const next=[...path,r];const offer:Offer={immediate_package_id:next[0].target,final_package_id:p.id,route_ids:next.map(r=>r.id)};
      if(!best||compare(position(p),position(best.p))>0||compare(position(p),position(best.p))===0&&(offer.route_ids.length<best.offer.route_ids.length||offer.route_ids.length===best.offer.route_ids.length&&(compareText(offer.final_package_id,best.offer.final_package_id)<0||offer.final_package_id===best.offer.final_package_id&&compareText(offer.route_ids.join('\0'),best.offer.route_ids.join('\0'))<0)))best={p,offer};
      const id=p.id+'\0'+[...array(r.destination_capabilities)].sort().join('\0');
      if(r.bridge&&!seen.has(id)){seen.add(id);queue.push({state:{...state,...position(p),capabilities:array(r.destination_capabilities)},path:next});}
    }
  }
  return best?{outcome:best.offer.route_ids.length>1?'bridge':'direct',offer:best.offer}:{outcome:'manual'};
}
