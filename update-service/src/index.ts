import {selectUpdate,type Catalog,type ClientProfile} from './routing.ts';
export interface Env {ASSETS:{fetch(request:Request):Promise<Response>};UPDATES_RATE_LIMITER:{limit(input:{key:string}):Promise<{success:boolean}>};DYNAMIC_ENABLED?:string;UPDATE_PUBLIC_KEY?:string}
export async function handle(request:Request,env:Env):Promise<Response> {
  const url=new URL(request.url);
  const response=(body:string,status:number,extra:Record<string,string>={})=>new Response(request.method==='HEAD'?null:body,{status,headers:{'Content-Type':'application/json','Cache-Control':'no-store',...extra}});
  if(url.pathname!=='/v2/check')return response('{"error":"not found"}',404);
  if(!['GET','HEAD'].includes(request.method))return response('{"error":"method"}',405,{Allow:'GET, HEAD'});
  if(!await env.UPDATES_RATE_LIMITER.limit({key:'craft-manager-updates:'+ (request.headers.get('CF-Connecting-IP')??'unknown')}).then(v=>v.success))return response('{"error":"retry later"}',429,{'Retry-After':'60'});
  if(env.DYNAMIC_ENABLED!=='true')return response('{"error":"use signed static catalog"}',503,{'Retry-After':'60'});
  try {
    if(url.search.length>2048)throw new Error('query too long');
    const names=['version','revision','platform','architecture','installation','capabilities','repository','channel'];
    for(const name of url.searchParams.keys())if(!names.includes(name)||url.searchParams.getAll(name).length!==1)throw new Error('invalid parameter');
    const p=Object.fromEntries(names.map(n=>[n,url.searchParams.get(n)]));
    if(Object.values(p).some(v=>!v||v.length>256))throw new Error('missing/long parameter');
    const capabilities=p.capabilities!.split(',');
    if(capabilities.length>32||capabilities.some(c=>!/^[-.a-z0-9]+$/.test(c)))throw new Error('invalid capabilities');
    const client:ClientProfile={version:p.version!,revision:Number(p.revision),platform:p.platform!,architecture:p.architecture!,installation:p.installation!,capabilities,repository:p.repository!,channel:p.channel!};
    const asset=await env.ASSETS.fetch(new Request(new URL('/v2/catalog.json',url.origin)));
    if(!asset.ok)return response('{"error":"catalog unavailable"}',503);
    const text=await asset.text();if(text.length>2*1024*1024)return response('{"error":"catalog bounds"}',503);
    const envelope=JSON.parse(text) as {payload:string;signature:string};
    if(typeof envelope.payload!=='string'||new TextEncoder().encode(envelope.payload).length>1024*1024||!env.UPDATE_PUBLIC_KEY||!/^[a-f0-9]{128}$/i.test(envelope.signature))return response('{"error":"catalog invalid"}',503);
    const hex=(s:string)=>Uint8Array.from(s.match(/../g)!.map(v=>parseInt(v,16)));
    const key=await crypto.subtle.importKey('raw',hex(env.UPDATE_PUBLIC_KEY),{name:'Ed25519'},false,['verify']);
    if(!await crypto.subtle.verify('Ed25519',key,hex(envelope.signature),new TextEncoder().encode(envelope.payload)))return response('{"error":"catalog invalid"}',503);
    const catalog=JSON.parse(envelope.payload) as Catalog;const now=Math.floor(Date.now()/1000);
    if(catalog.expires<=now||catalog.issued>now+300||catalog.expires-catalog.issued>2592000)return response('{"error":"catalog expired"}',503);
    return response(JSON.stringify({envelope,selection:selectUpdate(catalog,client)}),200);
  } catch {return response('{"error":"invalid update request"}',400);}
}
export default {fetch:handle};
