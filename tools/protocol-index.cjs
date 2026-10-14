#!/usr/bin/env node
// Offline descriptor extraction. Evaluates only the self-contained protobuf bundle
// in a VM without provided require/process/network/timer/filesystem APIs.
// Node VM is a bounded evaluation environment, not a security sandbox guarantee.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '..');
const rel = 'dist/electron-main/proto.cjs';
const raw = fs.readFileSync(path.join(root, 'artifacts/client', rel), 'utf8');
const formatted = fs.readFileSync(path.join(root, 'artifacts/formatted', rel), 'utf8');
let indexedSource = null, lineOffsets = [];
const classSymbols = [...formatted.matchAll(/(?:var\s+|,\s*)([\w$]+)\s*=\s*class\s+[\w$]*\s+extends\s+[\w$]+/g)].map(m => m[1]);
const serviceSymbols = [...formatted.matchAll(/(?:var\s+|,\s*)([\w$]+)\s*=\s*\{\s*typeName:\s*"(aiserver\.v1\.[^"]+Service)"/g)].map(m => ({symbol:m[1], typeName:m[2], formattedLine:line(formatted,m.index)}));
const ctx = vm.createContext({module:{exports:{}}, TextEncoder, TextDecoder, Uint8Array});
vm.runInContext(raw + '\n;globalThis.__protocol={classes:{' + classSymbols.map(x => `${x}:${x}`).join(',') + '},services:{' + serviceSymbols.map(x => `${x.symbol}:${x.symbol}`).join(',') + '},exports:module.exports};',ctx,{timeout:15000,filename:rel});
const scalar = {1:'double',2:'float',3:'int64',4:'uint64',5:'int32',6:'fixed64',7:'fixed32',8:'bool',9:'string',12:'bytes',13:'uint32',15:'sfixed32',16:'sfixed64',17:'sint32',18:'sint64'};
const enums = new Map(), messages = new Map();
function type(t,kind) {
  if (kind === 'scalar') return scalar[t] || `unknown_scalar_${t}`;
  if (kind === 'message') {visit(t);return t.typeName;}
  if (kind === 'enum') { enums.set(t.typeName,{typeName:t.typeName,values:Array.from(t.values).map(x=>({number:x.no,name:x.name,localName:x.localName}))});return t.typeName;}
}
function visit(t,symbol) {
 if (!t || typeof t !== 'function' || !t.typeName || !t.fields || typeof t.fields.list !== 'function') return;
 const name=t.typeName;
 if(messages.has(name)) {if(symbol && !messages.get(name).symbols.includes(symbol)) messages.get(name).symbols.push(symbol);return;}
 const schema=typeof t.$ === 'function' ? t.$()[0] : null;
 const literal='"'+(schema??name).replace(/"/g,'\\"')+'"';
 const assignment = new RegExp('\\.typeName\\s*=\\s*"'+name.replaceAll('.', '\\.')+'"');
 const at=schema === null ? formatted.search(assignment) : formatted.indexOf(literal);
 const origin=schema === null ? raw.search(assignment) : raw.indexOf(literal);
 const record={typeName:name,symbols:symbol?[symbol]:[],source:{path:rel,formattedLine:at<0?null:line(formatted,at),originalCharacterOffset:origin<0?null:origin,originalOffsetUnit:'UTF-16 code units in decoded JavaScript'},compactDescriptor:schema,fields:[]};
 messages.set(name,record);
 record.fields=Array.from(t.fields.list()).map(f=>({number:f.no,protoName:f.name,localName:f.localName,jsonName:f.jsonName,kind:f.kind,repeated:f.repeated===true,optional:f.opt===true,oneof:f.oneof?.name??null,...(f.kind==='map'?{keyType:scalar[f.K],valueType:type(f.V.T,f.V.kind)}:{type:type(f.T,f.kind)})}));
}
for(const [symbol,t] of Object.entries(ctx.__protocol.classes))visit(t,symbol);
for(const [name,v] of Object.entries(ctx.__protocol.exports))if(typeof v==='function')visit(v);
const services=serviceSymbols.map(s=>{
 const obj=ctx.__protocol.services[s.symbol];
 return {...s,source:{path:rel,formattedLine:s.formattedLine,originalCharacterOffset:raw.indexOf('typeName:"'+s.typeName+'"'),originalOffsetUnit:'UTF-16 code units in decoded JavaScript'},methods:Object.entries(obj.methods).map(([local,m])=>{visit(m.I);visit(m.O);return{localName:local,rpcName:m.name,input:m.I.typeName,output:m.O.typeName,kind:['Unary','ServerStreaming','ClientStreaming','BiDiStreaming'][m.kind]??m.kind,kindNumber:m.kind,formattedLine:findMethodLine(formatted,s,local),callCandidates:[]};})};
});
const aliases=Object.entries(ctx.__protocol.exports).filter(([k,v])=>v&&typeof v==='object'&&v.typeName&&v.methods).map(([exportName,v])=>({exportName,typeName:v.typeName,methods:Object.keys(v.methods)}));
const methodMap = new Map();
for(const s of services)for(const m of s.methods){const a=methodMap.get(m.localName)||[];a.push(m);methodMap.set(m.localName,a);}
const names=[...methodMap.keys()].sort((a,b)=>b.length-a.length);
const pattern=new RegExp('\\.('+names.join('|')+')\\s*\\(','g');
let filesScanned=0;
function walk(dir){for(const d of fs.readdirSync(dir,{withFileTypes:true})){const p=path.join(dir,d.name);if(d.isDirectory())walk(p);else if(/\.(?:js|cjs|mjs)$/.test(d.name)){const rr=path.relative(path.join(root,'artifacts/client'),p);if(rr===rel)continue;const src=fs.readFileSync(p,'utf8');filesScanned++;const fp=path.join(root,'artifacts/formatted',rr);const ft=fs.existsSync(fp)?fs.readFileSync(fp,'utf8'):null;const lineMap=new Map();if(ft){const re=new RegExp(pattern.source,'g');for(const m of ft.matchAll(re)){const a=lineMap.get(m[1])||[];a.push(line(ft,m.index));lineMap.set(m[1],a);}}const ord=new Map();for(const c of src.matchAll(new RegExp(pattern.source,'g'))){const n=ord.get(c[1])||0;ord.set(c[1],n+1);const loc={path:rr,originalCharacterOffset:c.index,originalOffsetUnit:'UTF-16 code units in decoded JavaScript',formattedLine:lineMap.get(c[1])?.[n]??null,evidenceType:'static property-call candidate; receiver and branch require manual tracing'};for(const m of methodMap.get(c[1]))m.callCandidates.push(loc);}}}}
walk(path.join(root,'artifacts/client/dist'));
function line(src,at){
 if(indexedSource!==src){indexedSource=src;lineOffsets=[];for(let i=src.indexOf('\n');i>=0;i=src.indexOf('\n',i+1))lineOffsets.push(i);}
 let lo=0,hi=lineOffsets.length;while(lo<hi){const m=(lo+hi)>>>1;if(lineOffsets[m]<at)lo=m+1;else hi=m;}return lo+1;
}
function findMethodLine(src,s,name){const start=src.indexOf('typeName: "'+s.typeName+'"');const p=src.indexOf(name+':',start);return p<0?null:line(src,p);}
const result={target:'Grok Bot 0.70.2',source:{path:rel,sha256:crypto.createHash('sha256').update(raw).digest('hex')},methodKindMapping:{0:'Unary',1:'ServerStreaming',2:'ClientStreaming',3:'BiDiStreaming'},descriptorRecovery:{mode:'controlled evaluation of shipped proto.cjs in Node VM',executesTargetJavaScript:true,operations:['initialize descriptor classes and service objects','read compact and explicit protobuf field metadata'],providedIoApis:false,applicationLaunched:false,remoteCallsMade:false,securitySandboxGuarantee:false},limitations:['Behavioral claims are static traces; descriptor recovery evaluates shipped JavaScript.', 'Not application runtime observation; no Electron or coordinator launch.','Exported services are subsets of full descriptors.','Property-call candidates do not prove receiver identity or reachable server behavior.','No backend source, account state, secrets or remote calls were accessed.'],counts:{fullServices:services.length,fullServiceMethods:services.reduce((a,s)=>a+s.methods.length,0),uniqueRpcTypePairs:new Set(services.flatMap(s=>s.methods.map(m=>m.rpcName+'|'+m.input+'|'+m.output))).size,messages:messages.size,enums:enums.size,filesScanned},services,exportedServiceAliases:aliases,messages:[...messages.values()].sort((a,b)=>a.typeName.localeCompare(b.typeName)),enums:[...enums.values()].sort((a,b)=>a.typeName.localeCompare(b.typeName))};
fs.writeFileSync(path.join(root,'evidence/protocol-contracts.json'),JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({counts:result.counts,services:services.map(s=>({name:s.typeName,symbol:s.symbol,methods:s.methods.length,called:s.methods.filter(m=>m.callCandidates.length).length})),aliases:aliases.map(a=>({name:a.exportName,methods:a.methods.length}))},null,2));
