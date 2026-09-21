import {readFile,writeFile} from 'node:fs/promises';
const base=new URL('../',import.meta.url);
const schemas=JSON.parse(await readFile(new URL('protocol/trae/codecraft-v1/schema.json',base),'utf8'));
const all={};
for(const [name,schema] of Object.entries(schemas)){Object.assign(all,schema.$defs);all[name]=schema;}
function type(s){
  if(s===true)return 'unknown';if(s===false)return 'never';
  if(s.$ref)return s.$ref.split('/').at(-1);
  if(s.const!==undefined)return JSON.stringify(s.const);
  if(s.enum)return s.enum.map(v=>JSON.stringify(v)).join(' | ');
  if(s.anyOf||s.oneOf)return (s.anyOf??s.oneOf).map(type).join(' | ');
  if(Array.isArray(s.type))return s.type.map(t=>type({...s,type:t})).join(' | ');
  if(s.type==='null')return 'null';if(s.type==='string')return 'string';if(s.type==='boolean')return 'boolean';if(s.type==='integer'||s.type==='number')return 'number';
  if(s.type==='array')return `Array<${type(s.items??true)}>`;
  if(s.type==='object'||s.properties)return `{ ${Object.entries(s.properties??{}).map(([k,v])=>`${JSON.stringify(k)}${s.required?.includes(k)?'':'?'}: ${type(v)}`).join('; ')} }`;
  return 'unknown';
}
const output='// Generated from Rust JSON Schema. Run scripts/generate-trae-types.mjs.\n'+Object.entries(all).map(([name,s])=>`export type ${name} = ${type(s)};`).join('\n')+'\n';
const target=new URL('src/trae-contract.generated.ts',base);
if(process.argv.includes('--check')) {
  if((await readFile(target,'utf8')).replaceAll('\r\n','\n')!==output)throw new Error('Trae DTO is stale; run scripts/generate-trae-types.mjs');
} else await writeFile(target,output);
