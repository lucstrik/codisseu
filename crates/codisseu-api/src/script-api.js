// Independent synchronous compatibility layer, constructed entirely inside the VM.
const env=Object.assign(Object.create(null),data.environment);
const coll=Object.assign(Object.create(null),data.collection);
const glob=Object.assign(Object.create(null),data.context.globals);
const local=Object.assign(Object.create(null),data.context.locals);
const iterationData=Object.assign(Object.create(null),data.context.iteration_data);
const updates=Object.create(null),collection=Object.create(null),globalUpdates=Object.create(null);
const envUnset=[],collUnset=[],globalUnset=[],tests=[],logs=[];
const write=(target,scope,unsets,k,v)=>{
  k=String(k);if(v===null||v===undefined)throw Error('CODISSEU_UNDEFINED_VARIABLE');v=String(v);
  if(k.length>1024||v.length>65536||Object.keys(target).length>=256)throw Error('Variable limit reached');
  target[k]=v;scope[k]=v;const i=unsets.indexOf(k);if(i>=0)unsets.splice(i,1);
};
const remove=(target,scope,unsets,k)=>{k=String(k);if(unsets.length>=256)throw Error('Variable limit reached');delete target[k];delete scope[k];if(!unsets.includes(k))unsets.push(k);};
const scope=(values,changes,unsets,available=true)=>Object.freeze({
 get:k=>values[k],has:k=>Object.hasOwn(values,k),toObject:()=>Object.assign({},values),
 set:(k,v)=>{if(!available)throw Error('CODISSEU_NO_ENVIRONMENT');write(changes,values,unsets,k,v);},
 unset:k=>{if(!available)throw Error('CODISSEU_NO_ENVIRONMENT');remove(changes,values,unsets,k);},
 clear:()=>{if(!available)throw Error('CODISSEU_NO_ENVIRONMENT');for(const k of Object.keys(values))remove(changes,values,unsets,k);}
});
const environment=scope(env,updates,envUnset,data.environment_available);
const collectionVariables=scope(coll,collection,collUnset);
const globals=scope(glob,globalUpdates,globalUnset);
const dynamic=k=>k==='$timestamp'?String(Math.floor(Date.now()/1000)):k==='$isoTimestamp'?new Date().toISOString():k==='$randomInt'?String(Math.floor(Math.random()*1001)):k==='$guid'||k==='$randomUUID'?'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,c=>{const n=Math.floor(Math.random()*16);return(c==='x'?n:(n&3)|8).toString(16);}):undefined;
const get=k=>Object.hasOwn(local,k)?local[k]:Object.hasOwn(iterationData,k)?iterationData[k]:Object.hasOwn(env,k)?env[k]:Object.hasOwn(coll,k)?coll[k]:Object.hasOwn(glob,k)?glob[k]:dynamic(k);
const replaceIn=s=>String(s).replace(/\{\{([^}]+)\}\}/g,(_,k)=>{const value=get(k.trim());if(value===undefined)throw Error('Unresolved variable');return value;});
const variables=Object.freeze({...scope(local,local,[]),get,has:k=>get(k)!==undefined,replaceIn,toObject:()=>({...glob,...coll,...env,...iterationData,...local})});
const iteration=Object.freeze({get:k=>iterationData[k],has:k=>Object.hasOwn(iterationData,k),toObject:()=>({...iterationData})});
const deepEqual=(a,b)=>{
 if(Object.is(a,b))return true;if(!a||!b||typeof a!=='object'||typeof b!=='object'||Array.isArray(a)!==Array.isArray(b))return false;
 const ak=Object.keys(a),bk=Object.keys(b);return ak.length===bk.length&&ak.every(k=>Object.hasOwn(b,k)&&deepEqual(a[k],b[k]));
};
const expect=(actual,message,negated=false,deep=false)=>{
 const check=ok=>{if(Boolean(ok)===negated)throw Error(message||'Assertion failed');};
 const chain={
  equal:expected=>{check(deep?deepEqual(actual,expected):actual===expected);return chain;},
  eql:expected=>{check(deepEqual(actual,expected));return chain;},
  include:expected=>{check(typeof actual==='string'||Array.isArray(actual)?actual.includes(expected):actual&&typeof expected==='object'?Object.keys(expected).every(k=>deepEqual(actual[k],expected[k])):actual&&Object.hasOwn(actual,expected));return chain;},
  property:(key,...values)=>{const exists=actual!==null&&actual!==undefined&&Object.hasOwn(Object(actual),key);check(exists&&(!values.length||deepEqual(actual[key],values[0])));return exists?expect(actual[key],message):chain;},
  a:type=>{check(type==='array'?Array.isArray(actual):type==='null'?actual===null:typeof actual===type);return chain;},
  above:n=>{check(actual>n);return chain;},below:n=>{check(actual<n);return chain;},
  least:n=>{check(actual>=n);return chain;},most:n=>{check(actual<=n);return chain;},
  within:(a,b)=>{check(actual>=a&&actual<=b);return chain;},
  lengthOf:n=>{check(actual?.length===n);return chain;},
  oneOf:items=>{check(items.some(i=>deepEqual(actual,i)));return chain;},
  match:pattern=>{check(pattern.test(actual));return chain;},
  keys:(...keys)=>{const expected=Array.isArray(keys[0])?keys[0]:keys;check(deepEqual(Object.keys(actual).sort(),expected.sort()));return chain;}
 };
 chain.equals=chain.eq=chain.equal;chain.includes=chain.contain=chain.contains=chain.include;chain.an=chain.a;chain.greaterThan=chain.above;chain.lessThan=chain.below;
 for(const name of ['to','be','have','and','is','that','which','at','of','same'])Object.defineProperty(chain,name,{get:()=>chain});
 Object.defineProperty(chain,'not',{get:()=>expect(actual,message,!negated,deep)});
 Object.defineProperty(chain,'deep',{get:()=>expect(actual,message,negated,true)});
 for(const [name,test]of Object.entries({true:()=>actual===true,false:()=>actual===false,null:()=>actual===null,undefined:()=>actual===undefined,ok:()=>!!actual,exist:()=>actual!==null&&actual!==undefined,empty:()=>typeof actual==='string'||Array.isArray(actual)?actual.length===0:actual&&typeof actual==='object'&&Object.keys(actual).length===0}))Object.defineProperty(chain,name,{get(){check(test());return chain;}});
 return chain;
};
const list=(rows,caseInsensitive=false,mutable=true)=>{
 const same=(a,b)=>caseInsensitive?String(a).toLowerCase()===String(b).toLowerCase():String(a)===String(b);
 const requireMutable=()=>{if(!mutable)throw Error('Read-only list');};
 const normalize=row=>{if(typeof row==='string'){const index=row.indexOf(':');return{key:row.slice(0,index).trim(),value:row.slice(index+1).trim(),enabled:true};}return{key:String(row.key),value:String(row.value??''),enabled:row.disabled!==true&&row.enabled!==false};};
 const api={get:k=>rows.find(r=>r.enabled!==false&&same(r.key,k))?.value,has:k=>rows.some(r=>r.enabled!==false&&same(r.key,k)),all:()=>rows.map(r=>({...r})),toObject:()=>Object.fromEntries(rows.filter(r=>r.enabled!==false).map(r=>[r.key,r.value])),each:fn=>rows.forEach((r,i)=>fn({...r},i)),add:row=>{requireMutable();if(rows.length>=256)throw Error('List limit');rows.push(normalize(row));},remove:k=>{requireMutable();for(let i=rows.length-1;i>=0;i--)if(same(rows[i].key,k))rows.splice(i,1);},upsert:row=>{requireMutable();const value=normalize(row);const i=rows.findIndex(r=>same(r.key,value.key));if(i>=0)rows[i]=value;else api.add(value);}};return Object.freeze(api);
};
const headerRows=(data.request?.headers||[]).map(r=>({...r}));
const queryRows=(data.request?.params||[]).map(r=>({...r}));

const request=data.request?{method:data.request.method,headers:list(headerRows,true),url:Object.freeze({toString:()=>data.request.url,query:list(queryRows),addQueryParams:rows=>{for(const row of(Array.isArray(rows)?rows:[rows]))request.url.query.add(row);}}),body:Object.freeze({mode:data.request.body?.Raw?'raw':'none',raw:data.request.body?.Raw?.text})}:undefined;
const responseHeaders=data.response?.headers||[];
const response=data.response?Object.freeze({code:data.response.code,status:data.response.status,responseTime:data.response.responseTime,size:()=>data.response.body.length,json:()=>JSON.parse(data.response.body),text:()=>data.response.body,headers:list(responseHeaders,true,false),to:{have:{status:n=>expect(data.response.code).equal(n),header:(k,v)=>{expect(responseHeaders.some(h=>h.key.toLowerCase()===String(k).toLowerCase()&&(v===undefined||h.value===v))).true;},body:value=>expect(data.response.body).equal(value),jsonBody:value=>value===undefined?JSON.parse(data.response.body):expect(JSON.parse(data.response.body)).eql(value)},be:{get json(){JSON.parse(data.response.body);expect(responseHeaders.some(h=>h.key.toLowerCase()==='content-type'&&/[/+]json(?:;|$)/i.test(h.value))).true;return true;},get ok(){expect(data.response.code).within(200,299);return true;}}}}):undefined;
const location=[...data.context.location];Object.defineProperty(location,'current',{value:location.at(-1)});
let nextRequestSet=false,nextRequest=null,skipped=false;const skipSignal=Object.freeze({});
const pm=Object.freeze({environment,collectionVariables,globals,variables,iterationData:iteration,request,response,expect,info:Object.freeze({requestId:data.request?.id,requestName:data.request?.name,eventName:data.response?'test':'prerequest',iteration:data.context.iteration,iterationCount:data.context.iteration_count}),execution:Object.freeze({location,skipRequest:()=>{if(data.response)throw Error('skipRequest is pre-request only');skipped=true;tests.length=0;throw skipSignal;},setNextRequest:value=>{if(value!==null&&typeof value!=='string')throw Error('Next request must be name, ID or null');nextRequestSet=true;nextRequest=value;}}),test:(name,fn)=>{if(tests.length>=256)throw Error('Test limit');try{const returned=fn();if(returned&&typeof returned.then==='function')throw Error('Async tests unsupported');tests.push({name:String(name).slice(0,256),passed:true,error:null});}catch{tests.push({name:String(name).slice(0,256),passed:false,error:'Assertion failed'});}}});
const log=(...values)=>{if(logs.length<128)logs.push(values.map(v=>typeof v==='object'?JSON.stringify(v):String(v)).join(' ').slice(0,2048));};
const console=Object.freeze({log,warn:log,error:log,info:log,debug:log});
