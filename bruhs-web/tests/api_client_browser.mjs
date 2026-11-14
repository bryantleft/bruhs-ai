// Production WASM with isolated storage and mocked API-client endpoints. Rust tests
// separately cover real SQLite/Postgres, native HTTP calls, and agent grants.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const dist=path.resolve(process.env.BRUHS_API_UI_DIST??path.join(root,'target/api-client-ui-dist'));
const output=path.join(root,'target/api-client-ui-qa');
await mkdir(output,{recursive:true});
assert((await stat(path.join(dist,'index.html'))).mtimeMs>=Math.max(...await Promise.all(['bruhs-web/src/components/api_client_panel.rs','bruhs-web/src/components/files_panel.rs','bruhs-models/src/api_client.rs'].map(async file=>(await stat(path.join(root,file))).mtimeMs))),'Refuse stale UI assets');
let playwright;
try {playwright=await import('playwright');}catch {
  for(const entry of await readdir(path.join(homedir(),'.npm/_npx'))) {try {playwright=await import(pathToFileURL(path.join(homedir(),'.npm/_npx',entry,'node_modules/playwright/index.mjs')).href);break;}catch {}}
}
assert(playwright,'An existing Playwright installation is required');
const server=createServer(async(req,res)=>{
  const url=new URL(req.url,'http://127.0.0.1');
  if(url.pathname==='/__fixture-stream') {res.writeHead(200,{'Content-Type':'text/event-stream','Access-Control-Allow-Origin':'*'});res.write(': fixture\n\n');const timer=setInterval(()=>res.write(': connected\n\n'),1000);req.on('close',()=>clearInterval(timer));return;}
  const file=path.resolve(dist,`.${url.pathname==='/'?'/index.html':decodeURIComponent(url.pathname)}`);
  if(!file.startsWith(dist+path.sep)){res.writeHead(403);res.end();return;}
  try {const bytes=await readFile(file);res.writeHead(200,{'Content-Type':{'.html':'text/html','.js':'text/javascript','.wasm':'application/wasm','.css':'text/css','.svg':'image/svg+xml','.woff2':'font/woff2'}[path.extname(file)]??'application/octet-stream'});res.end(bytes);}catch {res.writeHead(404);res.end();}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin=`http://127.0.0.1:${server.address().port}`;
const browser=await playwright.chromium.launch({headless:true,executablePath:process.env.BRUHS_CHROME??(process.platform==='darwin'?'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome':undefined)});
const checks=[];
const check=(ok,label)=>{assert(ok,label);checks.push(label);};
const empty=()=>({agents:[],sources:[],subscriptions:[],events:[],runs:[],proposals:[],source_statuses:[]});
async function dismissSidebar(page,width) {
  const scrim=page.locator('div[aria-hidden="true"].fixed.inset-0');
  if(await scrim.isVisible()) {await scrim.click({position:{x:width-12,y:60}});await page.getByRole('complementary',{name:'Sidebar navigation',exact:true}).waitFor({state:'hidden'});}
}


try {
for(const cell of [{name:'compact',width:640,height:480,hasTouch:false},{name:'touch',width:390,height:844,hasTouch:true}]) {
  const context=await browser.newContext({viewport:{width:cell.width,height:cell.height},hasTouch:cell.hasTouch,serviceWorkers:'block'});
  const state={snapshot:{collection:{version:1,requests:[],environment:{values:{},secrets:{}}},revision:null},proposals:[],calls:[],unexpected:[],errors:[],cancelled:new Set(),conflict:false};
  const response=id=>({request_id:id,status:201,elapsed_ms:17,size_bytes:128,headers:[{name:'content-type',value:'application/json'},{name:'set-cookie',value:'[redacted]'}],body:JSON.stringify({data:{viewer:{name:'Ada',id:'9223372036854775807'}},html:'<img src=x onerror="window.__apiXss=true">',echo:'[redacted]'}),redacted:true});
  await context.route('**/*',async route=>{
    const request=route.request(),url=new URL(request.url());
    if(!url.pathname.startsWith('/api/')) {if(url.origin===origin)return route.continue();state.unexpected.push(request.url());return route.abort();}
    const json=(value,status=200)=>route.fulfill({status,contentType:'application/json',headers:{'Access-Control-Allow-Origin':origin,'Access-Control-Allow-Headers':'*','Access-Control-Allow-Methods':'GET, POST, OPTIONS','Cache-Control':'no-store'},body:JSON.stringify(value)});
    if(request.method()==='OPTIONS')return json({});
    if(url.pathname==='/api/api-client/collection'&&request.method()==='GET')return json(url.searchParams.get('project_path')==='/fixture/other-project'?{collection:{version:1,requests:[],environment:{values:{},secrets:{}}},revision:null}:state.snapshot);
    if(url.pathname==='/api/api-client/collection'&&request.method()==='POST') {const input=request.postDataJSON();state.calls.push({type:'save',input});if(state.conflict)return json({error:'The collection changed on disk. Reload it, review your draft, and save again.'},409);assert.equal(input.expected_revision,state.snapshot.revision);state.snapshot={collection:input.collection,revision:randomUUID()};return json(state.snapshot);}
    if(url.pathname==='/api/api-client/credential'){state.calls.push({type:'credential',input:request.postDataJSON()});return json({saved:true});}
    if(url.pathname==='/api/api-client/send') {const input=request.postDataJSON();state.calls.push({type:'send',input});if(input.request.url.includes('/slow')){while(!state.cancelled.has(input.request_id))await new Promise(resolve=>setTimeout(resolve,20));return json({error:'Request cancelled'},400);}await new Promise(resolve=>setTimeout(resolve,cell.hasTouch?300:30));return json(response(input.request_id));}
    if(url.pathname.startsWith('/api/api-client/cancel/')){const id=url.pathname.split('/').at(-1);state.cancelled.add(id);state.calls.push({type:'cancel',id});return route.fulfill({status:204,headers:{'Access-Control-Allow-Origin':origin},body:''});}
    if(url.pathname==='/api/api-client/proposals')return json(state.proposals.filter(proposal=>proposal.project_path===url.searchParams.get('project_path')));
    if(url.pathname==='/api/api-client/decision'){const input=request.postDataJSON();const proposal=state.proposals.find(proposal=>proposal.id===input.id);assert.equal(proposal.state.state,'pending');state.calls.push({type:'decision',input});proposal.state=input.approve?{state:'finished',response:response(input.id)}:{state:'declined'};return json(proposal);}
    if(url.pathname==='/api/agent/files/children')return json([]);
    if(url.pathname==='/api/agent/git-branches')return json({branches:[],current:null});
    if(url.pathname==='/api/agent/git-status')return json({branch:null,ahead:0,behind:0,dirty:[]});
    if(request.method()!=='GET'){state.unexpected.push(`${request.method()} ${url.pathname}`);return json({error:'Unexpected live action'},500);}
    if(url.pathname.endsWith('/stream'))return route.continue({url:`${origin}/__fixture-stream`});
    if(url.pathname==='/api/reactive')return json(empty());
    if(url.pathname==='/api/weekly-reports')return json({presets:[],reports:[],next_before:null});
    if(/\/(meetings|people|credentials|tasks|threads|skills|models|children)$/.test(url.pathname))return json([]);
    return json({});
  });
  const thread=randomUUID(),otherThread=randomUUID();
  await context.addInitScript(({thread,otherThread,origin})=>{if(location.origin!==origin)return;const request=indexedDB.open('bruhs',1);request.onupgradeneeded=()=>request.result.createObjectStore('threads',{keyPath:'id'});request.onsuccess=()=>{const db=request.result;const tx=db.transaction('threads','readwrite');tx.objectStore('threads').put({id:thread,title:'API fixture',project_name:'Fixture',project_path:'/fixture/project',updated_at:Date.now(),pinned:false,settled:false});tx.objectStore('threads').put({id:otherThread,title:'Other API project',project_name:'Other fixture',project_path:'/fixture/other-project',updated_at:Date.now()-86400000,pinned:false,settled:false});tx.oncomplete=()=>db.close();tx.onabort=()=>db.close();};request.onerror=()=>{if(request.error?.name!=='VersionError')console.error('Synthetic thread seed failed',request.error?.name);};},{thread,otherThread,origin});
  const page=await context.newPage();page.setDefaultTimeout(15000);page.on('pageerror',error=>{state.errors.push(error.message);console.error(error.message);});
  await page.goto(`${origin}/#thread/${thread}`);await page.locator('h1[title="Double click to rename"]').first().waitFor({state:'attached'});await page.evaluate(id=>{location.hash=`thread/${id}`;dispatchEvent(new HashChangeEvent('hashchange'));},thread);await page.getByText('API fixture',{exact:true}).first().waitFor({state:'attached'});await dismissSidebar(page,cell.width);await page.keyboard.press('Meta+Alt+b');
  if(!await page.getByRole('tab',{name:'Files',exact:true}).first().isVisible())await page.keyboard.press('Meta+d');
  await page.getByRole('tab',{name:'Files',exact:true}).first().click();await page.getByRole('tab',{name:'API',exact:true}).click();
  const panel=page.locator('[data-api-client-panel]');await panel.getByRole('status').filter({hasText:'Collection loaded'}).waitFor();
  await panel.getByText('Environment and keychain',{exact:true}).click();await panel.getByLabel('Variables (name=value)',{exact:true}).fill('base_url=http://fixture.invalid\nwho=Ada');
  await panel.getByLabel('Keychain references (variable=credential name)',{exact:true}).fill('key=Fixture API');
  await panel.getByLabel('New credential name',{exact:true}).fill('Fixture API');await panel.getByLabel('New credential value',{exact:true}).fill('synthetic-browser-only-canary');await panel.getByRole('button',{name:'Save credential',exact:true}).click();
  await panel.getByRole('status').filter({hasText:'Credential saved'}).waitFor();check(await panel.getByLabel('New credential value',{exact:true}).inputValue()==='',`${cell.name}: saving a credential clears its private input`);
  await panel.getByLabel('Request name',{exact:true}).fill('GraphQL fixture');await panel.getByLabel('Method',{exact:true}).selectOption('POST');await panel.getByLabel('Request URL',{exact:true}).fill('{{base_url}}/graphql');await panel.getByLabel('Headers (Name: value)',{exact:true}).fill('Content-Type: application/json\nX-Api-Key: {{key}}');await panel.getByLabel('Body (raw or GraphQL JSON)',{exact:true}).fill('{"query":"query($name: String!) { viewer(name: $name) { name } }","variables":{"name":"{{who}}"}}');
  await panel.getByRole('button',{name:'Send request',exact:true}).click();await panel.getByRole('region',{name:'API response',exact:true}).waitFor();
  const send=state.calls.find(call=>call.type==='send').input;check(send.request.method==='POST'&&send.environment.values.who==='Ada'&&send.environment.secrets.key==='Fixture API'&&!JSON.stringify(send).includes('synthetic-browser-only-canary'),`${cell.name}: request uses a keychain name and ordinary environment variables`);
  const result=panel.getByRole('region',{name:'API response',exact:true});check((await result.innerText()).includes('HTTP 201')&&(await result.innerText()).includes('17 ms'),`${cell.name}: response shows status, timing and size`);
  await result.getByRole('button',{name:'▸ data: {1 keys}',exact:true}).click();await result.getByRole('button',{name:'▸ viewer: {2 keys}',exact:true}).click();check((await result.getByLabel('JSON tree').innerText()).includes('Ada'),`${cell.name}: JSON tree expands nested data`);
  await result.getByRole('tab',{name:'Body',exact:true}).click();check((await result.innerText()).includes('<img')&&await result.locator('img').count()===0&&!await page.evaluate(()=>Boolean(window.__apiXss)),`${cell.name}: HTML response stays inert text`);
  await result.getByRole('tab',{name:'Headers',exact:true}).click();check((await result.innerText()).includes('set-cookie: [redacted]'),`${cell.name}: redacted response headers are inspectable`);
  const beforeAttach=await page.locator('#chat-input').inputValue();check(!beforeAttach.includes('Explicitly attached API response'),`${cell.name}: a response does not enter the draft automatically`);await result.getByRole('button',{name:'Add response to draft',exact:true}).click();check((await page.locator('#chat-input').inputValue()).includes('Explicitly attached API response')&&!JSON.stringify(state.calls.filter(call=>call.type!=='credential')).includes('synthetic-browser-only-canary'),`${cell.name}: explicit attachment prepares an unsent redacted draft`);
  await panel.getByLabel('New credential value',{exact:true}).fill('never-saved-private-entry');await panel.getByLabel('Request name',{exact:true}).fill('Unsent tab edit');await page.getByRole('tab',{name:'Files',exact:true}).last().click();await page.getByRole('tab',{name:'API',exact:true}).click();await panel.getByRole('region',{name:'API response',exact:true}).waitFor();check(await panel.getByLabel('Request name',{exact:true}).inputValue()==='Unsent tab edit'&&(await panel.getByLabel('Variables (name=value)',{exact:true}).inputValue()).includes('who=Ada'),`${cell.name}: visiting Files preserves the unsaved draft, environment and response`);check(await panel.getByLabel('New credential value',{exact:true}).inputValue()==='',`${cell.name}: hidden credential input is excluded from the preserved draft`);await panel.getByLabel('Request name',{exact:true}).fill('GraphQL fixture');
  await panel.getByRole('button',{name:'Save request',exact:true}).click();await panel.getByRole('status').filter({hasText:'Saved .bruhs/api/collection.json'}).waitFor();check(state.snapshot.collection.requests[0].name==='GraphQL fixture'&&state.snapshot.collection.environment.secrets.key==='Fixture API',`${cell.name}: collection saves request and environment references`);
  state.conflict=true;await panel.getByLabel('Request name',{exact:true}).fill('Unsent edit');await panel.getByRole('button',{name:'Save request',exact:true}).click();await panel.getByRole('alert').filter({hasText:'changed on disk'}).waitFor();check(await panel.getByLabel('Request name',{exact:true}).inputValue()==='Unsent edit'&&state.snapshot.collection.requests[0].name==='GraphQL fixture',`${cell.name}: a conflict retains the draft and saved collection`);state.conflict=false;
  await panel.getByLabel('Request URL',{exact:true}).fill('http://fixture.invalid/slow');await panel.getByRole('button',{name:'Send request',exact:true}).click();await panel.getByRole('button',{name:'Cancel request',exact:true}).click();await panel.getByRole('status').filter({hasText:'Request cancelled locally'}).waitFor();await page.waitForTimeout(80);check(state.calls.some(call=>call.type==='cancel')&&await panel.getByRole('region',{name:'API response',exact:true}).count()===0,`${cell.name}: cancel reaches the host and late results stay discarded`);
  await panel.getByRole('button',{name:'Send request',exact:true}).click();await panel.getByRole('button',{name:'Cancel request',exact:true}).waitFor();await page.getByRole('tab',{name:'Files',exact:true}).last().click();await page.getByRole('tab',{name:'API',exact:true}).click();await panel.getByRole('status').filter({hasText:'panel closed'}).waitFor();check(state.calls.filter(call=>call.type==='cancel').length===2&&await panel.getByLabel('Request URL',{exact:true}).inputValue()==='http://fixture.invalid/slow',`${cell.name}: leaving API cancels its active send and retains the draft`);
  await panel.getByText('Import curl',{exact:true}).click();const callsBeforeImport=state.calls.length;await panel.getByLabel('Curl command',{exact:true}).fill("curl 'https://fixture.invalid/graphql' --json '{\"query\":\"{ viewer { name } }\"}'");await panel.getByRole('button',{name:'Import into draft',exact:true}).click();check(await panel.getByLabel('Method',{exact:true}).inputValue()==='POST'&&await panel.getByLabel('Request URL',{exact:true}).inputValue()==='https://fixture.invalid/graphql'&&state.calls.length===callsBeforeImport,`${cell.name}: curl import creates a draft without executing a request`);
  await panel.getByLabel('Curl command',{exact:true}).fill('curl https://fixture.invalid/ --config /tmp/never-read');await panel.getByRole('button',{name:'Import into draft',exact:true}).click();await panel.getByRole('alert').filter({hasText:'Unsupported curl'}).waitFor();check((await panel.getByLabel('Curl command',{exact:true}).inputValue()).includes('--config'),`${cell.name}: unsupported file-reading import retains its input`);
  const proposal=name=>({id:randomUUID(),project_path:'/fixture/project',thread_id:thread,destination:'http://fixture.invalid/agent-route',request:{...state.snapshot.collection.requests[0],id:randomUUID(),name,url:'{{base_url}}/agent-route'},environment:state.snapshot.collection.environment,state:{state:'pending'}});state.proposals=[proposal('Agent fixture'),proposal('Decline fixture')];
  await panel.getByRole('button',{name:'Refresh approvals',exact:true}).click();const card=panel.locator('article').filter({hasText:'Agent fixture'});await card.waitFor();check((await card.innerText()).includes('http://fixture.invalid/agent-route')&&(await card.innerText()).includes('Fixture API')&&!state.calls.some(call=>call.type==='decision'),`${cell.name}: agent proposal shows destination and reference names before approval`);
  await card.getByRole('button',{name:'Send once',exact:true}).click();await card.getByText('Completed: HTTP 201',{exact:true}).waitFor();check(await card.getByRole('button',{name:'Send once',exact:true}).count()===0&&state.calls.filter(call=>call.type==='decision'&&call.input.approve).length===1,`${cell.name}: approval is one use and exposes the approved response`);
  const decline=panel.locator('article').filter({hasText:'Decline fixture'});await decline.getByRole('button',{name:'Decline',exact:true}).click();await decline.getByText('Declined',{exact:true}).waitFor();check(state.calls.filter(call=>call.type==='decision'&&call.input.approve).length===1,`${cell.name}: declining sends no request`);
  await panel.getByRole('region',{name:'API response',exact:true}).getByRole('tab',{name:'JSON tree',exact:true}).click();await panel.getByRole('region',{name:'API response',exact:true}).getByRole('button',{name:'▸ data: {1 keys}',exact:true}).click();await panel.getByRole('region',{name:'API response',exact:true}).getByRole('button',{name:'▸ viewer: {2 keys}',exact:true}).click();await panel.getByRole('region',{name:'API response',exact:true}).scrollIntoViewIfNeeded();await page.screenshot({path:path.join(output,`${cell.name}-response.png`)});
  check((await panel.evaluate(element=>element.scrollWidth<=element.clientWidth+1)),`${cell.name}: panel has no horizontal overflow`);
  if(cell.hasTouch)check(await panel.getByLabel('Request URL',{exact:true}).evaluate(element=>parseFloat(getComputedStyle(element).fontSize))>=16,`${cell.name}: touch fields avoid browser zoom`);
  await panel.getByLabel('Request URL',{exact:true}).fill('http://fixture.invalid/slow');await panel.getByRole('button',{name:'Send request',exact:true}).click();await panel.getByRole('button',{name:'Cancel request',exact:true}).waitFor();await page.evaluate(id=>{location.hash=`thread/${id}`;},otherThread);await page.getByText('Other API project',{exact:true}).first().waitFor({state:'attached'});await page.getByRole('tab',{name:'API',exact:true}).click();await panel.getByRole('status').filter({hasText:'Collection loaded'}).waitFor();await page.waitForTimeout(80);check(state.calls.filter(call=>call.type==='cancel').length===3,`${cell.name}: changing projects cancels the former project's active send`);check(await panel.getByLabel('Request name',{exact:true}).inputValue()==='New request'&&await panel.getByLabel('Variables (name=value)',{exact:true}).inputValue()===''&&await panel.getByRole('region',{name:'API response',exact:true}).count()===0,`${cell.name}: another project never restores the former project's draft or response`);
  check(state.errors.length===0,`${cell.name}: no browser runtime errors`);check(state.unexpected.length===0,`${cell.name}: only fixture APIs were used`);await context.close();
}
await import('node:fs/promises').then(fs=>fs.writeFile(path.join(output,'checks.json'),JSON.stringify({checks},null,2)));console.log(JSON.stringify({passed:checks.length,checks},null,2));
} finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
